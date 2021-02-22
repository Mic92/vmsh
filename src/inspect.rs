use crate::result::Result;
use nix::unistd::Pid;
use simple_error::try_with;

use crate::kvm;

pub struct InspectOptions {
    pub pid: Pid,
}

pub fn inspect(opts: &InspectOptions) -> Result<()> {
    let vm = try_with!(
        kvm::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    vm.get_maps()?;
    Ok(())
}
