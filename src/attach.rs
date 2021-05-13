use crate::device::Block;
use crate::result::Result;
use event_manager::EventManager;
use event_manager::MutEventSubscriber;
use log::*;
use nix::unistd::Pid;
use simple_error::try_with;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use vm_device::bus::MmioAddress;
use vm_virtio::device::VirtioDevice;
use vm_virtio::device::WithDriverSelect;
use vm_virtio::Queue;

use crate::device::{Device, DEVICE_MAX_MEM};
use crate::kvm::{self, hypervisor::Hypervisor};
use crate::tracer::wrap_syscall::KvmRunWrapper;

// Arc<Mutex<>> because the same device (a dyn DevicePio/DeviceMmio from IoManager's
// perspective, and a dyn MutEventSubscriber from EventManager's) is managed by the 2 entities,
// and isn't Copy-able; so once one of them gets ownership, the other one can't anymore.
pub type SubscriberEventManager = EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>;

pub struct AttachOptions {
    pub pid: Pid,
    pub backing: PathBuf,
}

pub fn attach(opts: &AttachOptions) -> Result<()> {
    info!("attaching");

    let vm = Arc::new(try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    ));
    vm.stop()?;

    let mut event_manager = try_with!(SubscriberEventManager::new(), "cannot create event manager");
    // TODO add subscriber (wrapped_exit_handler) and stuff?

    // instantiate blkdev
    let device = try_with!(
        Device::new(&vm, &mut event_manager, &opts.backing),
        "cannot create vm"
    );
    info!("mmio dev attached");
    event_thread(event_manager);

    let child = blkdev_monitor_thread(&device);

    // run guest until driver has inited
    try_with!(
        run_kvm_wrapped(&vm, &device),
        "device init stage with KvmRunWrapper failed"
    );
    info!("blkdev queue ready.");
    vm.resume()?;

    info!("pause");
    nix::unistd::pause();
    let _err = child.join();
    Ok(())
}

fn event_thread(mut event_mgr: SubscriberEventManager) {
    thread::spawn(move || loop {
        match event_mgr.run() {
            Ok(nr) => log::debug!("EventManager: processed {} events", nr),
            Err(e) => log::warn!("Failed to handle events: {:?}", e),
        }
        // TODO if !self.exit_handler.keep_running() { break; }
    });
}

fn exit_condition(_blkdev: &Arc<Mutex<Block>>) -> Result<bool> {
    Ok(false)
    // TODO think of teardown
    //let blkdev = &try_with!(blkdev.lock(), "cannot get blkdev lock");
    //Ok(blkdev.selected_queue().map(|q| q.ready).unwrap())
}

fn blkdev_monitor_thread(device: &Device) -> JoinHandle<()> {
    let blkdev = device.blkdev.clone();
    thread::spawn(move || loop {
        {
            match exit_condition(&blkdev) {
                Err(e) => warn!("cannot evaluate exit condition: {}", e),
                Ok(b) => {
                    if b {
                        break;
                    }
                }
            }
            let blkdev = blkdev.lock().unwrap();
            info!("");
            info!("dev type {}", blkdev.device_type());
            info!("dev features b{:b}", blkdev.device_features());
            info!(
                "dev interrupt stat b{:b}",
                blkdev
                    .interrupt_status()
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
            info!("dev status b{:b}", blkdev.device_status());
            info!("dev config gen {}", blkdev.config_generation());
            info!(
                "dev selqueue max size {}",
                blkdev.selected_queue().map(Queue::max_size).unwrap()
            );
            info!(
                "dev selqueue ready {}",
                blkdev.selected_queue().map(|q| q.ready).unwrap()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    })
}

/// returns when blkdev queue is ready
fn run_kvm_wrapped(vm: &Arc<Hypervisor>, device: &Device) -> Result<()> {
    let mut mmio_mgr = device.mmio_mgr.lock().unwrap();

    vm.kvmrun_wrapped(|wrapper_mo: &Mutex<Option<KvmRunWrapper>>| {
        let mmio_space = {
            let blkdev = device.blkdev.clone();
            let blkdev = &try_with!(blkdev.lock(), "TODO");
            blkdev.mmio_cfg.range
        };

        loop {
            let mut kvm_exit;
            {
                let mut wrapper_go = try_with!(wrapper_mo.lock(), "cannot obtain wrapper mutex");
                let wrapper_g = wrapper_go.as_mut().unwrap(); // kvmrun_wrapped guarentees Some()
                kvm_exit = try_with!(
                    wrapper_g.wait_for_ioctl(),
                    "failed to wait for vmm exit_mmio"
                );
            }
            if let Some(mmio_rw) = &mut kvm_exit {
                let addr = MmioAddress(mmio_rw.addr);
                let from = mmio_space.base();
                // virtio mmio space + virtio device specific space
                let to = from + mmio_space.size() + DEVICE_MAX_MEM;
                if from <= addr && addr < to {
                    // intercept op
                    debug!("mmio access 0x{:x}", addr.0);
                    try_with!(mmio_mgr.handle_mmio_rw(mmio_rw), "failed to handle MmioRw");
                } else {
                    // do nothing, just continue to ingore and pass to hv
                }
                if exit_condition(&device.blkdev)? {
                    break;
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}
