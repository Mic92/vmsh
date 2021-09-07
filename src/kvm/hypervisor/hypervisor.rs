use crate::cpu;
use crate::page_table::PhysAddr;
use crate::tracer::inject_syscall;
use kvm_bindings as kvmb;
use libc::c_int;
use log::*;
use nix::unistd::Pid;
use simple_error::{bail, require_with, simple_error, try_with};
use std::ffi::OsStr;
use std::mem::size_of;
use std::os::unix::io::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};

use super::ioeventfd::IoEventFd;
use super::ioregionfd::IoRegionFd;
use super::memory::*;
use crate::kvm::fd_transfer;
use crate::kvm::ioctls;
use crate::kvm::tracee::{kvm_msrs, Tracee};
use crate::page_math::{self, compute_host_offset};
use crate::result::Result;
use crate::tracer::proc::{openpid, Mapping, PidHandle};
use crate::tracer::wrap_syscall::KvmRunWrapper;

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone)]
pub struct VCPU {
    /// The idx as used in the inode name: anon_inode:kvm-vcpu:0
    pub idx: usize,
    pub fd_num: RawFd,
    /// hypervisor memory where fd_num is mapped to. Must be initialized before use.
    pub vcpu_map: Option<Mapping>,
}

impl VCPU {
    pub fn match_maps(vcpus: &mut Vec<VCPU>, vcpu_maps: &[Mapping]) {
        for vcpu in vcpus {
            let name = format!("{}{}", VCPUFD_INODE_NAME_STARTS_WITH, vcpu.idx);
            match vcpu_maps.iter().find(|map| map.pathname == name) {
                Some(map) => vcpu.vcpu_map = Some(map.clone()),
                None => warn!(
                    "no mapped memory of vcpu fd {} found called {}",
                    vcpu.fd_num, name
                ),
            }
        }
    }

    pub fn map(&self) -> Result<&Mapping> {
        self.vcpu_map.as_ref().ok_or_else(|| {
            simple_error!("vcpu_map must be initialized before use (programming error)")
        })
    }
}

struct TransferContext {
    local_sock: fd_transfer::Socket,
    remote_sock: fd_transfer::HvSocket,
    msg_hdr_mem: HvMem<libc::msghdr>,
    iov_mem: HvMem<libc::iovec>,
    iov_buf_mem: HvMem<[u8; 1]>,
    cmsg_mem: HvMem<[u8; 64]>,
}

/// Owns the tracee to prevent that multiple tracees are created for a Hypervisor. The Hypervisor
/// is used to handle the lock on `Self.tracee` and is used to instantiate `HvMem` and `VmMem`.
pub struct Hypervisor {
    pub pid: Pid,
    pub vm_fd: RawFd,
    pub vcpus: Vec<VCPU>,
    pub(super) tracee: Arc<RwLock<Tracee>>,
    pub wrapper: Mutex<Option<KvmRunWrapper>>,
    transfer_ctx: Mutex<Option<TransferContext>>,
}

impl Hypervisor {
    fn attach(pid: Pid, vm_fd: RawFd) -> Tracee {
        Tracee::new(pid, vm_fd, None)
    }

    pub fn setup_transfer_sockets(&mut self) -> Result<()> {
        let msg_hdr_mem = self.alloc_mem()?;
        let iov_mem = self.alloc_mem()?;
        let iov_buf_mem = self.alloc_mem::<[u8; 1]>()?;
        let cmsg_mem = self.alloc_mem::<[u8; 64]>()?; // should be of size CMSG_SPACE, but thats not possible at compile time

        let addr_local_mem = self.alloc_mem()?;
        let addr_remote_mem = self.alloc_mem()?;
        let vmsh_id = format!("vmsh_fd_transfer_{}", nix::unistd::getpid());
        let hypervisor_id = format!("vmsh_fd_transfer_{}", self.pid);
        let local_sock = fd_transfer::Socket::new(&vmsh_id)?;
        // remote_sock needs to outlive tracee or we run in a deadlock
        let remote_sock =
            fd_transfer::HvSocket::new(self.tracee.clone(), &hypervisor_id, &addr_local_mem)?;

        local_sock.connect(&hypervisor_id)?;

        let res = {
            let tracee = try_with!(
                self.tracee.write(),
                "cannot obtain tracee write lock: poinsoned"
            );
            let proc = tracee.try_get_proc()?;
            remote_sock.connect(proc, &vmsh_id, &addr_remote_mem)
        };
        try_with!(res, "failed to connect to local socket from hypervisor");
        self.transfer_ctx = Mutex::new(Some(TransferContext {
            local_sock,
            remote_sock,
            msg_hdr_mem,
            iov_mem,
            iov_buf_mem,
            cmsg_mem,
        }));
        Ok(())
    }

    pub fn close_transfer_sockets(&self) -> Result<()> {
        try_with!(self.transfer_ctx.lock(), "cannot take lock").take();
        Ok(())
    }

    /// Must be called from the thread that created Hypervisor before using it in a different thread
    pub fn prepare_thread_transfer(&self) -> Result<()> {
        try_with!(self.tracee.write(), "cannot take write lock").disown()
    }

    /// Must be called from the new thread that wants to use Hypervisor.
    pub fn finish_thread_transfer(&self) -> Result<()> {
        try_with!(self.tracee.write(), "cannot take write lock").adopt()
    }

    pub fn resume(&self) -> Result<()> {
        let mut tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        let _ = tracee.detach();
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

    pub fn tracee_write_guard(&self) -> Result<RwLockWriteGuard<Tracee>> {
        let twg: RwLockWriteGuard<Tracee> = try_with!(
            self.tracee.write(),
            "cannot obtain tracee read lock: poinsoned"
        );

        Ok(twg)
    }

    /// run code while having full control over ioctl(KVM_RUN).
    /// Guarantees that self.wrapper is Some() during f().
    /// Can be called regardless of de/attached state.
    pub fn kvmrun_wrapped(
        &self,
        mut f: impl FnMut(&Mutex<Option<KvmRunWrapper>>) -> Result<()>,
    ) -> Result<()> {
        // detach tracee and convert to owned wrapper
        let (was_attached, wrapper) = {
            let mut tracee = try_with!(
                self.tracee.write(),
                "cannot obtain tracee read lock: poinsoned"
            );
            match tracee.detach() {
                Some(injector) => {
                    let wrapper = KvmRunWrapper::from_tracer(inject_syscall::into_tracer(
                        injector,
                        self.vcpus.clone(),
                    )?)?;
                    (true, wrapper)
                }
                None => {
                    let wrapper = KvmRunWrapper::attach(self.pid, &self.vcpus)?;
                    (false, wrapper)
                }
            }
        };

        // put wrapper: self.wrapper = Some(wrapper)
        {
            let mut self_wrapper = try_with!(self.wrapper.lock(), "cannot obtain wrapper mutex");
            let _ = self_wrapper.replace(wrapper);
        }

        let res = f(&self.wrapper);

        // take wrapper out of self.wrapper
        let wrapper: KvmRunWrapper;
        {
            let mut wguard = try_with!(self.wrapper.lock(), "cannot obtain wrapper mutex");
            wrapper = require_with!(wguard.take(), "earlier in this function we put it here");
        }

        // convert wrapper to tracee and attach it
        {
            let mut tracee = try_with!(
                self.tracee.write(),
                "cannot obtain tracee read lock: poinsoned"
            );
            if was_attached {
                let err =
                    "cannot re-attach injector after having detached it favour of KvmRunWrapper";
                let injector = try_with!(inject_syscall::from_tracer(wrapper.into_tracer()?), &err);
                try_with!(tracee.attach_to(injector), &err);
            }
        }
        try_with!(res, "closure on KvmRunWrapper failed");

        Ok(())
    }

    pub fn get_maps(&self) -> Result<Vec<Mapping>> {
        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee read lock: poinsoned"
        );
        tracee.get_maps()
    }

    pub fn get_vcpu_maps(&self) -> Result<Vec<Mapping>> {
        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee read lock: poinsoned"
        );
        tracee.get_vcpu_maps()
    }

    /// `readonly`: If true, a guest writing to it leads to KVM_EXIT_MMIO.
    ///
    /// Safety: This function is safe even for the guest because VmMem enforces, that only the
    /// allocated T is written to.
    pub fn vm_add_mem<T: Sized + Copy>(
        &self,
        guest_addr: u64,
        size: usize,
        readonly: bool,
    ) -> Result<PhysMem<T>> {
        // must be a multiple of PAGESIZE
        let slot_len = page_math::page_align(size);
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
        let host_offset = compute_host_offset(hv_memslot.ptr, guest_addr as usize);
        Ok(PhysMem {
            mem: hv_memslot,
            ioctl_arg: arg_hv,
            guest_phys_addr: PhysAddr {
                value: guest_addr as usize,
                host_offset,
            },
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
            ptr: ptr as libc::uintptr_t,
            pid: self.pid,
            tracee: self.tracee.clone(),
            phantom: SendPhantom::default(),
        })
    }

    pub fn transfer(&self, fds: &[RawFd]) -> Result<Vec<RawFd>> {
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );

        let message = [1u8; 1];
        let m_slice = &message[0..1];
        let mut messages = Vec::with_capacity(fds.len());
        fds.iter().for_each(|_| messages.push(m_slice));
        let ctx = try_with!(self.transfer_ctx.lock(), "cannot lock transfer context");
        let ctx = require_with!(ctx.as_ref(), "transfer context was not set up");

        let proc = tracee.try_get_proc()?;
        ctx.local_sock.send(messages.as_slice(), fds)?;
        let (msg, fds) = ctx.remote_sock.receive(
            proc,
            &ctx.msg_hdr_mem,
            &ctx.iov_mem,
            &ctx.iov_buf_mem,
            &ctx.cmsg_mem,
        )?;
        if msg != message {
            bail!("received message differs from sent one");
        }
        Ok(fds)
    }

    pub fn ioeventfd(&self, guest_addr: u64) -> Result<IoEventFd> {
        self.ioeventfd_(guest_addr, 0, None)
    }

    /// @param len: 0, 1, 2, 4, or 8 [bytes]
    /// TODO check support for 0 via KVM_CAP_IOEVENTFD_ANY_LENGTH
    pub fn ioeventfd_(
        &self,
        guest_addr: u64,
        len: u32,
        datamatch: Option<u64>,
    ) -> Result<IoEventFd> {
        IoEventFd::new(self, guest_addr, len, datamatch)
    }

    pub fn ioregionfd(&self, start: u64, len: usize) -> Result<IoRegionFd> {
        IoRegionFd::new(self, start, len)
    }

    /// param `gsi`: pin on the irqchip to be toggled by fd events
    pub fn irqfd(&self, gsi: u32) -> Result<EventFd> {
        let eventfd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd");
        info!("irqfd {:?}, interupt gsi/nr {:?}", eventfd.as_raw_fd(), gsi);
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

    pub fn userfaultfd(&self) -> Result<c_int> {
        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee read lock: poinsoned"
        );
        let proc = tracee.try_get_proc()?;

        let uffd = proc.userfaultfd(libc::O_NONBLOCK)?;
        if uffd <= 0 {
            bail!("userfaultfd failed with {}", uffd);
        }

        // TODO impl userfaultfd handling

        Ok(-1)
    }

    pub fn check_extension(&self, cap: c_int) -> Result<c_int> {
        let tracee = try_with!(
            self.tracee.read(),
            "cannot obtain tracee read lock: poinsoned"
        );
        tracee.check_extension(cap)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_cpuid2(&self, vcpu: &VCPU) -> Result<ioctls::kvm_cpuid2> {
        let mem = self.alloc_mem()?;
        try_with!(
            mem.write(&ioctls::kvm_cpuid2 {
                nent: ioctls::KVM_MAX_CPUID_ENTRIES as u32,
                padding: 0,
                entries: [kvmb::kvm_cpuid_entry2 {
                    function: 0,
                    index: 0,
                    flags: 0,
                    eax: 0,
                    ebx: 0,
                    ecx: 0,
                    edx: 0,
                    padding: [0; 3],
                }; ioctls::KVM_MAX_CPUID_ENTRIES]
            }),
            "cannot update cpuid2 kvm structure in hypervisor"
        );
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.get_cpuid2(vcpu, &mem)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    pub fn get_irqchip(&self, chip_id: u32) -> Result<kvmb::kvm_irqchip> {
        let mem = self.alloc_mem()?;
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        try_with!(
            mem.write(&kvmb::kvm_irqchip {
                chip_id,
                ..Default::default()
            }),
            "cannot update kvm_irqchip structure"
        );
        tracee.get_irqchip(&mem)
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
    pub fn set_regs(&self, vcpu: &VCPU, regs: &cpu::Regs) -> Result<()> {
        let mem = self.alloc_mem()?;
        let regs = kvmb::kvm_regs {
            rax: regs.rax,
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rsp: regs.rsp,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            rflags: regs.eflags,
        };
        mem.write(&regs)?;
        let tracee = try_with!(
            self.tracee.write(),
            "cannot obtain tracee write lock: poinsoned"
        );
        tracee.set_regs(vcpu, &mem)
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

pub const VMFD_INODE_NAME: &str = "anon_inode:kvm-vm";
pub const VCPUFD_INODE_NAME_STARTS_WITH: &str = "anon_inode:kvm-vcpu:";

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
        if name == VMFD_INODE_NAME {
            vm_fds.push(fd.fd_num)
        // i.e. anon_inode:kvm-vcpu:0
        } else if name.starts_with(VCPUFD_INODE_NAME_STARTS_WITH) {
            let parts = name.rsplitn(2, ':').collect::<Vec<_>>();
            assert!(parts.len() == 2);
            let idx = try_with!(
                parts[0].parse::<usize>(),
                "cannot parse number {}",
                parts[0]
            );
            info!("vcpu {} fd {}", idx, fd.fd_num);
            vcpu_fds.push(VCPU {
                idx,
                fd_num: fd.fd_num,
                vcpu_map: None,
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

    let (vm_fds, mut vcpus) = try_with!(find_vm_fd(&handle), "failed to access kvm fds");
    if vm_fds.is_empty() {
        bail!("no KVM-VMs found. If this is qemu, does it enable KVM?");
    }
    if vm_fds.len() > 1 {
        bail!("multiple VMs found, this is not supported yet.");
    }

    let tracee = Hypervisor::attach(pid, vm_fds[0]);
    let vcpu_maps = try_with!(tracee.get_vcpu_maps(), "cannot get vcpufd memory maps");
    if vcpus.is_empty() {
        bail!("found KVM instance but no VCPUs");
    }
    if vcpu_maps.is_empty() {
        bail!("found VCPUs but no mappings of their fds");
    }
    VCPU::match_maps(&mut vcpus, &vcpu_maps);
    Ok(Hypervisor {
        pid,
        tracee: Arc::new(RwLock::new(tracee)),
        vm_fd: vm_fds[0],
        vcpus,
        wrapper: Mutex::new(None),
        transfer_ctx: Mutex::new(None),
    })
}
