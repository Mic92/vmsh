use event_manager::EventManager;
use event_manager::MutEventSubscriber;
use log::{debug, info, log_enabled, warn, Level};
use simple_error::{require_with, try_with};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::sync::{Condvar, Mutex};
use vm_device::bus::MmioAddress;
use vm_virtio::device::VirtioDevice;
use vm_virtio::device::WithDriverSelect;
use vm_virtio::Queue;

use crate::device::{Block, Device, DEVICE_MAX_MEM};
use crate::interrutable_thread::InterrutableThread;
use crate::kvm::hypervisor::Hypervisor;
use crate::result::Result;
use crate::tracer::wrap_syscall::KvmRunWrapper;

// Arc<Mutex<>> because the same device (a dyn DevicePio/DeviceMmio from IoManager's
// perspective, and a dyn MutEventSubscriber from EventManager's) is managed by the 2 entities,
// and isn't Copy-able; so once one of them gets ownership, the other one can't anymore.
pub type SubscriberEventManager = EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>;

/// data structure to wait for block device to become ready
struct DeviceReady {
    // supress warning because of https://github.com/rust-lang/rust-clippy/issues/1516
    #[allow(clippy::mutex_atomic)]
    lock: Mutex<bool>,
    condvar: Condvar,
}

// supress warning because of https://github.com/rust-lang/rust-clippy/issues/1516
#[allow(clippy::mutex_atomic)]
impl DeviceReady {
    /// Creates a new DeviceReady structure
    fn new() -> Self {
        DeviceReady {
            lock: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }
    fn notify(&self) -> Result<()> {
        let mut started = try_with!(self.lock.lock(), "failed to lock");
        *started = true;
        self.condvar.notify_all();
        Ok(())
    }

    /// Blocks until block device is ready
    fn wait(&self) -> Result<()> {
        let mut started = try_with!(self.lock.lock(), "failed to lock");
        while !*started {
            started = try_with!(self.condvar.wait(started), "failed to wait for condvar");
        }
        Ok(())
    }
}

fn event_thread(
    mut event_mgr: SubscriberEventManager,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<()>> {
    let res = InterrutableThread::spawn(
        "event-manager",
        err_sender,
        move |should_stop: Arc<AtomicBool>| {
            loop {
                match event_mgr.run_with_timeout(500) {
                    Ok(nr) => log::debug!("EventManager: processed {} events", nr),
                    Err(e) => log::warn!("Failed to handle events: {:?}", e),
                }
                if should_stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            Ok(())
        },
    );
    Ok(try_with!(res, "failed to spawn event-manager thread"))
}

fn exit_condition(_blkdev: &Arc<Mutex<Block>>) -> Result<bool> {
    Ok(false)
    // TODO think of teardown
    //let blkdev = &try_with!(blkdev.lock(), "cannot get blkdev lock");
    //Ok(blkdev.selected_queue().map(|q| q.ready).unwrap())
}

/// Periodically print block device state
fn blkdev_monitor_thread(
    device: &Device,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<()>> {
    let blkdev = device.blkdev.clone();
    let res = InterrutableThread::spawn(
        "blkdev-monitor",
        err_sender,
        move |should_stop: Arc<AtomicBool>| loop {
            match exit_condition(&blkdev) {
                Err(e) => warn!("cannot evaluate exit condition: {}", e),
                Ok(b) => {
                    if b {
                        return Ok(());
                    }
                }
            }
            {
                let blkdev = try_with!(blkdev.lock(), "cannot unlock thread");
                debug!("");
                debug!("dev type {}", blkdev.device_type());
                debug!("dev features b{:b}", blkdev.device_features());
                debug!(
                    "dev interrupt stat b{:b}",
                    blkdev.interrupt_status().load(Ordering::Relaxed)
                );
                debug!("dev status b{:b}", blkdev.device_status());
                debug!("dev config gen {}", blkdev.config_generation());
                debug!(
                    "dev selqueue max size {}",
                    blkdev.selected_queue().map(Queue::max_size).unwrap()
                );
                debug!(
                    "dev selqueue ready {}",
                    blkdev.selected_queue().map(|q| q.ready).unwrap()
                );
            }

            if should_stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        },
    );

    Ok(try_with!(res, "failed to spawn blkdev-monitor"))
}

/// Traps KVM_MMIO_EXITs with ptrace and forward them as needed to out block device driver
fn handle_mmio_exits(
    wrapper_mo: &Mutex<Option<KvmRunWrapper>>,
    should_stop: &Arc<AtomicBool>,
    device: &Device,
    device_ready: &Arc<DeviceReady>,
) -> Result<()> {
    let mut mmio_mgr = try_with!(device.mmio_mgr.lock(), "cannot lock mmio manager");
    let mmio_space = {
        let blkdev = device.blkdev.clone();
        let blkdev = try_with!(blkdev.lock(), "cannot lock block device driver");
        blkdev.mmio_cfg.range
    };
    device_ready.notify()?;

    loop {
        let mut kvm_exit;
        {
            let mut wrapper_go = try_with!(wrapper_mo.lock(), "cannot obtain wrapper mutex");
            let wrapper_g = require_with!(wrapper_go.as_mut(), "KvmRunWrapper not initialized");
            kvm_exit = try_with!(
                wrapper_g.wait_for_ioctl(),
                "failed to wait for vmm exit_mmio"
            )
        };

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

        if should_stop.load(Ordering::Relaxed) {
            break;
        }
    }
    Ok(())
}

/// see handle_mmio_exits
fn mmio_exit_handler_thread(
    vm: &Arc<Hypervisor>,
    device: Device,
    err_sender: &SyncSender<()>,
    device_ready: &Arc<DeviceReady>,
) -> Result<InterrutableThread<()>> {
    let device_ready = Arc::clone(device_ready);
    let vm = Arc::clone(vm);
    info!("mmio dev attached");

    vm.prepare_thread_transfer()?;

    let res = InterrutableThread::spawn(
        "mmio-exit-handler",
        err_sender,
        move |should_stop: Arc<AtomicBool>| {
            vm.finish_thread_transfer()?;

            info!("mmio dev attached");

            let res = vm.kvmrun_wrapped(|wrapper_mo: &Mutex<Option<KvmRunWrapper>>| {
                // Signal that our blockdevice driver is ready now
                handle_mmio_exits(wrapper_mo, &should_stop, &device, &device_ready)?;

                Ok(())
            });

            // we need to return ptrace control before returning to the main thread
            vm.prepare_thread_transfer()?;
            res
        },
    );

    Ok(try_with!(res, "cannot spawn mmio exit handler thread"))
}

pub fn create_block_device(
    vm: &Arc<Hypervisor>,
    err_sender: &SyncSender<()>,
    backing_file: &Path,
) -> Result<Vec<InterrutableThread<()>>> {
    let mut event_manager = try_with!(SubscriberEventManager::new(), "cannot create event manager");
    // instantiate blkdev
    let device = try_with!(
        Device::new(&vm, &mut event_manager, backing_file),
        "cannot create vm"
    );

    let device_ready = Arc::new(DeviceReady::new());
    let mut threads = vec![event_thread(event_manager, err_sender)?];

    if log_enabled!(Level::Debug) {
        threads.push(blkdev_monitor_thread(&device, err_sender)?);
    }
    threads.push(mmio_exit_handler_thread(
        vm,
        device,
        err_sender,
        &device_ready,
    )?);

    device_ready.wait()?;

    Ok(threads)
}
