use crate::cpu::Regs;
use libc::c_void;
use log::{debug, info};
/// This module loads kernel code into the VM that we want to attach to.
use simple_error::bail;
use simple_error::try_with;
use stage1_interface::DeviceState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Duration;

use crate::interrutable_thread::InterrutableThread;
use crate::kernel::find_kernel;
use crate::kvm;
use crate::kvm::hypervisor::{memory::process_read, memory::process_write, Hypervisor};
use crate::loader::Loader;
use crate::page_table::VirtMem;
use crate::result::Result;

const STAGE1_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libstage1.so"));

pub struct Stage1 {
    #[allow(unused)]
    virt_mem: VirtMem,
    pub device_status: Option<DeviceStatus>,
    pub driver_status: Option<DriverStatus>,
    regs: Regs,
}

pub struct DeviceStatus {
    pub host_addr: usize,
}

impl DeviceStatus {
    pub fn update(&self, hv: &Hypervisor, state: DeviceState) -> Result<()> {
        try_with!(
            process_write(hv.pid, self.host_addr as *mut c_void, &state),
            "failed to write state field to hypervisor memory"
        );
        Ok(())
    }
}

#[derive(Clone)]
pub struct DriverStatus {
    pub host_addr: usize,
}

impl DriverStatus {
    pub fn check(&self, hv: &Hypervisor) -> Result<DeviceState> {
        process_read(hv.pid, self.host_addr as *mut c_void)
    }
}

impl Stage1 {
    pub fn new(
        mut allocator: kvm::PhysMemAllocator,
        command: &[String],
        mmio_ranges: Vec<u64>,
    ) -> Result<Stage1> {
        let kernel = find_kernel(&allocator.guest_mem, &allocator.hv)?;

        let mut regs = try_with!(
            allocator.hv.get_regs(&allocator.hv.vcpus[0]),
            "failed to get vm registers"
        );

        let mut loader = try_with!(
            Loader::new(STAGE1_LIB, &kernel, regs.ip() as usize, &mut allocator),
            "cannot load stage1"
        );

        let init_func = loader.init_func;

        let (virt_mem, device_status, driver_status) = try_with!(
            loader.load_binary(command, mmio_ranges),
            "cannot load stage1"
        );

        debug!(
            "load stage1 ({} kB) into vm at address {}",
            STAGE1_LIB.len() / 1024,
            virt_mem.mappings[0].virt_start
        );

        if regs.is_userspace() {
            bail!("vcpu was stopped in userspace. This is not supported");
        }
        regs.set_ip(init_func as u64);

        Ok(Stage1 {
            virt_mem,
            device_status: Some(device_status),
            driver_status: Some(driver_status),
            regs,
        })
    }

    pub fn spawn(
        &self,
        hv: Arc<Hypervisor>,
        driver_status: DriverStatus,
        result_sender: &SyncSender<()>,
    ) -> Result<InterrutableThread<(), ()>> {
        info!("spawn stage1 in vm at ip {:#x}", self.regs.ip());
        try_with!(
            hv.set_regs(&hv.vcpus[0], &self.regs),
            "failed to set cpu registers"
        );

        let res = InterrutableThread::spawn(
            "stage1",
            result_sender,
            move |_ctx: &(), should_stop: Arc<AtomicBool>| {
                // wait until vmsh can process block device requests
                stage1_thread(driver_status, &hv, should_stop)
            },
            (),
        );
        Ok(try_with!(res, "failed to create stage1 thread"))
    }
}

fn stage1_thread(
    driver_status: DriverStatus,
    hv: &Hypervisor,
    should_stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut initialized = false;
    loop {
        match try_with!(driver_status.check(hv), "cannot check driver state") {
            DeviceState::Initializing => {
                if !initialized {
                    info!("stage1 driver initializing...");
                }
                initialized = true;
            }
            DeviceState::Undefined => {}
            DeviceState::Terminating => {
                bail!("guest driver is in unexpecting terminating state");
            }
            DeviceState::Error => {
                bail!("guest driver failed with error");
            }
            DeviceState::Ready => break,
        };
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    info!("stage1 driver started");
    Ok(())
}
