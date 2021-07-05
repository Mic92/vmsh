use log::{error, info};
use nix::unistd::Pid;
use simple_error::try_with;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::sync::Arc;

use crate::devices::create_devices;
use crate::pty::pty_thread;
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

    let (sender, receiver) = sync_channel(1);

    signal_handler::setup(&sender)?;

    let pty_thread = try_with!(pty_thread(&sender), "cannot create pty forwarder");
    //let monitor_thread = try_with!(monitor_thread(&sender), "cannot create monitor forwarder");

    let stage1 = try_with!(
        spawn_stage1(opts.ssh_args.as_str(), &opts.command, &sender),
        "stage1 failed"
    );

    let mut threads = try_with!(
        create_devices(&vm, &sender, &opts.backing),
        "cannot create devices"
    );

    threads.push(pty_thread);
    //threads.push(monitor_thread);

    info!("blkdev queue ready.");
    drop(sender);

    // termination wait or vmsh_stop()
    let _ = receiver.recv();
    stage1.shutdown();
    if let Err(e) = stage1.join() {
        error!("stage1 failed: {}", e);
    }
    threads.iter().for_each(|t| t.shutdown());
    for t in threads {
        if let Err(e) = t.join() {
            error!("{}", e);
        }
    }

    // MMIO exit handler thread took over pthread control
    // We need ptrace the process again before we can finish.
    vm.finish_thread_transfer()?;
    vm.resume()?;

    Ok(())
}
