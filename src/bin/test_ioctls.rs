use clap::{value_t, App, Arg, SubCommand};
use kvm_bindings as kvmb;
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};
use vmsh::kvm::hypervisor::get_hypervisor;
use vmsh::result::Result;

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
                "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
                map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
            )
        });

        // add memslot
        let vm_mem = vm.vm_add_mem::<u64>(0xd0000000, false)?;
        println!("--");

        if re_get_slots {
            // count memslots again
            let memslots_b = vm.get_maps()?;
            memslots_b.iter().for_each(|map| {
                println!(
                    "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
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
                "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
                map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
            )
        });
        assert_eq!(memslots_a_len, memslots_c.len());
    }
    Ok(())
}

fn fd_transfer(pid: Pid, nr_fds: u32) -> Result<()> {
    use std::path::Path;

    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let mut fds = vec![];
    for _ in 0..nr_fds {
        let fd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd").as_raw_fd();
        fds.push(fd);
    }

    let remote_fds = vm.transfer(fds.as_slice())?;
    assert_eq!(remote_fds.len(), fds.len());

    for fd in remote_fds {
        let pathname = format!("/proc/{}/fd/{}", pid, fd);
        let path = Path::new(&pathname);
        assert_eq!(path.exists(), true);
    }

    Ok(())
}

fn guest_userfaultfd(pid: Pid) -> Result<()> {
    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);
    vm.stop()?;

    let vm_mem = vm.vm_add_mem::<u64>(0xd0000000, true)?;
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

    let vm_mem = vm.vm_add_mem::<u32>(0xd0000000, true)?;
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

fn guest_kvm_exits(pid: Pid) -> Result<()> {
    //let tracee = vmsh::inject_syscall::attach(pid)?;
    //for i in 0..10 {
        //println!("{}", i);
        //tracee.await_syscall()?;
    //}
    //return Ok(());

    let vm = try_with!(get_hypervisor(pid), "cannot get vms for process {}", pid);

    //vm.stop()?;
    for i in 0..10 {
        println!("{}", i);
        vm.log_kvm_exits()?;
    }
    //vm.resume()?;

    Ok(())
}

fn subtest(name: &str) -> App {
    SubCommand::with_name(name).arg(Arg::with_name("pid").required(true).index(1))
}

fn main() {
    let app = App::new("test_ioctls")
        .about("Something between integration and unit test to be used by pytest.")
        .subcommand(subtest("alloc_mem"))
        .subcommand(subtest("inject"))
        .subcommand(subtest("guest_add_mem"))
        .subcommand(subtest("guest_add_mem_get_maps"))
        .subcommand(subtest("fd_transfer1"))
        .subcommand(subtest("fd_transfer2"))
        .subcommand(subtest("guest_userfaultfd"))
        .subcommand(subtest("guest_kvm_exits"))
        .subcommand(subtest("guest_ioeventfd"));

    let matches = app.get_matches();
    let subcommand_name = matches.subcommand_name().expect("subcommad required");
    let subcommand_matches = matches.subcommand_matches(subcommand_name).expect("foo");
    let pid = value_t!(subcommand_matches, "pid", i32).unwrap_or_else(|e| e.exit());
    let pid = Pid::from_raw(pid);

    let result = match subcommand_name {
        "alloc_mem" => alloc_mem(pid),
        "inject" => inject(pid),
        "guest_add_mem" => guest_add_mem(pid, false),
        "guest_add_mem_get_maps" => guest_add_mem(pid, true),
        "fd_transfer1" => fd_transfer(pid, 1),
        "fd_transfer2" => fd_transfer(pid, 2),
        "guest_userfaultfd" => guest_userfaultfd(pid),
        "guest_kvm_exits" => guest_kvm_exits(pid),
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
