use kvm_bindings as kvmb;
use libc::{c_int, c_ulong, c_void};
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::os::unix::prelude::RawFd;
use std::ptr;

use crate::cpu;
use crate::kvm::hypervisor::{HvMem, VCPU};
use crate::kvm::ioctls::KVM_CHECK_EXTENSION;
use crate::kvm::memslots::{get_maps, get_vcpu_maps};
use crate::result::Result;
use crate::tracer::inject_syscall;
use crate::tracer::inject_syscall::Process as Injectee;
use crate::tracer::proc::Mapping;

/// In theory this is dynamic however for for simplicity we limit it to 1 entry to not have to rewrite our vm allocation stack
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct kvm_msrs {
    pub nmsrs: u32,
    pub pad: u32,
    //
    pub entries: [kvmb::kvm_msr_entry; 1],
}

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

impl Tracee {
    pub fn new(pid: Pid, vm_fd: RawFd, proc: Option<Injectee>) -> Tracee {
        Tracee { pid, vm_fd, proc }
    }

    /// Attach to pid. The target `proc` will be stopped until `Self.detach` or the end of the
    /// lifetime of self.
    pub fn attach(&mut self) -> Result<()> {
        if self.proc.is_none() {
            self.proc = Some(try_with!(
                inject_syscall::attach(self.pid),
                "cannot attach to hypervisor"
            ));
        }
        Ok(())
    }

    /// See attach()
    pub fn attach_to(&mut self, injector: Injectee) -> Result<()> {
        let inj_pid = injector.main_thread().tid;
        if self.pid != inj_pid {
            bail!(
                "cannot attach Tracee {} using a tracer on {}",
                self.pid,
                inj_pid
            );
        }
        if self.proc.is_some() {
            bail!("cannot attach tracee because it is already attach to something else");
        }

        self.proc = Some(injector);
        Ok(())
    }

    pub fn detach(&mut self) -> Option<Injectee> {
        self.proc.take()
    }

    pub fn try_get_proc(&self) -> Result<&Injectee> {
        match &self.proc {
            None => bail!("programming error: tracee is not attached."),
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
    ///
    /// # Safety
    ///
    /// Safe for this crate, not so for the remote process being manipulated. Ensure that to write
    /// and read at most `size_of::<T> <= size` bytes.
    pub fn mmap(&self, length: libc::size_t) -> Result<*mut c_void> {
        let proc = self.try_get_proc()?;
        let addr = libc::AT_NULL as *mut c_void; // make kernel choose location for us
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_SHARED | libc::MAP_ANONYMOUS;
        let fd = -1; // ignored because of MAP_ANONYMOUS => should be -1
        let offset = 0; // MAP_ANON => should be 0
        proc.mmap(addr, length, prot, flags, fd, offset)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    /// You are free to try to convince the compiler that this is constant. In theory it is.
    ///
    /// # Safety
    ///
    /// This is pointer calculus.
    #[allow(non_snake_case)]
    pub unsafe fn CMSG_SPACE(length: libc::c_uint) -> libc::c_uint {
        libc::CMSG_SPACE(length)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    ///
    /// # Safety
    ///
    /// This is pointer calculus.
    #[allow(non_snake_case)]
    pub unsafe fn __CMSG_FIRSTHDR(
        msg_control: *mut libc::c_void,
        msg_controllen: libc::size_t,
    ) -> *mut libc::cmsghdr {
        let msg_hdr = libc::msghdr {
            msg_name: std::ptr::null_mut::<libc::c_void>(),
            msg_namelen: 0,
            msg_iov: std::ptr::null_mut::<libc::iovec>(),
            msg_iovlen: 0,
            msg_control,
            msg_controllen,
            msg_flags: 0,
        };
        libc::CMSG_FIRSTHDR(&msg_hdr as *const libc::msghdr)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    ///
    /// # Safety
    ///
    /// This is pointer calculus.
    #[allow(non_snake_case)]
    pub unsafe fn CMSG_LEN(length: libc::c_uint) -> libc::c_uint {
        libc::CMSG_LEN(length)
    }

    /// Guarantees not to allocate or follow pointers. Pure pointer calculus.
    ///
    /// # Safety
    ///
    /// This is pointer calculus.
    #[allow(non_snake_case)]
    pub unsafe fn CMSG_DATA(cmsg: *const libc::cmsghdr) -> *mut libc::c_uchar {
        libc::CMSG_DATA(cmsg)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_sregs(
        &self,
        vcpu: &VCPU,
        sregs: &HvMem<kvmb::kvm_sregs>,
    ) -> Result<kvmb::kvm_sregs> {
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
    pub fn get_regs(&self, vcpu: &VCPU, regs: &HvMem<kvmb::kvm_regs>) -> Result<cpu::Regs> {
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
    pub fn get_fpu_regs(&self, vcpu: &VCPU, regs: &HvMem<kvmb::kvm_fpu>) -> Result<cpu::FpuRegs> {
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
            st_space,
            xmm_space,
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
    pub fn munmap(&self, addr: *mut c_void, length: libc::size_t) -> Result<()> {
        let proc = self.try_get_proc()?;
        proc.munmap(addr, length)
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

    pub fn get_vcpu_maps(&self) -> Result<Vec<Mapping>> {
        get_vcpu_maps(self.pid)
    }
}
