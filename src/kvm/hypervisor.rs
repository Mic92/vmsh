use crate::page_table::PhysAddr;
use crate::tracer::inject_syscall;
use kvm_bindings as kvmb;
use libc::{c_int, c_void};
use log::*;
use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
use nix::unistd::Pid;
use nix::unistd::{read, write};
use simple_error::{bail, simple_error, try_with};
use std::ffi::OsStr;
use std::marker::PhantomData;
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::os::unix::io::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};
use vm_memory::remote_mem;
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};

use crate::cpu;
use crate::devices::virtio::{register_ioeventfd, MmioConfig};
use crate::devices::USE_IOREGIONFD;
use crate::kvm::fd_transfer;
use crate::kvm::ioctls;
use crate::kvm::ioctls::kvm_ioregion;
use crate::kvm::ioctls::{ioregionfd_cmd, ioregionfd_resp};
use crate::kvm::tracee::{kvm_msrs, Tracee};
use crate::page_math::{self, compute_host_offset};
use crate::result::Result;
use crate::tracer::proc::{openpid, Mapping, PidHandle};
use crate::tracer::wrap_syscall::KvmRunWrapper;

pub fn process_read<T: Sized + Copy>(pid: Pid, addr: *const c_void) -> Result<T> {
    remote_mem::process_read(pid, addr).map_err(|e| simple_error!("{}", e))
}

pub fn process_write<T: Sized + Copy>(pid: Pid, addr: *mut c_void, val: &T) -> Result<()> {
    remote_mem::process_write(pid, addr, val).map_err(|e| simple_error!("{}", e))
}

/// Hypervisor Memory
#[derive(Debug)]
pub struct HvMem<T: Copy> {
    pub ptr: libc::uintptr_t,
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
    ioctl_arg: HvMem<kvmb::kvm_userspace_memory_region>,
    pub guest_phys_addr: PhysAddr,
}

impl<T: Copy> Drop for PhysMem<T> {
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

#[allow(clippy::upper_case_acronyms)]
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
    /// hypervisor memory where fd_num is mapped to.
    /// sorted by vcpu nr. TODO what if there exist cpu [0, 1, 4]?
    pub vcpu_maps: Vec<Mapping>,
    tracee: Arc<RwLock<Tracee>>,
    pub wrapper: Mutex<Option<KvmRunWrapper>>,
}

/// Abstraction around IoEventFd and EventFd.
/// They can't be placed into the same memory location without an enum, because
/// vmm_sys_util::EventFd doesn't implement Sized which is required for `dyn`.
pub enum IoEvent {
    IoEventFd(IoEventFd),
    EventFd(EventFd),
}

impl IoEvent {
    pub fn register(
        vmm: &Arc<Hypervisor>,
        uioefd: &mut UserspaceIoEventFd,
        mmio_cfg: &MmioConfig,
        queue_idx: u64,
    ) -> Result<IoEvent> {
        if USE_IOREGIONFD {
            let eventfd = try_with!(
                uioefd.userpace_ioeventfd(Some(queue_idx as u32)),
                "cannot register userspace ioeventfd"
            );
            Ok(IoEvent::EventFd(eventfd))
        } else {
            let ioeventfd = try_with!(
                register_ioeventfd(vmm, mmio_cfg, queue_idx),
                "cannot register ioeventfd"
            );
            Ok(IoEvent::IoEventFd(ioeventfd))
        }
    }
}

impl AsRawFd for IoEvent {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            IoEvent::IoEventFd(e) => e.as_raw_fd(),
            IoEvent::EventFd(e) => e.as_raw_fd(),
        }
    }
}

impl Deref for IoEvent {
    type Target = EventFd;
    fn deref(&self) -> &EventFd {
        match self {
            IoEvent::IoEventFd(e) => e,
            IoEvent::EventFd(e) => e,
        }
    }
}

pub struct IoEventFd {
    fd: EventFd,
    hv_eventfd: RawFd,
    hv_mem: HvMem<kvmb::kvm_ioeventfd>,
    tracee: Arc<RwLock<Tracee>>,
    guest_addr: u64,
    len: u32,
    datamatch: Option<u64>,
}

fn kvm_ioeventfd(
    hv_eventfd: RawFd,
    guest_addr: u64,
    len: u32,
    datamatch: Option<u64>,
) -> kvmb::kvm_ioeventfd {
    let mut flags = 0;
    let mut datam = 0;
    if let Some(data) = datamatch {
        flags = 1 << kvmb::kvm_ioeventfd_flag_nr_datamatch;
        datam = data;
    }

    kvmb::kvm_ioeventfd {
        len,
        datamatch: datam,
        addr: guest_addr,
        fd: hv_eventfd.as_raw_fd(),
        flags,
        ..Default::default()
    }
}
impl IoEventFd {
    fn new(
        hv: &Hypervisor,
        guest_addr: u64,
        len: u32,
        datamatch: Option<u64>,
    ) -> Result<IoEventFd> {
        let eventfd = try_with!(EventFd::new(EFD_NONBLOCK), "cannot create event fd");
        info!(
            "ioeventfd {}, guest phys addr 0x{:x}",
            eventfd.as_raw_fd(),
            guest_addr
        );
        let hv_eventfd = hv.transfer(vec![eventfd.as_raw_fd()].as_slice())?[0];
        let ioeventfd = kvm_ioeventfd(hv_eventfd, guest_addr, len, datamatch);

        let mem = hv.alloc_mem()?;
        mem.write(&ioeventfd)?;

        let ret = {
            let tracee = try_with!(
                hv.tracee.read(),
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

        Ok(IoEventFd {
            guest_addr,
            len,
            datamatch,
            hv_eventfd,
            hv_mem: mem,
            fd: eventfd,
            tracee: hv.tracee.clone(),
        })
    }
}

impl Drop for IoEventFd {
    fn drop(&mut self) {
        let tracee = match self.tracee.read() {
            Err(e) => {
                warn!("IoEventfd: Could not aquire lock: {}", e);
                return;
            }
            Ok(t) => t,
        };
        let mut ioeventfd =
            kvm_ioeventfd(self.hv_eventfd, self.guest_addr, self.len, self.datamatch);
        ioeventfd.flags |= 1 << kvmb::kvm_ioeventfd_flag_nr_deassign;

        if let Err(e) = self.hv_mem.write(&ioeventfd) {
            warn!(
                "IoEventFd: Could not write to HvMem while dropping IoEventFd: {}",
                e
            );
            return;
        }

        if let Err(e) = tracee.vm_ioctl_with_ref(ioctls::KVM_IOEVENTFD(), &self.hv_mem) {
            warn!("IoEventfd: kvm ioeventfd ioctl injection failed: {}", e)
        }

        if let Err(e) = tracee.close(self.hv_eventfd) {
            warn!("IoEventfd: failed to close eventfd in hypervisor: {}", e)
        }
    }
}

use std::ops::Deref;
impl Deref for IoEventFd {
    type Target = EventFd;
    fn deref(&self) -> &EventFd {
        &self.fd
    }
}

impl AsRawFd for IoEventFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

struct UIoEFd {
    datamatch: Option<u32>,
    fd: EventFd,
}

/// For 32bit wide accesses on the queue notify register of virtio devices. Requires one UIoEventFd
/// per device.
pub struct UserspaceIoEventFd {
    ioeventfds: Vec<UIoEFd>,
}

impl Default for UserspaceIoEventFd {
    fn default() -> Self {
        Self { ioeventfds: vec![] }
    }
}

impl UserspaceIoEventFd {
    pub fn userpace_ioeventfd(&mut self, datamatch: Option<u32>) -> Result<EventFd> {
        if datamatch.is_none() && !self.ioeventfds.is_empty() {
            bail!("cannot add a userspace ioeventfd without datamatch when others with datamatch have already been registered");
        }

        let fd = try_with!(
            EventFd::new(EFD_NONBLOCK),
            "cannot create non-blocking eventfd for uioefd"
        );
        log::info!("eventfd {:?} for ioregionfd", fd.as_raw_fd(),);
        let uioefd = UIoEFd {
            datamatch,
            fd: try_with!(fd.try_clone(), "cannot clone uioefd"),
        };
        self.ioeventfds.push(uioefd);
        Ok(fd)
    }

    /// Callback for writes to QueueNotify register. Forwards notification to the corresponding
    /// EventFd reader.
    pub fn queue_notify(&self, val: u32) {
        let criterion = |uioefd: &&UIoEFd| match uioefd.datamatch {
            Some(dm) => dm == val,
            None => true,
        };
        if let Some(ioefd) = self.ioeventfds.iter().find(criterion) {
            // ioefd is the correct fd from our list
            if let Err(e) = ioefd.fd.write(1) {
                log::trace!(
                    "cannot write to UserspaceIoEventFd (datamatch {:?}, fd {}): {}",
                    ioefd.datamatch,
                    ioefd.fd.as_raw_fd(),
                    e
                );
            }
        } else {
            log::trace!("cannot find datamatch for ");
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IoRegionFd {
    pub ioregion: kvm_ioregion,
    rfile: RawFd, // our end: we write responses here
    wfile: RawFd, // we read commands from here
    rf_hv: RawFd, // their end: transferred to hyperisor
    wf_hv: RawFd,
}

impl IoRegionFd {
    fn new(hv: &Hypervisor, guest_paddr: u64, len: usize) -> Result<Self> {
        if Self::capability_present(hv)? {
            bail!("This operation requires KVM_CAP_IOREGIONFD which your KVM does not have.");
        }

        let (rf_dev, rf_hv) = try_with!(
            socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC
            ),
            "cannot create sockets for IoRegionFd"
        );
        let (wf_dev, wf_hv) = try_with!(
            socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC
            ),
            "cannot create sockets for IoRegionFd"
        );
        let hv_rf_hv = hv.transfer(vec![rf_hv.as_raw_fd()].as_slice())?[0];
        let hv_wf_hv = hv.transfer(vec![wf_hv.as_raw_fd()].as_slice())?[0];
        let ioregion = kvm_ioregion::new(guest_paddr, len, hv_rf_hv, hv_wf_hv);
        let mem = hv.alloc_mem()?;
        mem.write(&ioregion)?;

        let ret = {
            let tracee = try_with!(
                hv.tracee.read(),
                "cannot obtain tracee read lock: poinsoned"
            );
            try_with!(
                tracee.vm_ioctl_with_ref(ioctls::KVM_SET_IOREGION(), &mem),
                "kvm ioeventfd ioctl injection failed"
            )
        };
        if ret != 0 {
            bail!("ioregionfd ioctl failed with {}", ret);
        }

        Ok(IoRegionFd {
            ioregion,
            rfile: rf_dev,
            wfile: wf_dev,
            rf_hv,
            wf_hv,
        })
    }

    pub fn capability_present(hv: &Hypervisor) -> Result<bool> {
        let has_cap = try_with!(
            hv.check_extension(ioctls::KVM_CAP_IOREGIONFD as i32),
            "cannot check kvm extension capabilities"
        );
        Ok(has_cap == 0)
    }

    /// receive read and write events/commands
    pub fn read(&self) -> Result<ioregionfd_cmd> {
        let len = size_of::<ioregionfd_cmd>();
        let mut t_mem = MaybeUninit::<ioregionfd_cmd>::uninit();
        // safe, because slice.len() == len
        let t_slice = unsafe { std::slice::from_raw_parts_mut(t_mem.as_mut_ptr() as *mut u8, len) };
        let read = try_with!(
            read(self.wfile, t_slice),
            "read on ioregionfd {} failed",
            self.wfile
        );
        if read != len {
            bail!(
                "read returned ioregionfd command of incorrect length {}",
                read
            );
        }
        // safe, because we wrote exactly the correct amount of bytes (len)
        let cmd: ioregionfd_cmd = unsafe { t_mem.assume_init() };
        log::trace!(
            "read {:?}, {:?}, response={}: {:?}",
            cmd.info.cmd(),
            cmd.info.size(),
            cmd.info.is_response(),
            cmd
        );
        Ok(cmd)
    }

    /// Write a response back to the VM.
    pub fn write_slice(&self, data: &[u8]) -> Result<()> {
        log::trace!("write_slice()");
        let mut arr = [0u8; 8];
        let arr_slice = &mut arr[0..data.len()];
        arr_slice.copy_from_slice(data);
        self.write(u64::from_ne_bytes(arr))
    }

    /// Write a response back to the VM.
    pub fn write(&self, data: u64) -> Result<()> {
        log::trace!("write {:x}", data);
        let len = size_of::<ioregionfd_resp>();
        let response = ioregionfd_resp::new(data);
        // safe, because we won't need t_bytes for longer than this stack frame
        let t_bytes = unsafe {
            std::slice::from_raw_parts((&response as *const ioregionfd_resp) as *const u8, len)
        };
        let written = try_with!(
            write(self.rfile, t_bytes),
            "write on ioregionfd {} failed",
            self.rfile
        );
        if written != len {
            bail!("cannot write entire ioregionfd command {}/{}", written, len);
        }
        Ok(())
    }
}

// TODO impl Drop for IoRegionFd

impl Hypervisor {
    fn attach(pid: Pid, vm_fd: RawFd) -> Tracee {
        Tracee::new(pid, vm_fd, None)
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
                        self.vcpu_maps[0].clone(),
                    )?)?;
                    (true, wrapper)
                }
                None => {
                    let wrapper = KvmRunWrapper::attach(self.pid, &self.vcpu_maps)?;
                    (false, wrapper)
                }
            }
        };

        // put wrapper: self.wrapper = Some(wrapper)
        {
            let mut self_wrapper = try_with!(self.wrapper.lock(), "cannot obtain wrapper mutex");
            let _ = self_wrapper.replace(wrapper);
        }

        try_with!(f(&self.wrapper), "closure on KvmRunWrapper failed");

        // take wrapper out of self.wrapper
        let wrapper: KvmRunWrapper;
        {
            let mut wguard = try_with!(self.wrapper.lock(), "cannot obtain wrapper mutex");
            wrapper = wguard.take().expect("earlier in this fn we put it here");
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

    Ok(Hypervisor {
        pid,
        tracee: Arc::new(RwLock::new(tracee)),
        vm_fd: vm_fds[0],
        vcpus,
        vcpu_maps,
        wrapper: Mutex::new(None),
    })
}
