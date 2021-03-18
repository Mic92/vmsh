use crate::cpu::FpuPointer;
use crate::cpu::Rip;
use kvm_bindings as kvmb;
use libc::{c_int, c_ulong, c_void};
use log::warn;
use nix::sys::uio::{process_vm_readv, process_vm_writev, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::ffi::OsStr;
use std::marker::PhantomData;
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::os::unix::prelude::RawFd;
use std::ptr;

pub mod ioctls;
mod memslots;

use crate::cpu;
use crate::inject_syscall;
use crate::kvm::ioctls::KVM_CHECK_EXTENSION;
use crate::kvm::memslots::get_maps;
use crate::page_math;
use crate::proc::{openpid, Mapping, PidHandle};
use crate::result::Result;

pub struct Tracee {
    pid: Pid,
    vm_fd: RawFd,
    proc: inject_syscall::Process, // TODO make optional and implement play/pause
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

/// Safe wrapper for unsafe inject_syscall::Process operations.
impl Tracee {
    fn vm_ioctl(&self, request: c_ulong, arg: c_ulong) -> Result<c_int> {
        self.proc.ioctl(self.vm_fd, request, arg)
    }

    fn vcpu_ioctl(&self, vcpu: &VCPU, request: c_ulong, arg: c_ulong) -> Result<c_int> {
        self.proc.ioctl(vcpu.fd_num, request, arg)
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
        // safe, because TraceeMem enforces to write and read at most `size_of::<T> <= size` bytes.
        let ptr = unsafe { self.mmap(size)? };
        Ok(HvMem {
            ptr,
            tracee: self,
            phantom: PhantomData,
        })
    }

    // comment borrowed from vmm-sys-util
    /// Run an [`ioctl`](http://man7.org/linux/man-pages/man2/ioctl.2.html)
    /// with an immutable reference.
    ///
    /// # Arguments
    ///
    /// * `req`: a device-dependent request code.
    /// * `arg`: an immutable reference passed to ioctl.
    ///
    /// # Safety
    ///
    /// The caller should ensure to pass a valid file descriptor and have the
    /// return value checked. Also he may take care to use the correct argument type belonging to
    /// the request type.
    pub fn vm_ioctl_with_ref<T: Sized + Copy>(&self, request: c_ulong, arg: &T) -> Result<c_int> {
        let mem = try_with!(self.alloc_mem::<T>(), "cannot allocate memory");

        try_with!(
            process_write(self.pid, mem.ptr, arg),
            "cannot write ioctl arg struct to hv"
        );

        let ioeventfd: kvmb::kvm_ioeventfd = try_with!(process_read(self.pid, mem.ptr), "foobar");
        println!(
            "arg {:?}, {:?}, {:?}",
            ioeventfd.len, ioeventfd.addr, ioeventfd.fd
        );

        println!("arg_ptr {:?}", mem.ptr);
        let ret = self.vm_ioctl(request, mem.ptr as c_ulong);

        ret
    }

    /// Safety: This function is safe for vmsh and the hypervisor. It is not for the guest.
    pub fn vm_add_mem<T: Sized + Copy>(&self) -> Result<VmMem<T>> {
        // must be a multiple of PAGESIZE
        let slot_len = (size_of::<T>() / page_math::page_size() + 1) * page_math::page_size();
        let hv_memslot = self.alloc_mem_padded::<T>(slot_len)?;
        let arg = kvmb::kvm_userspace_memory_region {
            slot: self.get_maps()?.len() as u32, // guess a hopfully available slot id
            flags: 0x00,                         // maybe KVM_MEM_READONLY
            guest_phys_addr: 0xd0000000,         // must be page aligned
            memory_size: slot_len as u64,
            userspace_addr: hv_memslot.ptr as u64,
        };

        let ret = self.vm_ioctl_with_ref(ioctls::KVM_SET_USER_MEMORY_REGION(), &arg)?;
        if ret != 0 {
            bail!("ioctl_with_ref failed: {}", ret)
        }

        Ok(VmMem {
            mem: hv_memslot,
            ioctl_arg: arg,
        })
    }

    /// Make the kernel allocate anonymous memory (anywhere he likes, not bound to a file
    /// descriptor). This is not fully POSIX compliant, but works on linux.
    ///
    /// length in bytes.
    /// returns void pointer to the allocated virtual memory address of the hypervisor.
    unsafe fn mmap(&self, length: libc::size_t) -> Result<*mut c_void> {
        let addr = libc::AT_NULL as *mut c_void; // make kernel choose location for us
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_SHARED | libc::MAP_ANONYMOUS;
        let fd = -1 as RawFd; // ignored because of MAP_ANONYMOUS => should be -1
        let offset = 0 as libc::off_t; // MAP_ANON => should be 0
        self.proc.mmap(addr, length, prot, flags, fd, offset)
    }

    /// Unmap memory in the process
    ///
    /// length in bytes.
    fn munmap(&self, addr: *mut c_void, length: libc::size_t) -> Result<()> {
        self.proc.munmap(addr, length)
    }

    pub fn check_extension(&self, cap: c_int) -> Result<c_int> {
        self.vm_ioctl(KVM_CHECK_EXTENSION(), cap as c_ulong)
    }

    pub fn pid(&self) -> Pid {
        self.pid
    }

    pub fn get_maps(&self) -> Result<Vec<Mapping>> {
        get_maps(self)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_fpu_regs(&self, vcpu: &VCPU) -> Result<cpu::FpuRegs> {
        use crate::kvm::ioctls::KVM_GET_FPU;
        let regs_mem = try_with!(self.alloc_mem::<kvmb::kvm_fpu>(), "cannot allocate memory");
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_FPU(), regs_mem.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let regs = try_with!(regs_mem.read(), "cannot read fpu registers");
        let st_space = unsafe { ptr::read(&regs.fpr as *const [u8; 16] as *const [u32; 32]) };
        let xmm_space =
            unsafe { ptr::read(&regs.xmm as *const [[u8; 16]; 16] as *const [u32; 64]) };

        Ok(cpu::FpuRegs {
            cwd: regs.fcw,
            swd: regs.fsw,
            twd: regs.ftwx as u16,
            fop: regs.last_opcode,
            p: FpuPointer {
                ip: Rip {
                    rip: regs.last_ip,
                    rdp: regs.last_dp,
                },
            },
            mxcsr: regs.mxcsr,
            mxcsr_mask: 0,
            st_space: st_space,
            xmm_space: xmm_space,
            padding: [0; 12],
            padding1: [0; 12],
        })
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_sregs(&self, vcpu: &VCPU) -> Result<kvmb::kvm_sregs> {
        use crate::kvm::ioctls::KVM_GET_SREGS;
        let sregs_mem = try_with!(
            self.alloc_mem::<kvmb::kvm_sregs>(),
            "cannot allocate memory"
        );
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_SREGS(), sregs_mem.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let sregs = try_with!(sregs_mem.read(), "cannot read registers");
        Ok(sregs)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_regs(&self, vcpu: &VCPU) -> Result<cpu::Regs> {
        use crate::kvm::ioctls::KVM_GET_REGS;
        let regs_mem = try_with!(self.alloc_mem::<kvmb::kvm_regs>(), "cannot allocate memory");
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_REGS(), regs_mem.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let regs = try_with!(regs_mem.read(), "cannot read registers");
        Ok(cpu::Regs {
            r15: regs.r15,
            r14: regs.r14,
            r13: regs.r13,
            r12: regs.r12,
            rbp: regs.rbp,
            rbx: regs.rbx,
            r11: regs.r11,
            r10: regs.r10,
            r9: regs.r9,
            r8: regs.r8,
            rax: regs.rax,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            orig_rax: regs.rax,
            rip: regs.rip,
            cs: 0,
            eflags: regs.rflags,
            rsp: regs.rsp,
            ss: 0,
            fs_base: 0,
            gs_base: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
        })
    }
}

pub unsafe fn any_as_bytes<T: Sized>(p: &T) -> &[u8] {
    std::slice::from_raw_parts((p as *const T) as *const u8, size_of::<T>())
}

/// Hypervisor Memory
pub struct HvMem<'a, T: Copy> {
    pub ptr: *mut c_void,
    tracee: &'a Tracee,
    phantom: PhantomData<T>,
}

impl<'a, T: Copy> Drop for HvMem<'a, T> {
    fn drop(&mut self) {
        if let Err(e) = self.tracee.munmap(self.ptr, size_of::<T>()) {
            warn!("failed to unmap memory from process: {}", e);
        }
    }
}

impl<'a, T: Copy> HvMem<'a, T> {
    pub fn read(&self) -> Result<T> {
        process_read(self.tracee.pid, self.ptr)
    }
    pub fn write(&self, val: &T) -> Result<()> {
        process_write(self.tracee.pid, self.ptr, val)
    }
}

pub struct VmMem<'a, T: Copy> {
    pub mem: HvMem<'a, T>,
    ioctl_arg: kvmb::kvm_userspace_memory_region,
}

impl<'a, T: Copy> Drop for VmMem<'a, T> {
    fn drop(&mut self) {
        self.ioctl_arg.memory_size = 0; // indicates request for deletion
        let ret = match self
            .mem
            .tracee
            .vm_ioctl_with_ref(ioctls::KVM_SET_USER_MEMORY_REGION(), &self.ioctl_arg)
        {
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

pub struct VCPU {
    pub idx: usize,
    pub fd_num: RawFd,
}

pub struct Hypervisor {
    pub pid: Pid,
    pub vm_fd: RawFd,
    pub vcpus: Vec<VCPU>,
}

impl Hypervisor {
    pub fn attach(&self) -> Result<Tracee> {
        let proc = try_with!(
            inject_syscall::attach(self.pid),
            "cannot attach to hypervisor"
        );
        Ok(Tracee {
            pid: self.pid,
            vm_fd: self.vm_fd,
            proc,
        })
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
        vm_fd: vm_fds[0],
        vcpus,
    })
}
