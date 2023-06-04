use log::{error, info};
use std::sync::mpsc::Sender;

use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub fn setup(sender: Sender<()>) {
    let _ = std::thread::spawn(move || {
        let mut signals = match Signals::new([SIGTERM, SIGINT]) {
            Ok(v) => v,
            Err(e) => {
                error!("error setting up signal handler: {:?}", e);
                return;
            }
        };
        loop {
            for _ in signals.pending() {
                info!("stopping vmsh...");
                if let Err(err) = sender.send(()) {
                    error!("error sending signal: {:?}", err);
                }
            }
        }
    });
}
