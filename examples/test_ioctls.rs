use clap::{value_t, App, Arg, SubCommand};
use kvm_bindings as kvmb;
use nix::unistd::Pid;
use simple_error::{bail, require_with, try_with};
use std::mem::size_of;
use std::os::unix::io::AsRawFd;
use std::sync::Mutex;
use std::time::Duration;
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};
use vmsh::kvm::hypervisor::{get_hypervisor, memory::PhysMem};
use vmsh::kvm::kvm_ioregionfd::{self, Cmd};
use vmsh::result::Result;
use vmsh::tracer::wrap_syscall::KvmRunWrapper;

fn inject(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);

    print!("check_extensions");
    for _ in 1..100 {
        vm.stop()?;
        try_with!(vm.check_extension(0), "cannot query kvm extensions");
        print!(".");
        vm.resume()?;
    }
    println!(" ok");

    print!("get_regs");
    vm.stop()?;
    for cpu in vm.vcpus.iter() {
        let regs = vm.get_regs(cpu)?;
        assert_ne!(regs.ip(), 0);
        print!(".")
    }
    println!(" ok");

    print!("get_fpu_regs");
    for cpu in vm.vcpus.iter() {
        vm.get_fpu_regs(cpu)?;
        print!(".")
    }
    println!(" ok");
    Ok(())
}

fn alloc_mem(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);

    vm.stop()?;
    let mem = try_with!(vm.alloc_mem::<u32>(), "mmap failed");
    assert_eq!(mem.read()?, 0);
    let val: u32 = 0xdeadbeef;
    mem.write(&val)?;
    assert_eq!(mem.read()?, val);

    Ok(())
}

fn guest_add_mem(pid: Pid, re_get_slots: bool) -> Result<()> {
    let memslots_a_len;

    {
        let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
        vm.stop()?;

        // count memslots
        let memslots_a = vm.get_maps()?;
        memslots_a_len = memslots_a.len();
        memslots_a.iter().for_each(|map| {
            println!(
                "vm mem: {:#x} -> {:#x} (physical: {:#x}, flags: {:?} | {:?})",
                map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
            )
        });

        // add memslot
        let vm_mem: PhysMem<u64> = vm.vm_add_mem::<u64>(0xd0000000, size_of::<u64>(), false)?;
        println!("--");

        if re_get_slots {
            // count memslots again
            let memslots_b = vm.get_maps()?;
            memslots_b.iter().for_each(|map| {
                println!(
                    "vm mem: {:#x} -> {:#x} (physical: {:#x}, flags: {:?} | {:?})",
                    map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
                )
            });
            assert_eq!(memslots_a.len() + 1, memslots_b.len());
        }
        println!("write 0xdeadbeef to 0xd0000000");
        vm_mem.mem.write(&0xDEADBEEF)?;
    }

    // VmMem is out of scope and should thus have removed the memory again.
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    if re_get_slots {
        // count memslots again
        let memslots_c = vm.get_maps()?;
        memslots_c.iter().for_each(|map| {
            println!(
                "vm mem: {:#x} -> {:#x} (physical: {:#x}, flags: {:?} | {:?})",
                map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
            )
        });
        assert_eq!(memslots_a_len, memslots_c.len());
    }
    Ok(())
}

fn fd_transfer(pid: Pid, nr_fds: u32) -> Result<()> {
    use std::path::Path;

    let mut vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;
    try_with!(
        vm.setup_transfer_sockets(),
        "failed to set up transfer sockets"
    );

    let event_files = [
        try_with!(EventFd::new(EFD_NONBLOCK), "failed to create eventfd"),
        try_with!(EventFd::new(EFD_NONBLOCK), "failed to create eventfd"),
    ];
    let fds = event_files
        .iter()
        .map(|ev| ev.as_raw_fd())
        .collect::<Vec<_>>();

    let remote_fds = try_with!(vm.transfer(fds.as_slice()), "failed to transfer sockets");
    assert_eq!(remote_fds.len(), fds.len());

    for fd in remote_fds {
        let pathname = format!("/proc/{}/fd/{}", pid, fd);
        let path = Path::new(&pathname);
        assert_eq!(path.exists(), true);
    }
    dbg!(fds);
    vm.close_transfer_sockets()?;

    Ok(())
}

fn cpuid2(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let cpuid2 = try_with!(vm.get_cpuid2(&vm.vcpus[0]), "cannot get cpuid2");
    // Get Virtual and Physical address Sizes
    require_with!(
        cpuid2.entries.iter().find(|c| c.function == 0x80000008),
        "could not find cpuid function 0x80000008"
    );

    Ok(())
}

/// Some parts of this implementation are still missing.
fn guest_userfaultfd(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let vm_mem = vm.vm_add_mem::<u64>(0xd0000000, size_of::<u64>(), true)?;
    vm_mem.mem.write(&0xdeadbeef)?;
    assert_eq!(vm_mem.mem.read()?, 0xdeadbeef);

    // register userfaultfd which always returns something else
    vm.userfaultfd()?;

    vm.resume()?;

    println!("pause");
    nix::unistd::pause();
    // pytest shall now check that the memory does not contain 0xdeadbeef on read

    Ok(())
}

fn guest_ioeventfd(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let has_cap = try_with!(
        vm.check_extension(kvmb::KVM_CAP_IOEVENTFD as i32),
        "cannot check kvm extension capabilities"
    );
    if has_cap == 0 {
        bail!(
            "This operation requires KVM_CAP_IOEVENTFD which your KVM does not have: {}",
            has_cap
        );
    }
    println!("caps good");

    let vm_mem = vm.vm_add_mem::<u32>(0xd0000000, size_of::<u32>(), true)?;
    vm_mem.mem.write(&0xbeef)?;
    let ioeventfd = vm.ioeventfd(0xd0000000)?;
    vm.resume()?;

    use std::io::prelude::*;
    loop {
        match ioeventfd.read() {
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    print!(".");
                    std::io::stdout()
                        .lock()
                        .flush()
                        .expect("cannot flush stdout");
                    std::thread::sleep(Duration::from_millis(100));
                } else {
                    bail!("read error {}", e);
                }
            }
            Ok(v) => {
                println!("event: {}", v);
                break;
            }
        }
    }

    Ok(())
}

fn ioregionfd(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let has_cap = try_with!(
        vm.check_extension(kvm_ioregionfd::KVM_CAP_IOREGIONFD as i32),
        "cannot check kvm extension capabilities"
    );
    if has_cap == 0 {
        bail!(
            "This operation requires KVM_CAP_IOREGIONFD which your KVM does not have: {}",
            has_cap
        );
    }
    println!("caps good");

    let mut ioregionfd = vm.ioregionfd(0xd0000000, 32)?;
    let mut rawiorefd = ioregionfd.fdclone();
    vm.resume()?;

    for _ in 0..10000 {
        let cmd = match try_with!(rawiorefd.read(), "foo") {
            Some(cmd) => cmd,
            None => continue,
        };

        println!(
            "{:?}, {:?}, response={}: {:?}",
            cmd.info.cmd(),
            cmd.info.size(),
            cmd.info.is_response(),
            cmd
        );
        match cmd.info.cmd() {
            Cmd::Read => rawiorefd.write(0xFF)?,
            Cmd::Write => rawiorefd.write(0)?,
        };
    }

    vm.stop()?;
    Ok(())
}

fn guest_kvm_exits(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.kvmrun_wrapped(|wrapper_r: &Mutex<Option<KvmRunWrapper>>| {
        let mut wrapper_go = wrapper_r.lock().unwrap();
        let wrapper = wrapper_go.as_mut().unwrap();
        let value: [u8; 2] = 0xDEADu16.to_ne_bytes();
        println!("attached");

        for _i in 0..100000 {
            let mut mmio = wrapper.wait_for_ioctl()?;
            if let Some(mmio) = &mut mmio {
                println!("kvm exit: {}", mmio);
                if !mmio.is_write {
                    mmio.answer_read(&value)?;
                }
            }
        }

        Ok(())
    })?;
    Ok(())
}

fn vcpu_maps(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let kvm_run_len = size_of::<kvm_bindings::kvm_run>();
    println!("kvm_run len {}", kvm_run_len);

    let maps = vm.get_maps()?;
    assert!(!maps.is_empty());

    println!("vcpu maps");
    let vcpus = vm.get_vcpu_maps()?;
    assert!(!vcpus.is_empty());
    for map in vcpus {
        println!(
            "vm cpu mem: {:#x} -> {:#x} (physical: {:#x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        );
        assert!(map.end - map.start >= kvm_run_len);
    }

    Ok(())
}

fn subtest(name: &str) -> App {
    SubCommand::with_name(name).arg(Arg::with_name("pid").required(true).index(1))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let app = App::new("test_ioctls")
        .about("Something between integration and unit test to be used by pytest.")
        .subcommand(subtest("alloc_mem"))
        .subcommand(subtest("inject"))
        .subcommand(subtest("guest_add_mem"))
        .subcommand(subtest("guest_add_mem_get_maps"))
        .subcommand(subtest("fd_transfer1"))
        .subcommand(subtest("fd_transfer2"))
        .subcommand(subtest("cpuid2"))
        .subcommand(subtest("guest_userfaultfd"))
        .subcommand(subtest("guest_kvm_exits"))
        .subcommand(subtest("vcpu_maps"))
        .subcommand(subtest("ioregionfd"))
        .subcommand(subtest("guest_ioeventfd"));

    let matches = app.get_matches();
    let subcommand_name = matches.subcommand_name().expect("subcommad required");
    let subcommand_matches = matches.subcommand_matches(subcommand_name).expect("foo");
    let pid = value_t!(subcommand_matches, "pid", i32).unwrap_or_else(|e| e.exit());
    let pid = Pid::from_raw(pid);

    let result = match subcommand_name {
        "alloc_mem" => alloc_mem(pid),
        "inject" => inject(pid),
        "cpuid2" => cpuid2(pid),
        "guest_add_mem" => guest_add_mem(pid, false),
        "guest_add_mem_get_maps" => guest_add_mem(pid, true),
        "fd_transfer1" => fd_transfer(pid, 1),
        "fd_transfer2" => fd_transfer(pid, 2),
        "guest_userfaultfd" => guest_userfaultfd(pid),
        "guest_kvm_exits" => guest_kvm_exits(pid),
        "vcpu_maps" => vcpu_maps(pid),
        "ioregionfd" => ioregionfd(pid),
        "guest_ioeventfd" => guest_ioeventfd(pid),
        _ => std::process::exit(2),
    };

    if let Err(err) = result {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("ok");
    }
}
