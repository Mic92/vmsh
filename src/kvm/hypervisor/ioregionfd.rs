use log::*;
use nix::poll::{ppoll, PollFd, PollFlags};
use nix::sys::signal::SigSet;
use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
use nix::sys::time::TimeSpec;
use nix::unistd::{close, read, write};
use simple_error::{bail, try_with};
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::os::unix::io::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::memory::HvMem;
use super::Hypervisor;
use crate::kvm::ioctls;
use crate::kvm::kvm_ioregionfd::kvm_ioregion;
use crate::kvm::kvm_ioregionfd::{self, ioregionfd_cmd, ioregionfd_resp};
use crate::kvm::tracee::Tracee;
use crate::result::Result;

/// Implements the KVM IoRegionFd feature.
pub struct IoRegionFd {
    tracee: Arc<RwLock<Tracee>>,
    hv_mem: HvMem<kvm_ioregion>,
    ioregion: kvm_ioregion,
    rfile: RawFd, // our end: we write responses here
    wfile: RawFd, // we read commands from here
    rf_hv: RawFd, // their end: will be transferred to hyperisor
    wf_hv: RawFd,
    hv_rf_hv: RawFd, // rf_hv, but in hypervisor process
    hv_wf_hv: RawFd,
}

impl IoRegionFd {
    pub fn new(hv: &Hypervisor, guest_paddr: u64, len: usize) -> Result<Self> {
        if Self::capability_present(hv)? {
            bail!("This operation requires KVM_CAP_IOREGIONFD which your KVM does not have.");
        }

        let (rf_dev, rf_hv) = try_with!(
            socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC
            ),
            "cannot create sockets for IoRegionFd"
        );
        let (wf_dev, wf_hv) = try_with!(
            socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC
            ),
            "cannot create sockets for IoRegionFd"
        );
        let hv_rf_hv = hv.transfer(vec![rf_hv.as_raw_fd()].as_slice())?[0];
        let hv_wf_hv = hv.transfer(vec![wf_hv.as_raw_fd()].as_slice())?[0];
        let ioregion = kvm_ioregion::new(guest_paddr, len, hv_rf_hv, hv_wf_hv);
        let mem = hv.alloc_mem()?;
        mem.write(&ioregion)?;

        let ret = {
            let tracee = try_with!(
                hv.tracee.read(),
                "cannot obtain tracee read lock: poinsoned"
            );
            try_with!(
                tracee.vm_ioctl_with_ref(ioctls::KVM_SET_IOREGION(), &mem),
                "kvm ioeventfd ioctl injection failed"
            )
        };
        if ret != 0 {
            bail!("ioregionfd ioctl failed with {}", ret);
        }

        Ok(IoRegionFd {
            tracee: hv.tracee.clone(),
            hv_mem: mem,
            ioregion,
            rfile: rf_dev,
            wfile: wf_dev,
            rf_hv,
            wf_hv,
            hv_rf_hv,
            hv_wf_hv,
        })
    }

    pub fn fdclone(&mut self) -> RawIoRegionFd {
        let pollfds = vec![PollFd::new(self.wfile, PollFlags::POLLIN)];
        RawIoRegionFd {
            rfile: self.rfile,
            wfile: self.wfile,
            ioregion: self.ioregion,
            pollfds,
        }
    }

    pub fn capability_present(hv: &Hypervisor) -> Result<bool> {
        let has_cap = try_with!(
            hv.check_extension(kvm_ioregionfd::KVM_CAP_IOREGIONFD as i32),
            "cannot check kvm extension capabilities"
        );
        Ok(has_cap == 0)
    }
}

impl Drop for IoRegionFd {
    fn drop(&mut self) {
        let tracee = match self.tracee.read() {
            Err(e) => {
                warn!("IoEventfd: Could not aquire lock: {}", e);
                return;
            }
            Ok(t) => t,
        };

        let mut ioregion = self.ioregion;
        ioregion.rfd = -1;
        ioregion.wfd = -1;

        if let Err(e) = self.hv_mem.write(&ioregion) {
            warn!(
                "IoRegionFd: Could not write to HvMem while dropping IoRegionFd: {}",
                e
            );
            return;
        }

        match tracee.vm_ioctl_with_ref(ioctls::KVM_SET_IOREGION(), &self.hv_mem) {
            Err(e) => warn!("IoRegionFd: kvm ioregionfd ioctl injection failed: {}", e),
            Ok(ret) => {
                if ret != 0 {
                    warn!("IoRegionFd: kvm ioregionfd remove syscall failed: {}", ret);
                }
            }
        }

        match tracee.close(self.hv_rf_hv) {
            Err(e) => warn!("IoRegionFd: close injection failed: {}", e),
            Ok(ret) => {
                if ret != 0 {
                    warn!(
                        "IoRegionFd: failed to close hv_rf_hv in hypervisor: {}",
                        ret
                    )
                }
            }
        }

        match tracee.close(self.hv_wf_hv) {
            Err(e) => warn!("IoRegionFd: close injection failed: {}", e),
            Ok(ret) => {
                if ret != 0 {
                    warn!(
                        "IoRegionFd: failed to close hv_wf_hv in hypervisor: {}",
                        ret
                    )
                }
            }
        }

        if let Err(e) = close(self.rf_hv) {
            warn!("IoRegionFd: failed to close rf_hv: {}", e)
        }

        if let Err(e) = close(self.wf_hv) {
            warn!("IoRegionFd: failed to close wf_hv: {}", e)
        }

        if let Err(e) = close(self.rfile) {
            warn!("IoRegionFd: failed to close rfile: {}", e)
        }

        if let Err(e) = close(self.wfile) {
            warn!("IoRegionFd: failed to close wfile: {}", e)
        }
    }
}

/// A handle implementing clone, read and write for the non-clonable IoRegionFd.
#[derive(Debug, Clone)]
pub struct RawIoRegionFd {
    rfile: RawFd, // our end: we write responses here
    wfile: RawFd, // we read commands from here
    pollfds: Vec<PollFd>,
    pub ioregion: kvm_ioregion,
}

impl RawIoRegionFd {
    /// receive read and write events/commands
    pub fn read(&mut self) -> Result<Option<ioregionfd_cmd>> {
        let len = size_of::<ioregionfd_cmd>();
        let mut t_mem = MaybeUninit::<ioregionfd_cmd>::uninit();
        // safe, because slice.len() == len
        let t_slice = unsafe { std::slice::from_raw_parts_mut(t_mem.as_mut_ptr() as *mut u8, len) };

        // read
        let timeout = TimeSpec::from(Duration::from_millis(300));
        let nr_events = try_with!(
            ppoll(&mut self.pollfds, Some(timeout), SigSet::empty()),
            "read/ppoll failed"
        );
        if nr_events == 0 || self.pollfds[0].revents().is_none() {
            return Ok(None);
        }
        let read = try_with!(
            read(self.wfile, t_slice),
            "read on ioregionfd {} failed",
            self.wfile
        );
        if read != len {
            bail!(
                "read returned ioregionfd command of incorrect length {}",
                read
            );
        }

        // safe, because we wrote exactly the correct amount of bytes (len)
        let cmd: ioregionfd_cmd = unsafe { t_mem.assume_init() };
        log::trace!(
            "read {:?}, {:?}, response={}: {:?}",
            cmd.info.cmd(),
            cmd.info.size(),
            cmd.info.is_response(),
            cmd
        );
        Ok(Some(cmd))
    }

    /// Write a response back to the VM.
    pub fn write_slice(&self, data: &[u8]) -> Result<()> {
        log::trace!("write_slice()");
        let mut arr = [0u8; 8];
        let arr_slice = &mut arr[0..data.len()];
        arr_slice.copy_from_slice(data);
        self.write(u64::from_ne_bytes(arr))
    }

    /// Write a response back to the VM.
    pub fn write(&self, data: u64) -> Result<()> {
        log::trace!("write {:x}", data);
        let len = size_of::<ioregionfd_resp>();
        let response = ioregionfd_resp::new(data);
        // safe, because we won't need t_bytes for longer than this stack frame
        let t_bytes = unsafe {
            std::slice::from_raw_parts((&response as *const ioregionfd_resp) as *const u8, len)
        };
        let written = try_with!(
            write(self.rfile, t_bytes),
            "write on ioregionfd {} failed",
            self.rfile
        );
        if written != len {
            bail!("cannot write entire ioregionfd command {}/{}", written, len);
        }
        Ok(())
    }
}
