use lazy_static::lazy_static;
use log::{error, info};
use nix::sys::signal;
use nix::unistd::Pid;
use simple_error::try_with;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};

use crate::device::create_block_device;
use crate::kvm;
use crate::result::Result;
use crate::stage1::spawn_stage1;

pub struct AttachOptions {
    pub pid: Pid,
    pub ssh_args: Vec<String>,
    pub backing: PathBuf,
}

lazy_static! {
    static ref SIGNAL_SENDER: Mutex<Option<SyncSender<()>>> = Mutex::new(None);
}

extern "C" fn signal_handler(_: ::libc::c_int) {
    let sender = match SIGNAL_SENDER.lock().expect("cannot lock sender").take() {
        Some(s) => {
            info!("shutdown vmsh");
            s
        }
        None => {
            info!("received sigterm. stopping already in progress");
            return;
        }
    };
    if let Err(e) = sender.send(()) {
        error!("cannot notify main process: {}", e);
    }
}

fn setup_signal_handler(sender: &SyncSender<()>) -> Result<()> {
    try_with!(SIGNAL_SENDER.lock(), "cannot get lock").replace(sender.clone());

    let sig_action = signal::SigAction::new(
        signal::SigHandler::Handler(signal_handler),
        signal::SaFlags::empty(),
        signal::SigSet::empty(),
    );

    unsafe {
        try_with!(
            signal::sigaction(signal::SIGINT, &sig_action),
            "unable to register SIGINT handler"
        );
        try_with!(
            signal::sigaction(signal::SIGTERM, &sig_action),
            "unable to register SIGTERM handler"
        );
    }
    Ok(())
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

    setup_signal_handler(&sender)?;

    let stage1 = try_with!(spawn_stage1(&opts.ssh_args, &sender), "stage1 failed");

    let threads = try_with!(
        create_block_device(&vm, &sender, &opts.backing),
        "cannot create block device"
    );

    info!("blkdev queue ready.");
    drop(sender);

    let _ = dbg!(receiver.recv());
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

    vm.resume()?;

    Ok(())
}
