use simple_error::{bail, try_with};
use std::os::unix::io::AsRawFd;
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};

use crate::result::Result;

struct UIoEFd {
    datamatch: Option<u32>,
    fd: EventFd,
}

/// For 32bit wide accesses on the queue notify register of virtio devices. Requires one UIoEventFd
/// per device.
#[derive(Default)]
pub struct UserspaceIoEventFd {
    ioeventfds: Vec<UIoEFd>,
}

impl UserspaceIoEventFd {
    pub fn userpace_ioeventfd(&mut self, datamatch: Option<u32>) -> Result<EventFd> {
        if datamatch.is_none() && !self.ioeventfds.is_empty() {
            bail!("cannot add a userspace ioeventfd without datamatch when others with datamatch have already been registered");
        }

        let fd = try_with!(
            EventFd::new(EFD_NONBLOCK),
            "cannot create non-blocking eventfd for uioefd"
        );
        log::info!("eventfd {:?} for ioregionfd", fd.as_raw_fd(),);
        let uioefd = UIoEFd {
            datamatch,
            fd: try_with!(fd.try_clone(), "cannot clone uioefd"),
        };
        self.ioeventfds.push(uioefd);
        Ok(fd)
    }

    /// Callback for writes to QueueNotify register. Forwards notification to the corresponding
    /// EventFd reader.
    pub fn queue_notify(&self, val: u32) {
        let criterion = |uioefd: &&UIoEFd| match uioefd.datamatch {
            Some(dm) => dm == val,
            None => true,
        };
        if let Some(ioefd) = self.ioeventfds.iter().find(criterion) {
            // ioefd is the correct fd from our list
            if let Err(e) = ioefd.fd.write(1) {
                log::trace!(
                    "cannot write to UserspaceIoEventFd (datamatch {:?}, fd {}): {}",
                    ioefd.datamatch,
                    ioefd.fd.as_raw_fd(),
                    e
                );
            }
        } else {
            log::trace!("cannot find datamatch for ");
        }
    }
}
