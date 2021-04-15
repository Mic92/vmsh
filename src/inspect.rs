//mod device;

use crate::result::Result;
use nix::unistd::Pid;
use simple_error::try_with;

use crate::kvm;

pub struct InspectOptions {
    pub pid: Pid,
}

pub fn inspect(opts: &InspectOptions) -> Result<()> {
    let vm = try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    vm.stop()?;

    for map in vm.get_maps()? {
        println!(
            "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        )
    }

    println!("vcpu maps");
    for map in vm.get_vcpu_maps()? {
        println!(
            "vm cpu mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?}) @@ {}",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags, map.pathname
        );

        let map_ptr = map.start as *const kvm_bindings::kvm_run;
        let kvm_run: kvm_bindings::kvm_run =
            kvm::hypervisor::process_read(opts.pid, map_ptr as *const libc::c_void)?;

        println!("kvm_run: exit_reason {}", kvm_run.exit_reason);
    }

    Ok(())
}
