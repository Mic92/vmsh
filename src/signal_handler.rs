use std::sync::{mpsc::SyncSender, Mutex};

use lazy_static::lazy_static;
use log::{error, info};
use nix::sys::signal;
use simple_error::try_with;

use crate::result::Result;

lazy_static! {
    static ref SIGNAL_SENDER: Mutex<Option<SyncSender<()>>> = Mutex::new(None);
}

fn _stop_vmsh(is_signal: bool) {
    let sender = match SIGNAL_SENDER.lock().expect("cannot lock sender").take() {
        Some(s) => {
            info!("shutdown vmsh");
            s
        }
        None => {
            if is_signal {
                info!("received sigterm. stopping already in progress");
            }
            return;
        }
    };
    if let Err(e) = sender.send(()) {
        error!("cannot notify main process: {}", e);
    }
}

pub fn stop_vmsh() {
    _stop_vmsh(false);
}

extern "C" fn signal_handler(_: ::libc::c_int) {
    _stop_vmsh(true);
}

pub fn setup(sender: SyncSender<()>) -> Result<()> {
    try_with!(SIGNAL_SENDER.lock(), "cannot get lock").replace(sender);

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
