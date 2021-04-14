use kvm_bindings as kvmb;
use libc::{c_int, c_void};
use log::warn;
use nix::sys::uio::{process_vm_readv, process_vm_writev, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::ffi::OsStr;
use std::marker::PhantomData;
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::os::unix::io::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::sync::{Arc, RwLock};
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};

use crate::cpu;
use crate::kvm::fd_transfer;
use crate::kvm::ioctls;
use crate::kvm::tracee::{kvm_msrs, Tracee};
use crate::page_math;
use crate::proc::{openpid, Mapping, PidHandle};
use crate::result::Result;

/// # Safety
///
/// None. See safety chapter of `std::slice::from_raw_parts`.
pub unsafe fn any_as_bytes<T: Sized>(p: &T) -> &[u8] {
    std::slice::from_raw_parts((p as *const T) as *const u8, size_of::<T>())
}

/// read from a virtual addr of the hypervisor
pub fn process_read<T: Sized + Copy>(pid: Pid, addr: *const c_void) -> Result<T> {
    let len = size_of::<T>();
    let mut t_mem = MaybeUninit::<T>::uninit();
    let t_slice = unsafe { std::slice::from_raw_parts_mut(t_mem.as_mut_ptr() as *mut u8, len) };

    let local_iovec = vec![IoVec::from_mut_slice(t_slice)];
    let remote_iovec = vec![RemoteIoVec {
        base: addr as usize,
        len,
    }];

    let f = try_with!(
        process_vm_readv(pid, local_iovec.as_slice(), remote_iovec.as_slice()),
        "cannot read memory"
    );
    if f != len {
        bail!(
            "process_vm_readv read {} bytes when {} were expected",
            f,
            len
        )
    }

    let t: T = unsafe { t_mem.assume_init() };
    Ok(t)
}

/// write to a virtual addr of the hypervisor
pub fn process_write<T: Sized + Copy>(pid: Pid, addr: *mut c_void, val: &T) -> Result<()> {
    let len = size_of::<T>();
    // safe, because we won't need t_bytes for long
    let t_bytes = unsafe { any_as_bytes(val) };

    let local_iovec = vec![IoVec::from_slice(t_bytes)];
    let remote_iovec = vec![RemoteIoVec {
        base: addr as usize,
        len,
    }];

    let f = try_with!(
        process_vm_writev(pid, local_iovec.as_slice(), remote_iovec.as_slice()),
        "cannot write memory"
    );
    if f != len {
        bail!(
            "process_vm_writev written {} bytes when {} were expected",
            f,
            len
        )
    }

    Ok(())
}

/// Hypervisor Memory
pub struct HvMem<T: Copy> {
    pub ptr: *mut c_void,
    pid: Pid,
    tracee: Arc<RwLock<Tracee>>,
    phantom: PhantomData<T>,
}

impl<T: Copy> Drop for HvMem<T> {
    fn drop(&mut self) {
        let tracee = match self.tracee.write() {
            Err(e) => {
                warn!("Could not aquire lock to drop HvMem: {}", e);
                return;
            }
            Ok(t) => t,
        };
        if let Err(e) = tracee.munmap(self.ptr, size_of::<T>()) {
            warn!("failed to unmap memory from process: {}", e);
        }
    }
}

impl<T: Copy> HvMem<T> {
    pub fn read(&self) -> Result<T> {
        process_read(self.pid, self.ptr)
    }
    pub fn write(&self, val: &T) -> Result<()> {
        process_write(self.pid, self.ptr, val)
    }
}

/// Physical Memory attached to a VM. Backed by `VmMem.mem`.
pub struct VmMem<T: Copy> {
    pub mem: HvMem<T>,
    ioctl_arg: HvMem<kvmb::kvm_userspace_memory_region>,
}

impl<T: Copy> Drop for VmMem<T> {
    fn drop(&mut self) {
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

#[allow(upper_case_acronyms)]
pub struct VCPU {
    pub idx: usize,
    pub fd_num: RawFd,
}

/// Owns the tracee to prevent that multiple tracees are created for a Hypervisor. The Hypervisor
/// is used to handle the lock on `Self.tracee` and is used to instantiate `HvMem` and `VmMem`.
pub struct Hypervisor {
    pub pid: Pid,
    pub vm_fd: RawFd,
    pub vcpus: Vec<VCPU>,
    tracee: Arc<RwLock<Tracee>>,
}

impl Hypervisor {
    fn attach(pid: Pid, vm_fd: RawFd) -> Tracee {
        Tracee::new(pid, vm_fd, None)
    }

    pub fn resume(&self) -> Result<()> {
        let mut tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.detach();
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let mut tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.attach()?;
        Ok(())
    }

    pub fn get_maps(&self) -> Result<Vec<Mapping>> {
        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee read lock: poinsoned"
        );
        tracee.get_maps()
    }

    /// `readonly`: If true, a guest writing to it leads to KVM_EXIT_MMIO.
    ///
    /// Safety: This function is safe even for the guest because VmMem enforces, that only the
    /// allocated T is written to.
    pub fn vm_add_mem<T: Sized + Copy>(&self, guest_addr: u64, readonly: bool) -> Result<VmMem<T>> {
        // must be a multiple of PAGESIZE
        let slot_len = (size_of::<T>() / page_math::page_size() + 1) * page_math::page_size();
        let hv_memslot = self.alloc_mem_padded::<T>(slot_len)?;
        let mut flags = 0;
        flags |= if readonly { kvmb::KVM_MEM_READONLY } else { 0 };
        let arg = kvmb::kvm_userspace_memory_region {
            slot: self.get_maps()?.len() as u32, // guess a hopfully available slot id
            flags,
            guest_phys_addr: guest_addr, // must be page aligned
            memory_size: slot_len as u64,
            userspace_addr: hv_memslot.ptr as u64,
        };
        let arg_hv = self.alloc_mem()?;
        arg_hv.write(&arg)?;

        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee write lock: poinsoned"
        );
        let ret = tracee.vm_ioctl_with_ref(ioctls::KVM_SET_USER_MEMORY_REGION(), &arg_hv)?;
        if ret != 0 {
            bail!("ioctl_with_ref failed: {}", ret)
        }

        Ok(VmMem {
            mem: hv_memslot,
            ioctl_arg: arg_hv,
        })
    }

    pub fn alloc_mem<T: Copy>(&self) -> Result<HvMem<T>> {
        self.alloc_mem_padded::<T>(size_of::<T>())
    }

    /// allocate memory for T. Allocate more than necessary to increase allocation size to `size`.
    pub fn alloc_mem_padded<T: Copy>(&self, size: usize) -> Result<HvMem<T>> {
        if size < size_of::<T>() {
            bail!(
                "allocating {}b for item of size {} is not sufficient",
                size,
                size_of::<T>()
            )
        }
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        // safe, event for the tracee, because HvMem enforces to write and read at mose
        // `size_of::<T> <= size` bytes.
        let ptr = tracee.mmap(size)?;
        Ok(HvMem {
            ptr,
            pid: self.pid,
            tracee: self.tracee.clone(),
            phantom: PhantomData,
        })
    }

    pub fn transfer(&self, fds: &[RawFd]) -> Result<Vec<RawFd>> {
        let addr_local_mem = self.alloc_mem()?;
        let addr_remote_mem = self.alloc_mem()?;
        let msg_hdr_mem = self.alloc_mem()?;
        let iov_mem = self.alloc_mem()?;
        let iov_buf_mem = self.alloc_mem::<[u8; 1]>()?;
        let cmsg_mem = self.alloc_mem::<[u8; 64]>()?; // should be of size CMSG_SPACE, but thats not possible at compile time

        let ret;
        let hv;
        {
            let tracee = try_with!(
                self.tracee.write(),
                "cannot obtain tracee write lock: poinsoned"
            );

            let vmsh_id = format!("vmsh_fd_transfer_{}", nix::unistd::getpid());
            let hypervisor_id = format!("vmsh_fd_transfer_{}", self.pid);
            let vmsh = fd_transfer::Socket::new(&vmsh_id)?;
            let proc = tracee.try_get_proc()?;
            hv = fd_transfer::HvSocket::new(
                self.tracee.clone(),
                proc,
                &hypervisor_id,
                &addr_local_mem,
            )?;

            vmsh.connect(&hypervisor_id)?;
            hv.connect(proc, &vmsh_id, &addr_remote_mem)?;

            let message = [1u8; 1];
            let m_slice = &message[0..1];
            let mut messages = Vec::with_capacity(fds.len());
            fds.iter().for_each(|_| messages.push(m_slice));

            vmsh.send(messages.as_slice(), fds)?;
            let (msg, fds) = hv.receive(proc, &msg_hdr_mem, &iov_mem, &iov_buf_mem, &cmsg_mem)?;
            if msg != message {
                bail!("received message differs from sent one");
            }
            ret = fds;
        } // drop tracee write lock so that `hv: HvSock can obtain lock to drop itself

        Ok(ret)
    }

    pub fn ioeventfd(&self, guest_addr: u64) -> Result<EventFd> {
        let eventfd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd");
        println!(
            "ioeventfd {:?}, guest phys addr {:?}",
            eventfd.as_raw_fd(),
            guest_addr
        );
        let hv_eventfd = self.transfer(vec![eventfd.as_raw_fd()].as_slice())?[0];

        let ioeventfd = kvmb::kvm_ioeventfd {
            datamatch: 0,
            len: 0,
            addr: guest_addr,
            fd: hv_eventfd.as_raw_fd(), // thats why we get -22 EINVAL
            flags: 0,
            ..Default::default()
        };
        let mem = self.alloc_mem()?;
        mem.write(&ioeventfd)?;
        let ret = {
            let tracee = try_with!(
                self.tracee.read(),
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

        Ok(eventfd)
    }

    /// param `gsi`: pin on the irqchip to be toggled by fd events
    pub fn irqfd(&self, gsi: u32) -> Result<EventFd> {
        let eventfd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd");
        println!("irqfd {:?}, interupt gsi/nr {:?}", eventfd.as_raw_fd(), gsi);
        let hv_eventfd = self.transfer(vec![eventfd.as_raw_fd()].as_slice())?[0];

        let irqfd = kvmb::kvm_irqfd {
            fd: hv_eventfd.as_raw_fd() as u32,
            gsi,
            flags: 0,
            resamplefd: 0,
            ..Default::default()
        };
        let mem = self.alloc_mem()?;
        mem.write(&irqfd)?;
        let ret = {
            let tracee = try_with!(
                self.tracee.read(),
                "cannot obtain tracee read lock: poinsoned"
            );
            try_with!(
                tracee.vm_ioctl_with_ref(ioctls::KVM_IRQFD(), &mem),
                "kvm irqfd ioctl injection failed"
            )
        };
        if ret != 0 {
            bail!("cannot register KVM_IRQFD via ioctl: {:?}", ret);
        }

        Ok(eventfd)
    }

    pub fn check_extension(&self, cap: c_int) -> Result<c_int> {
        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee read lock: poinsoned"
        );
        tracee.check_extension(cap)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_sregs(&self, vcpu: &VCPU) -> Result<kvmb::kvm_sregs> {
        let mem = self.alloc_mem()?;
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.get_sregs(vcpu, &mem)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_regs(&self, vcpu: &VCPU) -> Result<cpu::Regs> {
        let mem = self.alloc_mem()?;
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.get_regs(vcpu, &mem)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_fpu_regs(&self, vcpu: &VCPU) -> Result<cpu::FpuRegs> {
        let mem = self.alloc_mem()?;
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.get_fpu_regs(vcpu, &mem)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_msr(&self, vcpu: &VCPU, msr: &kvmb::kvm_msr_entry) -> Result<kvmb::kvm_msr_entry> {
        let mem = self.alloc_mem()?;
        try_with!(
            mem.write(&kvm_msrs {
                nmsrs: 1,
                pad: 0,
                entries: [*msr; 1],
            }),
            "cannot obtain tracee write lock: poinsoned"
        );
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.get_msr(vcpu, &mem)
    }
}

fn find_vm_fd(handle: &PidHandle) -> Result<(Vec<RawFd>, Vec<VCPU>)> {
    let mut vm_fds: Vec<RawFd> = vec![];
    let mut vcpu_fds: Vec<VCPU> = vec![];
    let fds = try_with!(
        handle.fds(),
        "cannot lookup file descriptors of process {}",
        handle.pid
    );

    for fd in fds {
        let name = fd
            .path
            .file_name()
            .unwrap_or_else(|| OsStr::new(""))
            .to_str()
            .unwrap_or("");
        if name == "anon_inode:kvm-vm" {
            vm_fds.push(fd.fd_num)
        // i.e. anon_inode:kvm-vcpu:0
        } else if name.starts_with("anon_inode:kvm-vcpu:") {
            let parts = name.rsplitn(2, ':').collect::<Vec<_>>();
            assert!(parts.len() == 2);
            let idx = try_with!(
                parts[0].parse::<usize>(),
                "cannot parse number {}",
                parts[0]
            );
            vcpu_fds.push(VCPU {
                idx,
                fd_num: fd.fd_num,
            })
        }
    }
    let old_len = vcpu_fds.len();
    vcpu_fds.dedup_by_key(|vcpu| vcpu.idx);
    if old_len != vcpu_fds.len() {
        bail!("found multiple vcpus with same id, assume multiple VMs in same hypervisor. This is not supported yet")
    };

    Ok((vm_fds, vcpu_fds))
}

pub fn get_hypervisor(pid: Pid) -> Result<Hypervisor> {
    let handle = try_with!(openpid(pid), "cannot open handle in proc");

    let (vm_fds, vcpus) = try_with!(find_vm_fd(&handle), "failed to access kvm fds");
    if vm_fds.is_empty() {
        bail!("no VMs found");
    }
    if vm_fds.len() > 1 {
        bail!("multiple VMs found, this is not supported yet.");
    }
    if vcpus.is_empty() {
        bail!("found KVM instance but no VCPUs");
    }

    Ok(Hypervisor {
        pid,
        tracee: Arc::new(RwLock::new(Hypervisor::attach(pid, vm_fds[0]))),
        vm_fd: vm_fds[0],
        vcpus,
    })
}
