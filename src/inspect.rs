//mod device;

use crate::result::Result;
use nix::unistd::Pid;
use simple_error::try_with;
use std::sync::{Arc, Mutex};

use crate::kvm;
use crate::kvm::Hypervisor;
use virtio::attach_blk_dev;
use crate::device::Device;
use crate::kvm::get_hypervisor;

pub struct InspectOptions {
    pub pid: Pid,
}

pub fn inspect(opts: &InspectOptions) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    for map in vm.get_maps()? {
        println!(
            "vm mem: 0x{:x} -> 0x{:x} (physical: 0x{:x}, flags: {:?} | {:?})",
            map.start, map.end, map.phys_addr, map.prot_flags, map.map_flags,
        )
    }

    let device = try_with!(Device::new(Arc::new(vm)), "cannot create vm");
    device.create();
    device.create();
    Ok(())
}
