use clap::{value_t, App, Arg, SubCommand};
use kvm_bindings as kvmb;
use nix::unistd::Pid;
use simple_error::{bail, try_with};
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

    Ok(())
}

fn mmap(pid: Pid) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(pid),
        "cannot get vms for process {}",
        pid
    );

    let tracee = vm.attach()?;
    let addr = try_with!(tracee.mmap(4), "mmap failed");
    assert_eq!(vm.read::<u32>(addr)?, 0);
    vm.write::<u32>(addr, &0xdeadbeef)?;
    assert_eq!(vm.read::<u32>(addr)?, 0xdeadbeef);

    Ok(())
}

fn guest_add_mem(pid: Pid) -> Result<()> {
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
    let slot_len = 0x10;
    let hv_memslot_addr = tracee.mmap(slot_len)?;
    let arg = kvmb::kvm_userspace_memory_region {
        slot: memslots_a.len() as u32,
        flags: 0, // maybe KVM_MEM_READONLY
        guest_phys_addr: 0xd0000000,
        memory_size: slot_len as u64,
        userspace_addr: hv_memslot_addr as u64,
    };
    let ret = tracee.vm_ioctl_with_ref(ioctls::KVM_SET_USER_MEMORY_REGION(), &arg)?;
    if ret != 0 {
        bail!("ioctl_with_ref failed: {}", ret)
    }

    // count memslots again
    let memslots_b = tracee.get_maps()?;
    memslots_b.iter().for_each(|map| {
        println!(
            "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
        )
    });
    assert_eq!(memslots_a.len(), memslots_b.len());

    Ok(())
}

fn subtest(name: &str) -> App {
    SubCommand::with_name(name).arg(Arg::with_name("pid").required(true).index(1))
}

fn main() {
    let app = App::new("test_ioctls")
        .about("Something between integration and unit test to be used by pytest.")
        .subcommand(subtest("mmap"))
        .subcommand(subtest("inject"))
        .subcommand(subtest("guest_add_mem"));

    let matches = app.get_matches();
    let subcommand_name = matches.subcommand_name().expect("subcommad required");
    let subcommand_matches = matches.subcommand_matches(subcommand_name).expect("foo");
    let pid = value_t!(subcommand_matches, "pid", i32).unwrap_or_else(|e| e.exit());
    let pid = Pid::from_raw(pid);

    let result = match subcommand_name {
        "mmap" => mmap(pid),
        "inject" => inject(pid),
        "guest_add_mem" => guest_add_mem(pid),
        _ => std::process::exit(2),
    };

    if let Err(err) = result {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("ok");
    }
}
