use log::{error, info};
use nix::unistd::Pid;
use simple_error::try_with;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::sync::Arc;

use crate::devices::DeviceSet;
use crate::result::Result;
use crate::stage1::spawn_stage1;
use crate::{kvm, signal_handler};

pub struct AttachOptions {
    pub pid: Pid,
    pub ssh_args: String,
    pub command: Vec<String>,
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

    let mut allocator = try_with!(
        kvm::PhysMemAllocator::new(Arc::clone(&vm)),
        "cannot create allocator"
    );

    let (sender, receiver) = sync_channel(1);

    signal_handler::setup(&sender)?;

    let devices = try_with!(
        DeviceSet::new(&vm, &mut allocator, &opts.backing),
        "cannot create devices"
    );

    let stage1 = try_with!(
        spawn_stage1(
            opts.ssh_args.as_str(),
            &opts.command,
            devices.mmio_addrs()?,
            allocator,
            &sender
        ),
        "stage1 failed"
    );
    let threads = try_with!(devices.start(&vm, &sender), "failed to start devices");

    info!("blkdev queue ready.");
    drop(sender);

    // termination wait or vmsh_stop()
    let _ = receiver.recv();
    stage1.shutdown();
    let virt_memory = match stage1.join() {
        Ok(mut v) => v.virt_mem.take(),
        Err(e) => {
            error!("stage1 failed: {}", e);
            None
        }
    };
    threads.iter().for_each(|t| t.shutdown());
    for t in threads {
        if let Err(e) = t.join() {
            error!("{}", e);
        }
    }

    // MMIO exit handler thread took over pthread control
    // We need ptrace the process again before we can finish.
    vm.finish_thread_transfer()?;
    // now that we got the tracer back, we can cleanup pysical memory
    drop(virt_memory);
    vm.resume()?;

    Ok(())
}
