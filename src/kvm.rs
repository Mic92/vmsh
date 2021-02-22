use libc::{c_int, c_ulong};
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::prelude::RawFd;
use std::path::Path;
use std::str;

use crate::inject_syscall;
use crate::kvm_ioctls::KVM_CHECK_EXTENSION;
use crate::kvm_memslots::get_maps;
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
    pub fn check_extension(self, cap: c_int) -> Result<c_int> {
        self.vm_ioctl(KVM_CHECK_EXTENSION(), cap)
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
    pub fn get_maps(self) -> Result<Vec<Mapping>> {
        get_maps(self)
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
        if fd.path == Path::new("anon_inode:kvm-vm") {
            vm_fds.push(fd.fd_num)
        // i.e. anon_inode:kvm-vcpu:0
        } else if fd.path.starts_with("anon_inode:kvm-vcpu:") {
            let parts = fd
                .path
                .as_os_str()
                .as_bytes()
                .rsplitn(1, |e| *e == b':')
                .collect::<Vec<_>>();
            assert!(parts.len() == 2);
            let num = try_with!(str::from_utf8(parts[1]), "invalid encoding");
            let idx = try_with!(
                num.parse::<usize>(),
                "cannot parse number of {}",
                fd.path.display()
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
        bail!("found multiple vcpus with same id. Assume multiple VMs in same hypervisor. This is not supported yet")
    };

    Ok((vm_fds, vcpu_fds))
}

pub fn get_hypervisor(pid: Pid) -> Result<Hypervisor> {
    let handle = try_with!(openpid(pid), "cannot handle to process {}", pid);

    let (vm_fds, vcpus) = try_with!(find_vm_fd(&handle), "failed to access kvm fds");
    let mappings = try_with!(handle.maps(), "cannot read process maps");
    if vm_fds.is_empty() {
        bail!("No VMs found in process {}", pid);
    }
    if vm_fds.len() > 1 {
        bail!(
            "Multiple VMs found in process {}. This is not supported yet.",
            pid
        );
    }
    if vcpus.is_empty() {
        bail!("Found KVM instance but no VCPUs in process {}", pid);
    }

    Ok(Hypervisor {
        pid,
        vm_fd: vm_fds[0],
        vcpus,
        mappings,
    })
}
