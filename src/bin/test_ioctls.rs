use clap::{value_t, App, Arg, SubCommand};
use kvm_bindings as kvmb;
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::os::unix::io::AsRawFd;
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};
use vmsh::kvm::ioctls;
use vmsh::result::Result;

use vmsh::kvm;

fn inject(pid: Pid) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );

    print!("check_extensions");
    for _ in 1..100 {
        let tracee = vm.attach()?;
        try_with!(tracee.check_extension(0), "cannot query kvm extensions");
        print!(".")
    }
    println!(" ok");

    let tracee = vm.attach()?;

    print!("get_regs");
    for cpu in vm.vcpus.iter() {
        let regs = tracee.get_regs(cpu)?;
        assert_ne!(regs.ip(), 0);
        print!(".")
    }
    println!(" ok");

    print!("get_fpu_regs");
    for cpu in vm.vcpus.iter() {
        tracee.get_fpu_regs(cpu)?;
        print!(".")
    }
    println!(" ok");
    Ok(())
}

fn alloc_mem(pid: Pid) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );

    let tracee = vm.attach()?;
    let mem = try_with!(tracee.alloc_mem::<u32>(), "mmap failed");
    assert_eq!(mem.read()?, 0);
    let val: u32 = 0xdeadbeef;
    mem.write(&val)?;
    assert_eq!(mem.read()?, val);

    Ok(())
}

fn guest_add_mem(pid: Pid, re_get_slots: bool) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );
    let tracee = vm.attach()?;

    // count memslots
    let memslots_a = tracee.get_maps()?;
    memslots_a.iter().for_each(|map| {
        println!(
            "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
        )
    });
    println!("--");

    // add memslot
    let slot_len = 4096; // must be page aligned
    let hv_memslot = tracee.alloc_mem_padded::<u64>(slot_len)?;
    let arg = kvmb::kvm_userspace_memory_region {
        slot: memslots_a.len() as u32,
        flags: 0x00,                 // maybe KVM_MEM_READONLY
        guest_phys_addr: 0xd0000000, // must be page aligned
        memory_size: slot_len as u64,
        userspace_addr: hv_memslot.ptr as u64,
    };
    let ret = tracee.vm_ioctl_with_ref(ioctls::KVM_SET_USER_MEMORY_REGION(), &arg)?;
    if ret != 0 {
        bail!("ioctl_with_ref failed: {}", ret)
    }

    if re_get_slots {
        // count memslots again
        let memslots_b = tracee.get_maps()?;
        memslots_b.iter().for_each(|map| {
            println!(
                "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
                map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
            )
        });
        assert_eq!(memslots_a.len(), memslots_b.len());
    }

    Ok(())
}

fn guest_ioeventfd(pid: Pid) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );
    let tracee = vm.attach()?;

    let has_cap = try_with!(
        tracee.check_extension(kvmb::KVM_CAP_IOEVENTFD as i32),
        "cannot check kvm extension capabilities"
    );
    if has_cap == 0 {
        bail!(
            "This operation requires KVM_CAP_IOEVENTFD which your KVM does not have: {}",
            has_cap
        );
    }
    println!("caps good");

    //let ioeventfd_fd = tracee.open(); TODO
    let ioeventfd_fd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd");
    println!("{:?}", ioeventfd_fd.as_raw_fd());
    //let ioeventfd_mmap_fd = tracee.open(); TODO
    //let ioeventfd = tracee.malloc(ioeventfd_mmap_fd); TODO
    //let mmio_mem = try_with!(tracee.mmap(16), "cannot allocate mmio region");

    let ioeventfd = kvmb::kvm_ioeventfd {
        datamatch: 0,
        len: 8,
        addr: 0xfffffff0,
        fd: ioeventfd_fd.as_raw_fd(), // thats why we get -22 EINVAL
        flags: 0,
        ..Default::default()
    };
    let ret = try_with!(
        tracee.vm_ioctl_with_ref(ioctls::KVM_IOEVENTFD(), &ioeventfd),
        "kvm ioeventfd ioctl injection failed"
    );
    if ret != 0 {
        bail!("cannot register KVM_IOEVENTFD via ioctl: {:?}", ret);
    }

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
