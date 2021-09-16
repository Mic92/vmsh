use log::{error, info};
use nix::unistd::Pid;
use simple_error::{require_with, try_with};
use std::fs::read_to_string;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::time::Duration;

use crate::devices::use_ioregionfd;
use crate::devices::DeviceSet;
use crate::result::Result;
use crate::stage1::Stage1;
use crate::{kvm, signal_handler};

pub struct AttachOptions {
    pub pid: Pid,
    pub command: Vec<String>,
    pub backing: PathBuf,
    pub pts: Option<PathBuf>,
}

pub fn get_irq_num(pid: Pid) -> Result<usize> {
    let mut comm_path = PathBuf::from("/proc");
    comm_path.push(pid.as_raw().to_string());
    comm_path.push("comm");
    let comm = try_with!(
        read_to_string(&comm_path),
        "failed to read {}",
        comm_path.display()
    );
    // dirty hack until we have a better way to find out what IRQs we can use
    if comm.contains("crosvm") {
        Ok(4)
    } else {
        Ok(6)
    }
}

pub fn attach(opts: &AttachOptions) -> Result<()> {
    info!("attaching");

    let (sender, receiver) = sync_channel(1);

    signal_handler::setup(&sender)?;

    let mut vm = try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    vm.stop()?;
    try_with!(
        vm.setup_transfer_sockets(),
        "failed to setup unix sockets for fd transfer"
    );
    let vm = Arc::new(vm);

    let mut allocator = try_with!(
        kvm::PhysMemAllocator::new(Arc::clone(&vm)),
        "cannot create allocator"
    );

    let irq_num = try_with!(get_irq_num(opts.pid), "failed to get irq num");

    let devices = try_with!(
        DeviceSet::new(
            &vm,
            &mut allocator,
            irq_num,
            &opts.backing,
            opts.pts.clone()
        ),
        "cannot create devices"
    );

    if receiver.recv_timeout(Duration::from_millis(0)).is_ok() {
        return Ok(());
    }

    let addrs = devices.mmio_addrs()?;
    let mut stage1 = try_with!(
        Stage1::new(allocator, &opts.command, irq_num, addrs),
        "failed to initialize stage1"
    );
    let driver_status = require_with!(stage1.driver_status.take(), "no driver status set");
    let stage1_thread = try_with!(
        stage1.spawn(Arc::clone(&vm), driver_status.clone(), &sender),
        "failed to spawn stage1"
    );
    let device_status = require_with!(stage1.device_status.take(), "device status is not set");
    let (threads, driver_notifier) = try_with!(
        devices.start(&vm, device_status, driver_status, &sender),
        "failed to start devices"
    );

    info!("blkdev queue ready.");
    drop(sender);

    // termination wait or vmsh_stop()
    let _ = receiver.recv();
    stage1_thread.shutdown();
    if let Err(e) = stage1_thread.join() {
        error!("{}", e);
    };
    if let Err(e) = driver_notifier.terminate() {
        error!("failed to stop device: {}", e);
    }
    threads.iter().for_each(|t| t.shutdown());
    let contexts = threads
        .into_iter()
        .map(|t| {
            let (res, ctx) = match t.join() {
                Err(e) => (Err(e), None),
                Ok((res, ctx)) => (res, ctx),
            };
            if let Err(e) = res {
                error!("{}", e);
            }
            ctx
        })
        .collect::<Vec<_>>();

    // MMIO exit handler thread took over pthread control
    // We need ptrace the process again before we can finish.
    vm.stop()?;
    if !use_ioregionfd() {
        vm.finish_thread_transfer()?;
    }
    // now that we got the tracer back, we can cleanup physical memory and file descriptors
    drop(stage1);
    drop(contexts);
    try_with!(vm.close_transfer_sockets(), "cannot close transfer sockets");
    vm.resume()?;

    Ok(())
}
