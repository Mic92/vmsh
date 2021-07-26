use crate::devices::mmio::IoPirate;
use event_manager::EventManager;
use event_manager::MutEventSubscriber;
use log::{debug, info, log_enabled, trace, Level};
use simple_error::{bail, require_with, simple_error, try_with};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::sync::{Condvar, Mutex};
use virtio_device::{VirtioDevice, WithDriverSelect};

use crate::devices;
use crate::devices::DeviceContext;
use crate::devices::MaybeIoRegionFd;
use crate::interrutable_thread::InterrutableThread;
use crate::kvm::hypervisor::Hypervisor;
use crate::kvm::PhysMemAllocator;
use crate::result::Result;
use crate::tracer::wrap_syscall::KvmRunWrapper;

const EVENT_LOOP_TIMEOUT_MS: i32 = 1;

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
    device_space: &DeviceContext,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<()>> {
    let blkdev = device_space.blkdev.clone();
    let ack_handler = {
        let blkdev = try_with!(blkdev.lock(), "cannot unlock thread");
        blkdev.irq_ack_handler.clone()
    };
    log::debug!("event thread started");

    let res = InterrutableThread::spawn(
        "event-manager",
        err_sender,
        move |should_stop: Arc<AtomicBool>| {
            loop {
                match event_mgr.run_with_timeout(EVENT_LOOP_TIMEOUT_MS) {
                    Ok(nr) => {
                        if nr != 0 {
                            trace!("EventManager: processed {} events", nr)
                        }
                    }
                    Err(e) => log::warn!("Failed to handle events: {:?}", e),
                }
                {
                    let mut ack_handler = try_with!(ack_handler.lock(), "failed to lock");
                    ack_handler.handle_timeouts();
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

/// Periodically print block device state
fn blkdev_monitor_thread(
    device: &DeviceContext,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<()>> {
    let blkdev = device.blkdev.clone();
    let res = InterrutableThread::spawn(
        "blkdev-monitor",
        err_sender,
        move |should_stop: Arc<AtomicBool>| {
            //std::thread::sleep(std::time::Duration::from_millis(10000));
            loop {
                {
                    let blkdev = try_with!(blkdev.lock(), "cannot unlock thread");
                    // debug!("");
                    // debug!("dev type {}", blkdev.device_type());
                    // debug!("dev features b{:b}", blkdev.device_features());
                    // debug!(
                    //     "dev interrupt stat b{:b}",
                    //     blkdev.interrupt_status().load(Ordering::Relaxed)
                    // );
                    // debug!("dev status b{:b}", blkdev.device_status());
                    // debug!("dev config gen {}", blkdev.config_generation());
                    // debug!(
                    //     "dev queue {}: max size: {}, ready: {}, is valid: {}",
                    //     //blkdev.selected_queue().map(Queue::max_size).unwrap()
                    //     blkdev.queue_select(),
                    //     blkdev.selected_queue().unwrap().max_size(),
                    //     blkdev.selected_queue().unwrap().ready,
                    //     blkdev.selected_queue().unwrap().is_valid() // crashes before activate()
                    // );
                    // debug!(
                    //     "dev queue {}: avail idx {}, next avail idx {}",
                    //     blkdev.queue_select(),
                    //     blkdev.selected_queue().unwrap().avail_idx(Ordering::Relaxed).unwrap(),
                    //     blkdev.selected_queue().unwrap().next_avail(),
                    //     );
                    debug!(
                        "dev queue {}: irq status b{:b}",
                        blkdev.queue_select(),
                        blkdev.interrupt_status().load(Ordering::SeqCst),
                    );

                    //debug!("occasional irqfd << 1");
                    //blkdev.irqfd.write(1).unwrap();
                }

                if should_stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(10000));
            }
        },
    );

    Ok(try_with!(res, "failed to spawn blkdev-monitor"))
}

/// Traps KVM_MMIO_EXITs with ptrace and forward them as needed to our block and console device driver
fn handle_mmio_exits(
    wrapper_mo: &Mutex<Option<KvmRunWrapper>>,
    should_stop: &Arc<AtomicBool>,
    ctx: &DeviceContext,
    device_ready: &Arc<DeviceReady>,
) -> Result<()> {
    let mut mmio_mgr = try_with!(ctx.mmio_mgr.lock(), "cannot lock mmio manager");
    device_ready.notify()?;

    loop {
        let mut kvm_exit;
        {
            let mut wrapper_go = try_with!(wrapper_mo.lock(), "cannot obtain wrapper mutex");
            let wrapper_g = require_with!(wrapper_go.as_mut(), "KvmRunWrapper not initialized");
            kvm_exit = try_with!(
                wrapper_g.wait_for_ioctl(),
                "failed to wait for vmm exit_mmio"
            );
        };

        if let Some(mmio_rw) = &mut kvm_exit {
            if ctx.first_mmio_addr <= mmio_rw.addr && mmio_rw.addr < ctx.last_mmio_addr {
                // intercept op
                trace!("mmio access: 0x{:x}", mmio_rw.addr);
                try_with!(mmio_mgr.handle_mmio_rw(mmio_rw), "failed to handle MmioRw");
            } else {
                // do nothing, just continue to ignore and pass to hv
                trace!("ignore addr: 0x{:x}", mmio_rw.addr)
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
    device: DeviceContext,
    err_sender: &SyncSender<()>,
    device_ready: &Arc<DeviceReady>,
) -> Result<InterrutableThread<()>> {
    let device_ready = Arc::clone(device_ready);
    let vm = Arc::clone(vm);
    vm.prepare_thread_transfer()?;

    let res = InterrutableThread::spawn(
        "mmio-exit-handler",
        err_sender,
        move |should_stop: Arc<AtomicBool>| {
            if let Err(e) = vm.finish_thread_transfer() {
                bail!("failed transfer ptrace to mmio exit handler: {}", e);
            };

            info!("mmio dev attached");

            let res = vm.kvmrun_wrapped(|wrapper_mo: &Mutex<Option<KvmRunWrapper>>| {
                // Signal that our blockdevice driver is ready now
                handle_mmio_exits(wrapper_mo, &should_stop, &device, &device_ready)?;

                Ok(())
            });

            // drop remote resources like ioeventfd before disowning traced process.
            drop(device);

            // we need to return ptrace control before returning to the main thread
            vm.prepare_thread_transfer()?;
            res
        },
    );

    Ok(try_with!(res, "cannot spawn mmio exit handler thread"))
}

pub struct DeviceSet {
    context: DeviceContext,
    event_manager: SubscriberEventManager,
}

fn ioregion_event_loop(
    should_stop: &Arc<AtomicBool>,
    device_ready: &Arc<DeviceReady>,
    mmio_mgr: Arc<Mutex<IoPirate>>,
    device: Arc<Mutex<dyn MaybeIoRegionFd + Send>>,
) -> Result<()> {
    let ioregionfd = {
        let device = try_with!(device.lock(), "cannot lock device");
        let ioregionfd = device.get_ioregionfd();
        ioregionfd.ok_or(simple_error!(
            "cannot start ioregion event loop when ioregion does not exist"
        ))?
    };
    device_ready.notify()?;

    loop {
        let cmd = try_with!(
            ioregionfd.read(),
            "cannot read mmio command from ioregionfd (fd {:?})",
            ioregionfd
        );
        {
            let mut mmio_mgr = try_with!(
                mmio_mgr.lock(),
                "cannot lock mmio manager to handle mmio command"
            );
            mmio_mgr.handle_ioregion_rw(&ioregionfd, cmd)?;
        }

        if should_stop.load(Ordering::Relaxed) {
            break;
        }
    }
    Ok(())
}

/// see handle_mmio_exits
fn ioregion_handler_thread(
    device: Arc<Mutex<dyn MaybeIoRegionFd + Send>>,
    mmio_mgr: Arc<Mutex<IoPirate>>,
    err_sender: &SyncSender<()>,
    device_ready: &Arc<DeviceReady>,
) -> Result<InterrutableThread<()>> {
    let device_ready = device_ready.clone();

    let res = InterrutableThread::spawn(
        "ioregion-handler",
        err_sender,
        move |should_stop: Arc<AtomicBool>| {
            info!("ioregion mmio handler started");
            try_with!(
                ioregion_event_loop(&should_stop, &device_ready, mmio_mgr, device),
                "foo"
            );

            // TODO drop remote resources like ioeventfd before disowning traced process?
            //drop(device);
            Ok(())
        },
    );

    Ok(try_with!(res, "cannot spawn mmio exit handler thread"))
}

impl DeviceSet {
    pub fn mmio_addrs(&self) -> Result<Vec<u64>> {
        self.context.mmio_addrs()
    }

    pub fn new(
        vm: &Arc<Hypervisor>,
        allocator: &mut PhysMemAllocator,
        backing_file: &Path,
    ) -> Result<DeviceSet> {
        let mut event_manager =
            try_with!(SubscriberEventManager::new(), "cannot create event manager");
        // instantiate blkdev
        let context = try_with!(
            DeviceContext::new(vm, allocator, &mut event_manager, backing_file),
            "cannot create vm"
        );
        Ok(DeviceSet {
            context,
            event_manager,
        })
    }

    pub fn start(
        self,
        vm: &Arc<Hypervisor>,
        err_sender: &SyncSender<()>,
    ) -> Result<Vec<InterrutableThread<()>>> {
        let device_ready = Arc::new(DeviceReady::new());
        let mut threads = vec![event_thread(self.event_manager, &self.context, err_sender)?];

        if log_enabled!(Level::Debug) {
            threads.push(blkdev_monitor_thread(&self.context, err_sender)?);
        }

        if devices::USE_IOREGIONFD {
            vm.resume()?;
            threads.push(try_with!(
                ioregion_handler_thread(
                    self.context.blkdev,
                    self.context.mmio_mgr.clone(),
                    err_sender,
                    &device_ready,
                ),
                "cannot spawn block ioregion handler"
            ));
            threads.push(try_with!(
                ioregion_handler_thread(
                    self.context.console,
                    self.context.mmio_mgr,
                    err_sender,
                    &device_ready,
                ),
                "cannot spawn console ioregion handler"
            ));
        } else {
            threads.push(mmio_exit_handler_thread(
                vm,
                self.context,
                err_sender,
                &device_ready,
            )?);
        }

        device_ready.wait()?;
        Ok(threads)
    }
}
