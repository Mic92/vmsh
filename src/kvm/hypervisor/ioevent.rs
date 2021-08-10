use simple_error::try_with;
use std::os::unix::io::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::sync::Arc;
use vmm_sys_util::eventfd::EventFd;

use super::ioeventfd::IoEventFd;
use super::userspaceioeventfd::UserspaceIoEventFd;
use super::Hypervisor;
use crate::devices::virtio::{register_ioeventfd, MmioConfig};
use crate::devices::USE_IOREGIONFD;
use crate::result::Result;
use std::ops::Deref;

/// Abstraction around IoEventFd and EventFd.
/// They can't be placed into the same memory location without an enum, because
/// vmm_sys_util::EventFd doesn't implement Sized which is required for `dyn`.
pub enum IoEvent {
    IoEventFd(IoEventFd),
    EventFd(EventFd),
}

impl IoEvent {
    pub fn register(
        vmm: &Arc<Hypervisor>,
        uioefd: &mut UserspaceIoEventFd,
        mmio_cfg: &MmioConfig,
        queue_idx: u64,
    ) -> Result<IoEvent> {
        if USE_IOREGIONFD {
            let eventfd = try_with!(
                uioefd.userpace_ioeventfd(Some(queue_idx as u32)),
                "cannot register userspace ioeventfd"
            );
            Ok(IoEvent::EventFd(eventfd))
        } else {
            let ioeventfd = try_with!(
                register_ioeventfd(vmm, mmio_cfg, queue_idx),
                "cannot register ioeventfd"
            );
            Ok(IoEvent::IoEventFd(ioeventfd))
        }
    }
}

impl AsRawFd for IoEvent {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            IoEvent::IoEventFd(e) => e.as_raw_fd(),
            IoEvent::EventFd(e) => e.as_raw_fd(),
        }
    }
}

impl Deref for IoEvent {
    type Target = EventFd;
    fn deref(&self) -> &EventFd {
        match self {
            IoEvent::IoEventFd(e) => e,
            IoEvent::EventFd(e) => e,
        }
    }
}
