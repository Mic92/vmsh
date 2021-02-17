use nix::unistd::Pid;
use std::os::unix::prelude::RawFd;

use libc::{c_int, c_ulong};
use simple_error::try_with;

use crate::inject_syscall;
use crate::kvm_ioctls::KVM_CHECK_EXTENSION;
use crate::proc::Mapping;
use crate::result::Result;

pub struct Tracee<'a> {
    hypervisor: &'a Hypervisor,
    proc: inject_syscall::Process,
}

impl<'a> Tracee<'a> {
    fn vm_ioctl(&self, request: c_ulong, arg: c_int) -> Result<c_int> {
        self.proc.ioctl(self.hypervisor.vm_fd, request, arg)
    }
    fn cpu_ioctl(&self, cpu: usize, request: c_ulong, arg: c_int) -> Result<c_int> {
        self.proc.ioctl(self.hypervisor.vcpu_fds[cpu], request, arg)
    }
    pub fn check_extension(self, cap: c_int) -> Result<c_int> {
        return self.vm_ioctl(KVM_CHECK_EXTENSION(), cap);
    }
}

pub struct Hypervisor {
    pub pid: Pid,
    pub vm_fd: RawFd,
    pub vcpu_fds: Vec<RawFd>,
    pub mappings: Vec<Mapping>,
}

impl Hypervisor {
    pub fn attach<'a>(&'a self) -> Result<Tracee<'a>> {
        let proc = try_with!(
            inject_syscall::attach(self.pid),
            "cannot attach to hypervisor"
        );
        Ok(Tracee {
            hypervisor: self,
            proc: proc,
        })
    }
}

fn get_hypervisor(pid: Pid) -> Option<Hypervisor> {
    None
}
