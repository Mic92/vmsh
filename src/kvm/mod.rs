use kvm_bindings as kvmb;
use libc::{c_int, c_ulong, c_void};
use log::warn;
use nix::sys::uio::{process_vm_readv, process_vm_writev, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use rand::Rng;
use simple_error::{bail, simple_error, try_with};
use std::ffi::OsStr;
use std::marker::PhantomData;
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::os::unix::prelude::RawFd;
use std::ptr;
use std::sync::{Arc, RwLock};

pub mod fd_transfer;
pub mod ioctls;
mod memslots;

use crate::cpu;
use crate::inject_syscall;
use crate::inject_syscall::Process as Injectee;
use crate::kvm::ioctls::KVM_CHECK_EXTENSION;
use crate::kvm::memslots::get_maps;
use crate::page_math;
use crate::proc::{openpid, Mapping, PidHandle};
use crate::result::Result;

/// This is a handle with abstractions for the syscall injector. Its primary goal is to be an interface for the
/// destructors of `HvMem` and `VmMem` to be able to (de-)allocate memory.
pub struct Tracee {
    pid: Pid,
    vm_fd: RawFd,
    /// The Process which is traced and injected into is blocked for the lifetime of Injectee.
    /// It may be `Tracee.attach`ed or `Tracee.detached` during Tracees lifetime. Most
    /// functions assume though, that the programmer has attached the Tracee beforehand. Therefore
    /// the programmer should always assure that the tracee it attached, before running
    /// other functions.
    /// This hold especially true for the destructor of for example `VmMem`.
    proc: Option<Injectee>,
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

impl Tracee {
    /// Attach to pid. The target `proc` will be stopped until `Self.detach` or the end of the
    /// lifetime of self.
    fn attach(&mut self) -> Result<()> {
        if let None = self.proc {
            self.proc = Some(try_with!(
                inject_syscall::attach(self.pid),
                "cannot attach to hypervisor"
            ));
        }
        Ok(())
    }

    fn detach(&mut self) {
        self.proc = None;
    }

    fn try_get_proc(&self) -> Result<&Injectee> {
        match &self.proc {
            None => Err(simple_error!("Programming error: Tracee is not attached.")),
            Some(proc) => Ok(&proc),
        }
    }

    fn vm_ioctl(&self, request: c_ulong, arg: c_ulong) -> Result<c_int> {
        let proc = self.try_get_proc()?;
        proc.ioctl(self.vm_fd, request, arg)
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
    pub fn vm_ioctl_with_ref<T: Sized + Copy>(
        &self,
        request: c_ulong,
        arg: &HvMem<T>,
    ) -> Result<c_int> {
        self.vm_ioctl(request, arg.ptr as c_ulong)
    }

    fn vcpu_ioctl(&self, vcpu: &VCPU, request: c_ulong, arg: c_ulong) -> Result<c_int> {
        let proc = self.try_get_proc()?;
        proc.ioctl(vcpu.fd_num, request, arg)
    }

    /// Make the kernel allocate anonymous memory (anywhere he likes, not bound to a file
    /// descriptor). This is not fully POSIX compliant, but works on linux.
    ///
    /// length in bytes.
    /// returns void pointer to the allocated virtual memory address of the hypervisor.
    unsafe fn mmap(&self, length: libc::size_t) -> Result<*mut c_void> {
        let proc = self.try_get_proc()?;
        let addr = libc::AT_NULL as *mut c_void; // make kernel choose location for us
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_SHARED | libc::MAP_ANONYMOUS;
        let fd = -1 as RawFd; // ignored because of MAP_ANONYMOUS => should be -1
        let offset = 0 as libc::off_t; // MAP_ANON => should be 0
        proc.mmap(addr, length, prot, flags, fd, offset)
    }

    fn anon_unix_client_connect(
        &self,
        anon_id_local: u64,
        anon_id_remote: u64,
        addr_local_mem: &HvMem<libc::sockaddr_un>,
        addr_remote_mem: &HvMem<libc::sockaddr_un>,
    ) -> Result<fd_transfer::Socket> {
        let proc = self.try_get_proc()?;
        fd_transfer::Socket::new_remote(&proc, anon_id_local, addr_local_mem)
    }

    pub fn connect_remote(
        &self,
        socket: &fd_transfer::Socket,
        anon_id_remote: u64,
        addr_remote_mem: &HvMem<libc::sockaddr_un>,
    ) -> Result<()> {
        let proc = self.try_get_proc()?;
        socket.connect_remote(&proc, anon_id_remote, addr_remote_mem)
    }

    fn recvmsg<MT: Sized + Copy, CM: Sized + Copy>(
        &self,
        sock: fd_transfer::Socket,
        msg_hdr_mem: &HvMem<libc::msghdr>,
        iov_mem: &HvMem<libc::iovec>,
        iov_buf_mem: &HvMem<MT>,
        cmsg_mem: &HvMem<CM>,
    ) -> Result<(Vec<MT>, Vec<RawFd>)> {
        let proc = self.try_get_proc()?;
        sock.receive_remote(&proc, msg_hdr_mem, iov_mem, iov_buf_mem, cmsg_mem)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    const unsafe fn CMSG_SPACE(length: libc::c_uint) -> libc::c_uint {
        //libc::CMSG_SPACE(length) TODO
        0
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    unsafe fn __CMSG_FIRSTHDR(
        msg_control: *mut libc::c_void,
        msg_controllen: libc::size_t,
    ) -> *mut libc::cmsghdr {
        let msg_hdr = libc::msghdr {
            msg_name: 0 as *mut libc::c_void,
            msg_namelen: 0,
            msg_iov: 0 as *mut libc::iovec,
            msg_iovlen: 0,
            msg_control,
            msg_controllen,
            msg_flags: 0,
        };
        libc::CMSG_FIRSTHDR(&msg_hdr as *const libc::msghdr)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    unsafe fn CMSG_LEN(length: libc::c_uint) -> libc::c_uint {
        libc::CMSG_LEN(length)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    unsafe fn CMSG_DATA(cmsg: *const libc::cmsghdr) -> *mut libc::c_uchar {
        libc::CMSG_DATA(cmsg)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_sregs(&self, vcpu: &VCPU, sregs: &HvMem<kvmb::kvm_sregs>) -> Result<kvmb::kvm_sregs> {
        use crate::kvm::ioctls::KVM_GET_SREGS;
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_SREGS(), sregs.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let sregs = try_with!(sregs.read(), "cannot read registers");
        Ok(sregs)
    }

    /// Get general-purpose pointer registers of VCPU
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_regs(&self, vcpu: &VCPU, regs: &HvMem<kvmb::kvm_regs>) -> Result<cpu::Regs> {
        use crate::kvm::ioctls::KVM_GET_REGS;
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_REGS(), regs.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let regs = try_with!(regs.read(), "cannot read registers");
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

    /// Get floating pointer registers of VCPU
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn get_fpu_regs(&self, vcpu: &VCPU, regs: &HvMem<kvmb::kvm_fpu>) -> Result<cpu::FpuRegs> {
        use crate::kvm::ioctls::KVM_GET_FPU;
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_FPU(), regs.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let regs = try_with!(regs.read(), "cannot read fpu registers");
        let st_space = unsafe { ptr::read(&regs.fpr as *const [u8; 16] as *const [u32; 32]) };
        let xmm_space =
            unsafe { ptr::read(&regs.xmm as *const [[u8; 16]; 16] as *const [u32; 64]) };

        Ok(cpu::FpuRegs {
            cwd: regs.fcw,
            swd: regs.fsw,
            twd: regs.ftwx as u16,
            fop: regs.last_opcode,
            rip: regs.last_ip,
            rdp: regs.last_dp,
            mxcsr: regs.mxcsr,
            mxcsr_mask: 0,
            st_space: st_space,
            xmm_space: xmm_space,
            padding: [0; 12],
            padding1: [0; 12],
        })
    }

    /// Get model-specific pointer registers of VCPU
    /// See https://github.com/rust-vmm/kvm-ioctls/blob/8eee8cd7ffea51c9463220f25e505b57b60cb2c7/src/ioctls/vcpu.rs#L522 for usage
    ///
    /// Returns number of successfull read register
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_msr(&self, vcpu: &VCPU, msrs: &HvMem<kvm_msrs>) -> Result<kvmb::kvm_msr_entry> {
        use crate::kvm::ioctls::KVM_GET_MSRS;
        // Here we trust the kernel not to read past the end of the kvm_msrs struct.
        try_with!(
            self.vcpu_ioctl(vcpu, KVM_GET_MSRS(), msrs.ptr as c_ulong),
            "vcpu_ioctl failed"
        );
        let msrs = try_with!(msrs.read(), "cannot read registers");
        Ok(msrs.entries[0])
    }

    /// Unmap memory in the process
    ///
    /// length in bytes.
    fn munmap(&self, addr: *mut c_void, length: libc::size_t) -> Result<()> {
        let proc = self.try_get_proc()?;
        proc.munmap(addr, length)
    }

    fn check_extension(&self, cap: c_int) -> Result<c_int> {
        self.vm_ioctl(KVM_CHECK_EXTENSION(), cap as c_ulong)
    }

    fn pid(&self) -> Pid {
        self.pid
    }

    fn get_maps(&self) -> Result<Vec<Mapping>> {
        get_maps(self)
    }
}

pub unsafe fn any_as_bytes<T: Sized>(p: &T) -> &[u8] {
    std::slice::from_raw_parts((p as *const T) as *const u8, size_of::<T>())
}

/// Hypervisor Memory
pub struct HvMem<T: Copy> {
    pub ptr: *mut c_void, // TODO replace with T
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

/// In theory this is dynamic however for for simplicity we limit it to 1 entry to not have to rewrite our vm allocation stack
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct kvm_msrs {
    pub nmsrs: u32,
    pub pad: u32,
    //
    pub entries: [kvmb::kvm_msr_entry; 1],
}

impl Hypervisor {
    fn attach(pid: Pid, vm_fd: RawFd) -> Result<Tracee> {
        Ok(Tracee {
            pid: pid,
            vm_fd: vm_fd,
            proc: None,
        })
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

    /// Safety: This function is safe even for the guest because VmMem enforces, that only the
    /// allocated T is written to.
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
        // safe, because TraceeMem enforces to write and read at most `size_of::<T> <= size` bytes.
        let ptr = unsafe { tracee.mmap(size)? };
        Ok(HvMem {
            ptr,
            pid: self.pid,
            tracee: self.tracee.clone(),
            phantom: PhantomData,
        })
    }

    /// allocate memory for T. Allocate more than necessary to increase allocation size to `size`.
    pub fn alloc_fam<T: Copy>(&self, size: usize) -> Result<HvMem<T>> {
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
        // safe, because TraceeMem enforces to write and read at most `size_of::<T> <= size` bytes.
        let ptr = unsafe { tracee.mmap(size)? };
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
        let iov_buf_mem = self.alloc_mem::<u8>()?;
        let cmsg_mem = self.alloc_mem::<[u8; 64]>()?;

        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        let vmsh_id: u64 = rand::thread_rng().gen::<u64>();
        //let vmsh_id: u64 = 10;
        println!("vmsh id {}", vmsh_id);
        let hypervisor_id = vmsh_id + 1;
        println!("vmsh  = new_server");
        let vmsh = fd_transfer::Socket::new(vmsh_id)?;
        println!("hypervisor = anon_unix_client_connect");
        let hypervisor = tracee.anon_unix_client_connect(
            hypervisor_id,
            vmsh_id,
            &addr_local_mem,
            &addr_remote_mem,
        )?;

        println!("vmsh connect");
        vmsh.connect(hypervisor_id)?;
        println!("hypervisor connect");
        tracee.connect_remote(&hypervisor, vmsh_id, &addr_remote_mem)?;

        let message = [1u8; 1];
        let m_slice = &message[0..1];
        let mut messages = Vec::with_capacity(fds.len());
        fds.iter().for_each(|_| messages.push(m_slice));

        println!("vmsh.send");
        vmsh.send(messages.as_slice(), fds)?;
        println!("hypervisor.recvmsg");
        let (msgs, fds) =
            tracee.recvmsg(hypervisor, &msg_hdr_mem, &iov_mem, &iov_buf_mem, &cmsg_mem)?;
        println!("done");
        println!("msg: {:?}", msgs[0]);
        println!("fd: {}", fds[0]);

        Ok(fds)
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
        let mut mem = self.alloc_mem()?;
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
        tracee.get_msr(vcpu, &mut mem)
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
        tracee: Arc::new(RwLock::new(Hypervisor::attach(pid, vm_fds[0])?)),
        vm_fd: vm_fds[0],
        vcpus,
    })
}
