use simple_error::bail;
use std::fmt::Debug;
use std::io;
use std::ops::FnOnce;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread::Builder;
use std::thread::JoinHandle;

use crate::result::Result;

/// We don't need deep stacks for our threads so let's safe a bit memory by having
pub const DEFAULT_THREAD_STACKSIZE: usize = 128 * 1024;

pub struct InterrutableThread<T>
where
    T: Debug + Send + 'static,
{
    handle: JoinHandle<Result<T>>,
    should_stop: Arc<AtomicBool>,
}

impl<T> InterrutableThread<T>
where
    T: Debug + Send + 'static,
{
    /// Creates and runs a threads with the given name.
    /// The thread function will receive an atomic boolean as its first argument
    /// and should stop it's work once it becomes true.
    pub fn spawn<F>(name: &str, err_sender: &SyncSender<()>, func: F) -> io::Result<Self>
    where
        F: FnOnce(Arc<AtomicBool>) -> Result<T>,
        F: Send + 'static,
    {
        let builder = Builder::new()
            .name(String::from(name))
            .stack_size(DEFAULT_THREAD_STACKSIZE);
        let should_stop = Arc::new(AtomicBool::new(false));
        let should_stop2 = Arc::clone(&should_stop);
        let err_sender = err_sender.clone();

        let handle = builder.spawn(move || {
            let res = func(should_stop2);
            if res.is_err() {
                dbg!(&res);
                err_sender
                    .send(())
                    .expect("Could not send result back. Parent died");
            }
            res
        })?;

        Ok(Self {
            handle,
            should_stop,
        })
    }

    /// To be called before join() to stop the underlying thread
    pub fn shutdown(&self) {
        self.should_stop.store(true, Ordering::Release);
    }

    /// Join the underlying thread
    pub fn join(self) -> Result<T> {
        assert!(
            self.should_stop.load(Ordering::Acquire),
            "shutdown() needs to be called before join()"
        );
        match self.handle.join() {
            Err(e) => bail!("could not join thread: {:?}", e),
            Ok(v) => v,
        }
    }
}
