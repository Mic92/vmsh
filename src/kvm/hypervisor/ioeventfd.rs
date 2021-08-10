use kvm_bindings as kvmb;
use log::*;
use simple_error::{bail, try_with};
use std::os::unix::io::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::sync::{Arc, RwLock};
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};

use super::memory::HvMem;
use super::Hypervisor;
use crate::kvm::ioctls;
use crate::kvm::tracee::Tracee;
use crate::result::Result;
use std::ops::Deref;

pub struct IoEventFd {
    fd: EventFd,
    hv_eventfd: RawFd,
    hv_mem: HvMem<kvmb::kvm_ioeventfd>,
    tracee: Arc<RwLock<Tracee>>,
    guest_addr: u64,
    len: u32,
    datamatch: Option<u64>,
}

fn kvm_ioeventfd(
    hv_eventfd: RawFd,
    guest_addr: u64,
    len: u32,
    datamatch: Option<u64>,
) -> kvmb::kvm_ioeventfd {
    let mut flags = 0;
    let mut datam = 0;
    if let Some(data) = datamatch {
        flags = 1 << kvmb::kvm_ioeventfd_flag_nr_datamatch;
        datam = data;
    }

    kvmb::kvm_ioeventfd {
        len,
        datamatch: datam,
        addr: guest_addr,
        fd: hv_eventfd.as_raw_fd(),
        flags,
        ..Default::default()
    }
}
impl IoEventFd {
    pub fn new(
        hv: &Hypervisor,
        guest_addr: u64,
        len: u32,
        datamatch: Option<u64>,
    ) -> Result<IoEventFd> {
        let eventfd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd");
        info!(
            "ioeventfd {}, guest phys addr {:#x}",
            eventfd.as_raw_fd(),
            guest_addr
        );
        let hv_eventfd = hv.transfer(vec![eventfd.as_raw_fd()].as_slice())?[0];
        let ioeventfd = kvm_ioeventfd(hv_eventfd, guest_addr, len, datamatch);

        let mem = hv.alloc_mem()?;
        mem.write(&ioeventfd)?;

        let ret = {
            let tracee = try_with!(
                hv.tracee.read(),
                "cannot obtain tracee read lock: poinsoned"
            );
            try_with!(
                tracee.vm_ioctl_with_ref(ioctls::KVM_IOEVENTFD(), &mem),
                "kvm ioeventfd ioctl injection failed"
            )
        };
        if ret != 0 {
            bail!("cannot register KVM_IOEVENTFD via ioctl: {:?}", ret);
        }

        Ok(IoEventFd {
            guest_addr,
            len,
            datamatch,
            hv_eventfd,
            hv_mem: mem,
            fd: eventfd,
            tracee: hv.tracee.clone(),
        })
    }
}

impl Drop for IoEventFd {
    fn drop(&mut self) {
        let tracee = match self.tracee.read() {
            Err(e) => {
                warn!("IoEventfd: Could not aquire lock: {}", e);
                return;
            }
            Ok(t) => t,
        };
        let mut ioeventfd =
            kvm_ioeventfd(self.hv_eventfd, self.guest_addr, self.len, self.datamatch);
        ioeventfd.flags |= 1 << kvmb::kvm_ioeventfd_flag_nr_deassign;

        if let Err(e) = self.hv_mem.write(&ioeventfd) {
            warn!(
                "IoEventFd: Could not write to HvMem while dropping IoEventFd: {}",
                e
            );
            return;
        }

        if let Err(e) = tracee.vm_ioctl_with_ref(ioctls::KVM_IOEVENTFD(), &self.hv_mem) {
            warn!("IoEventfd: kvm ioeventfd ioctl injection failed: {}", e)
        }

        if let Err(e) = tracee.close(self.hv_eventfd) {
            warn!("IoEventfd: failed to close eventfd in hypervisor: {}", e)
        }
    }
}

impl Deref for IoEventFd {
    type Target = EventFd;
    fn deref(&self) -> &EventFd {
        &self.fd
    }
}

impl AsRawFd for IoEventFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}
