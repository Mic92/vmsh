use libc::{c_int, c_ulong};
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

impl<'a> Tracee<'a> {
    fn vm_ioctl(&self, request: c_ulong, arg: c_int) -> Result<c_int> {
        self.proc.ioctl(self.hypervisor.vm_fd, request, arg)
    }
    //fn cpu_ioctl(&self, cpu: usize, request: c_ulong, arg: c_int) -> Result<c_int> {
    //    self.proc
    //        .ioctl(self.hypervisor.vcpus[cpu].fd_num, request, arg)
    //}
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
