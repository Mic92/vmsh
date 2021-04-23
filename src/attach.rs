use crate::result::Result;
use simple_error::try_with;
use std::sync::Arc;
use vm_device::bus::MmioAddress;

use crate::device::Device;
use crate::inspect::InspectOptions;
use crate::kvm::{self, hypervisor::Hypervisor};
use crate::tracer::wrap_syscall::KvmRunWrapper;

pub fn attach(opts: &InspectOptions) -> Result<()> {
    println!("attaching");

    let vm = Arc::new(try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    ));
    vm.stop()?;

    let device = try_with!(Device::new(&vm), "cannot create vm");
    println!("mmio dev attached");

    try_with!(
        run_kvm_wrapped(&vm, &device),
        "device init stage with KvmRunWrapper failed"
    );

    device.create();
    device.create();
    println!("pause");
    nix::unistd::pause();
    Ok(())
}

fn run_kvm_wrapped(vm: &Arc<Hypervisor>, device: &Device) -> Result<()> {
    let mut mmio_mgr = device.mmio_mgr.lock().unwrap();

    vm.kvmrun_wrapped(|wrapper: &mut KvmRunWrapper| {
        let mmio_space = {
            let blkdev = device.blkdev.clone();
            let blkdev = &try_with!(blkdev.lock(), "TODO");
            blkdev.mmio_cfg.range
        };

        loop {
            let mut kvm_exit =
                try_with!(wrapper.wait_for_ioctl(), "failed to wait for vmm exit_mmio");
            if let Some(mmio_rw) = &mut kvm_exit {
                let addr = MmioAddress(mmio_rw.addr);
                if mmio_space.base() <= addr && addr <= mmio_space.last() {
                    // intercept op
                    try_with!(mmio_mgr.handle_mmio_rw(mmio_rw), "failed to handle MmioRw");
                } else {
                    // do nothing, just continue to ingore and pass to hv
                }
                if device.mmio_device_space.queue_ready == 0x1 {
                    println!("queue ready. break.");
                    break;
                }
            }
        }

        Ok(())
    })?;
    Ok(())
}
