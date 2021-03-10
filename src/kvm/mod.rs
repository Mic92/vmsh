use libc::{c_int, c_ulong, c_void};
use nix::sys::uio::{process_vm_readv, process_vm_writev, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::ffi::OsStr;
use std::os::unix::prelude::RawFd;

mod cpus;
mod ioctls;
mod memslots;

use crate::cpu::Regs;
use crate::inject_syscall;
use crate::kvm::ioctls::KVM_CHECK_EXTENSION;
use crate::kvm::memslots::get_maps;
use crate::proc::{openpid, Mapping, PidHandle};
use crate::result::Result;

pub struct Tracee<'a> {
    hypervisor: &'a Hypervisor,
    proc: inject_syscall::Process,
}

/// Safe wrapper for unsafe inject_syscall::Process operations.
impl<'a> Tracee<'a> {
    fn vm_ioctl(&self, request: c_ulong, arg: c_int) -> Result<c_int> {
        self.proc.ioctl(self.hypervisor.vm_fd, request, arg)
    }
    //fn cpu_ioctl(&self, cpu: usize, request: c_ulong, arg: c_int) -> Result<c_int> {
    //    self.proc
    //        .ioctl(self.hypervisor.vcpus[cpu].fd_num, request, arg)
    //}

    /// Make the kernel allocate anonymous memory (anywhere he likes, not bound to a file
    /// descriptor). This is not fully POSIX compliant, but works on linux.
    ///
    /// length in bytes.
    pub fn malloc(&self, length: libc::size_t) -> Result<*mut c_void> {
        let addr = libc::AT_NULL as *mut c_void; // make kernel choose location for us
        let length = 4 as libc::size_t; // in bytes
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_SHARED | libc::MAP_ANONYMOUS;
        let fd = 0 as RawFd; // ignored because of MAP_ANONYMOUS
        let offset = 0 as libc::off_t;
        self.proc.mmap(addr, length, prot, flags, fd, offset)
    }

    pub fn check_extension(&self, cap: c_int) -> Result<c_int> {
        self.vm_ioctl(KVM_CHECK_EXTENSION(), cap)
    }

    pub fn pid(&self) -> Pid {
        self.hypervisor.pid
    }
    pub fn get_maps(&self) -> Result<Vec<Mapping>> {
        get_maps(self)
    }
    pub fn mappings(&self) -> &[Mapping] {
        self.hypervisor.mappings.as_slice()
    }

    pub fn get_regs(&self, vcpu: &VCPU) -> Result<Regs> {
        let regs = Regs {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rax: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            orig_rax: 0,
            rip: 0,
            cs: 0,
            eflags: 0,
            rsp: 0,
            ss: 0,
            fs_base: 0,
            gs_base: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
        };
        Ok(regs)
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
    pub mappings: Vec<Mapping>,
}

impl Hypervisor {
    pub fn attach(&self) -> Result<Tracee> {
        let proc = try_with!(
            inject_syscall::attach(self.pid),
            "cannot attach to hypervisor"
        );
        Ok(Tracee {
            hypervisor: self,
            proc,
        })
    }

    /// read from the virtual addr of the hypervisor
    pub fn read_u32(&self, addr: usize) -> Result<u32> {
        const LEN: usize = 4;

        let mut buf = [0u8; LEN];
        let local_iovec = vec![IoVec::from_mut_slice(&mut buf)];
        let remote_iovec = vec![RemoteIoVec {
            base: addr,
            len: LEN,
        }];

        let f = try_with!(
            process_vm_readv(self.pid, local_iovec.as_slice(), remote_iovec.as_slice()),
            "cannot read hypervisor memory"
        );
        if f != LEN {
            bail!(
                "process_vm_readv read {} bytes when {} were expected",
                f,
                LEN
            )
        }

        let result: u32 = u32::from_ne_bytes(buf);
        return Ok(result);
    }

    /// write to the virtual addr of the hypervisor
    pub fn write_u32(&self, addr: usize, val: u32) -> Result<()> {
        const LEN: usize = 4;
        let mut buf = [0u8; LEN];
        buf = val.to_ne_bytes();
        let local_iovec = vec![IoVec::from_slice(&buf)];
        let remote_iovec = vec![RemoteIoVec {
            base: addr,
            len: LEN,
        }];

        let f = try_with!(
            process_vm_writev(self.pid, local_iovec.as_slice(), remote_iovec.as_slice()),
            "cannot read hypervisor memory"
        );
        if f != LEN {
            bail!(
                "process_vm_writev wrote {} bytes when {} were expected",
                f,
                LEN
            )
        }

        Ok(())
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
    let mappings = try_with!(handle.maps(), "cannot read process maps");
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
        mappings,
    })
}
