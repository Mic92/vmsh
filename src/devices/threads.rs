use crate::devices::mmio::IoPirate;
use crate::stage1::DeviceStatus;
use crate::stage1::DriverStatus;
use event_manager::EventManager;
use event_manager::MutEventSubscriber;
use log::debug;
use log::error;
use log::{info, log_enabled, trace, Level};
use simple_error::{bail, require_with, simple_error, try_with};
use stage1_interface::DeviceState;
use std::path::{Path, PathBuf};
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
pub struct DriverNotifier {
    // supress warning because of https://github.com/rust-lang/rust-clippy/issues/1516
    #[allow(clippy::mutex_atomic)]
    lock: Mutex<DeviceState>,
    condvar: Condvar,
    device_status: DeviceStatus,
    driver_status: DriverStatus,
    hv: Arc<Hypervisor>,
}

// supress warning because of https://github.com/rust-lang/rust-clippy/issues/1516
#[allow(clippy::mutex_atomic)]
impl DriverNotifier {
    /// Creates a new DriverNotifier structure
    fn new(device_status: DeviceStatus, driver_status: DriverStatus, hv: Arc<Hypervisor>) -> Self {
        DriverNotifier {
            lock: Mutex::new(DeviceState::Initializing),
            condvar: Condvar::new(),
            device_status,
            driver_status,
            hv,
        }
    }

    fn notify(&self, state: DeviceState) -> Result<()> {
        let mut state_guard = try_with!(self.lock.lock(), "failed to lock");
        if *state_guard == DeviceState::Initializing {
            *state_guard = state;
            try_with!(
                self.device_status.update(&self.hv, state),
                "failed to notify stage1 in VM"
            );
        }
        self.condvar.notify_all();
        Ok(())
    }

    pub fn terminate(&self) -> Result<()> {
        let mut state_guard = try_with!(self.lock.lock(), "failed to lock");
        if *state_guard == DeviceState::Initializing {
            bail!("cannot terminate unitialized device");
        }

        *state_guard = DeviceState::Terminating;
        try_with!(
            self.device_status
                .update(&self.hv, DeviceState::Terminating),
            "failed to notify stage1 in VM about termination"
        );

        loop {
            match try_with!(
                self.driver_status.check(&self.hv),
                "cannot check device state"
            ) {
                DeviceState::Ready => {}
                DeviceState::Terminating => break,
                s => {
                    bail!("unexpected driver state: {:?}", s);
                }
            }
        }

        Ok(())
    }

    /// Blocks until block device is ready
    fn wait(&self) -> Result<()> {
        let mut state = try_with!(self.lock.lock(), "failed to lock");
        while *state == DeviceState::Initializing {
            state = try_with!(self.condvar.wait(state), "failed to wait for condvar");
        }
        Ok(())
    }
}

impl Drop for DriverNotifier {
    fn drop(&mut self) {
        match self.lock.lock() {
            Ok(started) if *started == DeviceState::Initializing => {
                if let Err(e) = self.device_status.update(&self.hv, DeviceState::Error) {
                    error!("failed to update device status: {}", e);
                }
            }
            Ok(_) => {}
            Err(e) => {
                error!("cannot update device status: failed to take lock: {}", e);
            }
        }
    }
}

fn event_thread(
    mut event_mgr: SubscriberEventManager,
    device_space: &DeviceContext,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<(), Option<Arc<DeviceContext>>>> {
    let blkdev = device_space.blkdev.clone();
    let ack_handler = {
        let blkdev = try_with!(blkdev.lock(), "cannot unlock thread");
        blkdev.irq_ack_handler.clone()
    };
    log::debug!("event thread started");

    let res = InterrutableThread::spawn(
        "event-manager",
        err_sender,
        move |_ctx: &Option<Arc<DeviceContext>>, should_stop: Arc<AtomicBool>| {
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
        None,
    );
    Ok(try_with!(res, "failed to spawn event-manager thread"))
}

/// Periodically print block device state
fn blkdev_monitor_thread(
    device: &DeviceContext,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<(), Option<Arc<DeviceContext>>>> {
    let blkdev = device.blkdev.clone();
    let res = InterrutableThread::spawn(
        "blkdev-monitor",
        err_sender,
        move |_ctx: &Option<Arc<DeviceContext>>, should_stop: Arc<AtomicBool>| {
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
        None,
    );

    Ok(try_with!(res, "failed to spawn blkdev-monitor"))
}

/// Traps KVM_MMIO_EXITs with ptrace and forward them as needed to our block and console device driver
fn handle_mmio_exits(
    wrapper_mo: &Mutex<Option<KvmRunWrapper>>,
    should_stop: &Arc<AtomicBool>,
    ctx: &DeviceContext,
    driver_notifier: &Arc<DriverNotifier>,
) -> Result<()> {
    let mut mmio_mgr = try_with!(ctx.mmio_mgr.lock(), "cannot lock mmio manager");
    let mut wrapper_go = try_with!(wrapper_mo.lock(), "cannot obtain wrapper mutex");
    let wrapper_g = require_with!(wrapper_go.as_mut(), "KvmRunWrapper not initialized");
    try_with!(
        wrapper_g.stop_on_syscall(),
        "failed to wait for vmm exit_mmio"
    );
    info!("device ready!");
    driver_notifier.notify(DeviceState::Ready)?;

    loop {
        let mut kvm_exit = try_with!(
            wrapper_g.wait_for_ioctl(),
            "failed to wait for vmm exit_mmio"
        );

        if let Some(mmio_rw) = &mut kvm_exit {
            if ctx.first_mmio_addr <= mmio_rw.addr && mmio_rw.addr < ctx.last_mmio_addr {
                // intercept op
                trace!("mmio access: {:#x}", mmio_rw.addr);
                try_with!(mmio_mgr.handle_mmio_rw(mmio_rw), "failed to handle MmioRw");
            } else {
                // do nothing, just continue to ignore and pass to hv
                trace!("ignore addr: {:#x}", mmio_rw.addr)
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
    device: Arc<DeviceContext>,
    err_sender: &SyncSender<()>,
    driver_notifier: &Arc<DriverNotifier>,
) -> Result<InterrutableThread<(), Option<Arc<DeviceContext>>>> {
    let driver_notifier = Arc::clone(driver_notifier);
    let vm = Arc::clone(vm);
    vm.prepare_thread_transfer()?;

    let res = InterrutableThread::spawn(
        "mmio-exit-handler",
        err_sender,
        move |dev: &Option<Arc<DeviceContext>>, should_stop: Arc<AtomicBool>| {
            let dev = require_with!(dev.as_ref(), "no device passed");
            if let Err(e) = vm.finish_thread_transfer() {
                // don't shadow error here
                let _ = driver_notifier.notify(DeviceState::Error);
                bail!("failed transfer ptrace to mmio exit handler: {}", e);
            };

            info!("mmio dev attached");

            let res = vm.kvmrun_wrapped(|wrapper_mo: &Mutex<Option<KvmRunWrapper>>| {
                // Signal that our blockdevice driver is ready now
                let res = handle_mmio_exits(wrapper_mo, &should_stop, dev, &driver_notifier);
                if res.is_err() {
                    // don't shadow error here
                    let _ = driver_notifier.notify(DeviceState::Error);
                }
                res
            });

            // we need to return ptrace control before returning to the main thread
            vm.prepare_thread_transfer()?;
            res
        },
        Some(device),
    );

    Ok(try_with!(res, "cannot spawn mmio exit handler thread"))
}

pub struct DeviceSet {
    context: Arc<DeviceContext>,
    event_manager: SubscriberEventManager,
}

fn ioregion_event_loop(
    should_stop: &Arc<AtomicBool>,
    mmio_mgr: Arc<Mutex<IoPirate>>,
    device: Arc<Mutex<dyn MaybeIoRegionFd + Send>>,
) -> Result<()> {
    let mut ioregionfd = {
        let mut device = try_with!(device.lock(), "cannot lock device");
        let ioregion = device.get_ioregionfd();
        let ioregion = ioregion.as_mut().ok_or_else(|| {
            simple_error!("cannot start ioregion event loop when ioregion does not exist")
        })?;
        ioregion.fdclone()
    };

    loop {
        let cmd = try_with!(
            ioregionfd.read(),
            "cannot read mmio command from ioregionfd (fd {:?})",
            ioregionfd
        );
        if let Some(cmd) = cmd {
            let mut mmio_mgr = try_with!(
                mmio_mgr.lock(),
                "cannot lock mmio manager to handle mmio command"
            );
            mmio_mgr.handle_ioregion_rw(&ioregionfd, cmd)?;
            drop(mmio_mgr);
        }

        if should_stop.load(Ordering::Relaxed) {
            break;
        }
    }
    Ok(())
}

/// see handle_mmio_exits
fn ioregion_handler_thread(
    devices: Arc<DeviceContext>,
    device: Arc<Mutex<dyn MaybeIoRegionFd + Send>>,
    mmio_mgr: Arc<Mutex<IoPirate>>,
    err_sender: &SyncSender<()>,
) -> Result<InterrutableThread<(), Option<Arc<DeviceContext>>>> {
    let res = InterrutableThread::spawn(
        "ioregion-handler",
        err_sender,
        move |_ctx: &Option<Arc<DeviceContext>>, should_stop: Arc<AtomicBool>| {
            info!("ioregion mmio handler started");
            try_with!(
                ioregion_event_loop(&should_stop, mmio_mgr, device),
                "ioregion_event_loop failed"
            );

            Ok(())
        },
        Some(devices),
    );

    Ok(try_with!(res, "cannot spawn mmio exit handler thread"))
}
pub type Threads = Vec<InterrutableThread<(), Option<Arc<DeviceContext>>>>;

impl DeviceSet {
    pub fn mmio_addrs(&self) -> Result<Vec<u64>> {
        self.context.mmio_addrs()
    }

    pub fn new(
        vm: &Arc<Hypervisor>,
        allocator: &mut PhysMemAllocator,
        irq_num: usize,
        backing_file: &Path,
        pts: Option<PathBuf>,
    ) -> Result<DeviceSet> {
        let mut event_manager =
            try_with!(SubscriberEventManager::new(), "cannot create event manager");
        // instantiate blkdev
        let context = Arc::new(try_with!(
            DeviceContext::new(
                vm,
                allocator,
                &mut event_manager,
                irq_num,
                backing_file,
                pts
            ),
            "cannot create device context"
        ));
        Ok(DeviceSet {
            context,
            event_manager,
        })
    }

    pub fn start(
        self,
        vm: &Arc<Hypervisor>,
        device_status: DeviceStatus,
        driver_status: DriverStatus,
        err_sender: &SyncSender<()>,
    ) -> Result<(Threads, Arc<DriverNotifier>)> {
        let driver_notifier = Arc::new(DriverNotifier::new(
            device_status,
            driver_status,
            Arc::clone(vm),
        ));
        let mut threads = vec![event_thread(self.event_manager, &self.context, err_sender)?];

        if log_enabled!(Level::Debug) {
            threads.push(blkdev_monitor_thread(&self.context, err_sender)?);
        }

        if devices::use_ioregionfd() {
            vm.resume()?;
            // Device was ready already before that but this way,
            // we only only indicate readiness just before we create our io threads.
            try_with!(
                driver_notifier.notify(DeviceState::Ready),
                "cannot update device status"
            );
            threads.push(try_with!(
                ioregion_handler_thread(
                    self.context.clone(),
                    self.context.blkdev.clone(),
                    self.context.mmio_mgr.clone(),
                    err_sender,
                ),
                "cannot spawn block ioregion handler"
            ));
            threads.push(try_with!(
                ioregion_handler_thread(
                    self.context.clone(),
                    self.context.console.clone(),
                    self.context.mmio_mgr.clone(),
                    err_sender,
                ),
                "cannot spawn console ioregion handler"
            ));
        } else {
            threads.push(mmio_exit_handler_thread(
                vm,
                self.context,
                err_sender,
                &driver_notifier,
            )?);
        }

        driver_notifier.wait()?;
        Ok((threads, driver_notifier))
    }
}
