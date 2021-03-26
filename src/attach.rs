//mod device;

use crate::result::Result;
use simple_error::try_with;
use std::sync::Arc;

use crate::device::Device;
use crate::inspect::InspectOptions;
use crate::kvm;

pub fn attach(opts: &InspectOptions) -> Result<()> {
    println!("attaching");

    let vm = try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );

    let device = try_with!(Device::new(&Arc::new(vm)), "cannot create vm");
    device.create();
    device.create();
    Ok(())
}
