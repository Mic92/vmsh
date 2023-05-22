use crate::page_table::PhysAddr;
use kvm_bindings as kvmb;
use libc::c_void;
use log::*;
use nix::unistd::Pid;
use simple_error::simple_error;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::{Arc, RwLock};
use vm_memory::remote_mem;

use crate::kvm::ioctls;
use crate::kvm::tracee::Tracee;
use crate::result::Result;

pub fn process_read<T: Sized + Copy>(pid: Pid, addr: *const c_void) -> Result<T> {
    remote_mem::process_read(pid, addr).map_err(|e| simple_error!("{}", e))
}

pub fn process_write<T: Sized + Copy>(pid: Pid, addr: *mut c_void, val: &T) -> Result<()> {
    remote_mem::process_write(pid, addr, val).map_err(|e| simple_error!("{}", e))
}

#[derive(Debug)]
pub struct SendPhantom<T> {
    phantom: PhantomData<T>,
}

impl<T> Default for SendPhantom<T> {
    fn default() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

unsafe impl<T> Send for SendPhantom<T> {}
unsafe impl<T> Sync for SendPhantom<T> {}

/// Hypervisor Memory
#[derive(Debug)]
pub struct HvMem<T: Copy> {
    pub ptr: libc::uintptr_t,
    pub(super) pid: Pid,
    pub(super) tracee: Arc<RwLock<Tracee>>,
    #[allow(dead_code)]
    pub(super) phantom: SendPhantom<T>,
}

impl<T: Copy> Drop for HvMem<T> {
    fn drop(&mut self) {
        // Useful for debugging
        //warn!("SKIP CLEANUP");
        //return;
        let tracee = match self.tracee.write() {
            Err(e) => {
                warn!("Could not aquire lock to drop HvMem: {}", e);
                return;
            }
            Ok(t) => t,
        };
        if let Err(e) = tracee.munmap(self.ptr as *mut c_void, size_of::<T>()) {
            warn!("failed to unmap memory from process: {}", e);
        }
    }
}

impl<T: Copy> HvMem<T> {
    pub fn read(&self) -> Result<T> {
        process_read(self.pid, self.ptr as *mut c_void)
    }
    pub fn write(&self, val: &T) -> Result<()> {
        process_write(self.pid, self.ptr as *mut c_void, val)
    }
}

/// Physical Memory attached to a VM. Backed by `PhysMem.mem`.
#[derive(Debug)]
pub struct PhysMem<T: Copy> {
    pub mem: HvMem<T>,
    pub(super) ioctl_arg: HvMem<kvmb::kvm_userspace_memory_region>,
    pub guest_phys_addr: PhysAddr,
}

impl<T: Copy> Drop for PhysMem<T> {
    fn drop(&mut self) {
        // useful for debugging
        //warn!("SKIP CLEANUP");
        //return;

        let tracee = match self.mem.tracee.write() {
            Err(e) => {
                warn!("Could not aquire lock to drop HvMem: {}", e);
                return;
            }
            Ok(t) => t,
        };
        let mut ioctl_arg = match self.ioctl_arg.read() {
            Err(e) => {
                warn!("Could not read Hypervisor Memory to drop HvMem: {}", e);
                return;
            }
            Ok(t) => t,
        };
        ioctl_arg.memory_size = 0; // indicates request for deletion
        match self.ioctl_arg.write(&ioctl_arg) {
            Err(e) => {
                warn!("Could not write to Hypervisor Memory to drop HvMem: {}", e);
                return;
            }
            Ok(t) => t,
        };
        let ret =
            match tracee.vm_ioctl_with_ref(ioctls::KVM_SET_USER_MEMORY_REGION(), &self.ioctl_arg) {
                Ok(ret) => ret,
                Err(e) => {
                    warn!("failed to remove memory from VM: {}", e);
                    return;
                }
            };
        if ret != 0 {
            warn!(
                "ioctl_with_ref to remove memory from VM returned error code: {}",
                ret
            )
        }
    }
}
