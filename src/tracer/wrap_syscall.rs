use crate::tracer::Tracer;
use kvm_bindings as kvmb;
use log::{debug, trace, warn};
use nix::unistd::getpgid;
use nix::unistd::Pid;
use nix::{
    errno::Errno,
    sys::wait::{waitpid, WaitStatus},
};
use nix::{sys::signal::Signal, unistd::getpgrp};
use simple_error::bail;
use simple_error::try_with;
use std::{
    fmt,
    thread::{current, ThreadId},
};

use crate::kvm::hypervisor;
use crate::kvm::ioctls;
use crate::result::Result;
use crate::tracer::proc::Mapping;
use crate::tracer::ptrace;

type MmioRwRaw = kvmb::kvm_run__bindgen_ty_1__bindgen_ty_6;
pub const MMIO_RW_DATA_MAX: usize = 8;

pub struct MmioRw {
    /// address in the guest physical memory
    pub addr: u64,

    pub is_write: bool,
    data: [u8; MMIO_RW_DATA_MAX],
    len: usize,
    pid: Pid,
    vcpu_map: Mapping,
}

impl MmioRw {
    #[must_use]
    pub fn new(raw: &MmioRwRaw, pid: Pid, vcpu_map: Mapping) -> MmioRw {
        // should we sanity check len here in order to not crash on out of bounds?
        // should we check that vcpu_map is big enough for kvm_run?
        MmioRw {
            addr: raw.phys_addr,
            is_write: raw.is_write != 0,
            data: raw.data,
            len: raw.len as usize,
            pid,
            vcpu_map,
        }
    }

    #[must_use]
    pub fn from(kvm_run: &kvmb::kvm_run, pid: Pid, vcpu_map: Mapping) -> Option<MmioRw> {
        match kvm_run.exit_reason {
            kvmb::KVM_EXIT_MMIO => {
                // Safe because the exit_reason (which comes from the kernel) told us which
                // union field to use.
                let mmio: &MmioRwRaw = unsafe { &kvm_run.__bindgen_anon_1.mmio };
                Some(MmioRw::new(mmio, pid, vcpu_map))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }

    fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    /// # Safety of the tracee
    ///
    /// Do not run this function when the traced process has continued since
    /// the last `KvmRunWrapper.wait_for_ioctl()`! Additionally it assumes that the last
    /// `wait_for_ioctl()` has returned with Some(_)!
    ///
    /// TODO refactor api to reflect those preconditions better.
    pub fn answer_read(&mut self, data: &[u8]) -> Result<()> {
        if self.is_write {
            bail!("cannot answer a mmio write with a read value");
        }
        if data.len() != self.len {
            bail!(
                "cannot answer mmio read of {}b with {}b",
                self.len,
                data.len()
            );
        }
        self.data_mut().clone_from_slice(data);

        let kvm_run_ptr = self.vcpu_map.start as *mut kvm_bindings::kvm_run;
        // safe because those pointers will not be used in our process :) and additionally Self::new
        // may or may not perform vcpu_map size assertions.
        let mmio_ptr: *mut MmioRwRaw = unsafe { &mut ((*kvm_run_ptr).__bindgen_anon_1.mmio) };
        let data_ptr: *mut [u8; MMIO_RW_DATA_MAX] = unsafe { &mut ((*mmio_ptr).data) };
        hypervisor::process_write(self.pid, data_ptr.cast::<libc::c_void>(), &self.data)?;

        // guess who will never know that this was a mmio read
        let is_totally_write = 1u8;
        let is_write_ptr: *mut u8 = unsafe { &mut ((*mmio_ptr).is_write) };
        hypervisor::process_write(
            self.pid,
            is_write_ptr.cast::<libc::c_void>(),
            &is_totally_write,
        )?;

        Ok(())
    }
}

impl fmt::Display for MmioRw {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_write {
            write!(
                f,
                "MmioRw{{ write {:?} to guest phys @ {:#x} }}",
                self.data(),
                self.addr
            )
        } else {
            write!(
                f,
                "MmioRw{{ read {}b from guest phys @ {:#x} }}",
                self.len, self.addr
            )
        }
    }
}

/// Contains the state of the thread running a vcpu.
/// TODO in theory vcpus could change threads which they are run on
#[derive(Debug)]
struct Thread {
    ptthread: ptrace::Thread,
    vcpu_map: Mapping,
    is_running: bool,
    in_syscall: bool,
}

impl Thread {
    pub fn new(ptthread: ptrace::Thread, vcpu_map: Mapping) -> Thread {
        Thread {
            ptthread,
            is_running: false,
            in_syscall: false, // ptrace (in practice) never attaches to a process while it is in a syscall
            vcpu_map,
        }
    }

    pub fn toggle_in_syscall(&mut self) {
        self.in_syscall = !self.in_syscall;
    }

    /// Should be called before or during dropping a Thread
    pub fn prepare_detach(&self) -> Result<()> {
        if !self.is_running {
            // not interrupting because already stopped
            return Ok(());
        }
        self.ptthread.interrupt()?;
        // wait for thread to actually be interrupted
        loop {
            let status = try_with!(
                waitpid(self.ptthread.tid, None),
                "failed to waitpid on thread {}",
                self.ptthread.tid
            );
            if let WaitStatus::PtraceSyscall(pid)
            | WaitStatus::PtraceEvent(pid, Signal::SIGSTOP, _) = status
            {
                if pid == self.ptthread.tid {
                    break;
                }
            }
        }
        Ok(())
    }
}

/// TODO respect and handle newly spawned threads as well
pub struct KvmRunWrapper {
    process_idx: usize,
    threads: Vec<Thread>,
    process_group: Pid,
    owner: Option<ThreadId>,
}

impl Drop for KvmRunWrapper {
    fn drop(&mut self) {
        debug!("kvm run wrapper cleanup started");
        if let Err(e) = self.prepare_detach() {
            log::warn!("cannot drop KvmRunWrapper: {}", e);
        }
        debug!("kvm run wrapper cleanup finished");
    }
}

fn get_process_group(pid: Pid) -> Result<Pid> {
    let process_group = try_with!(getpgid(Some(pid)), "getppid failed");

    if getpgrp() == process_group {
        bail!("vmsh and hypervisor are in same process group. Are they sharing a terminal? This is not supported at the moment.")
    }
    Ok(process_group)
}

impl KvmRunWrapper {
    pub fn attach(pid: Pid, vcpu_maps: &[Mapping]) -> Result<KvmRunWrapper> {
        let (threads, process_idx) = try_with!(
            ptrace::attach_all_threads(pid),
            "cannot attach KvmRunWrapper to all threads of {} via ptrace",
            pid
        );
        let threads: Vec<Thread> = threads
            .into_iter()
            .map(|t| {
                let vcpu_map = vcpu_maps[0].clone(); // TODO support more than 1 cpu and respect remaps
                Thread::new(t, vcpu_map)
            })
            .collect();

        Ok(KvmRunWrapper {
            process_idx,
            threads,
            process_group: get_process_group(pid)?,
            owner: Some(current().id()),
        })
    }

    /// Should be called before or during dropping a `KvmRunWrapper`
    fn prepare_detach(&mut self) -> Result<()> {
        for thread in &self.threads {
            try_with!(
                thread.prepare_detach(),
                "cannot prepare thread {} for detaching",
                thread.ptthread.tid
            );
        }
        Ok(())
    }

    /// resume all threads and convert self into tracer.
    pub fn into_tracer(mut self) -> Result<Tracer> {
        let vcpu_map = self.threads[0].vcpu_map.clone();
        // Because we run the drop routine here,
        self.prepare_detach()?;
        // we may take all here, despite making KvmRunWrapper::drop ineffective.
        let threads = self.threads.split_off(0);
        let threads = threads.into_iter().map(|t| t.ptthread).collect();
        Ok(Tracer {
            process_idx: self.process_idx,
            threads,
            vcpu_map,
            owner: self.owner,
        })
    }

    pub fn from_tracer(tracer: Tracer) -> Result<Self> {
        let pid = tracer.main_thread().tid;
        let vcpu_map = tracer.vcpu_map;
        let threads: Vec<Thread> = tracer
            .threads
            .into_iter()
            .map(|t| Thread::new(t, vcpu_map.clone()))
            .collect();

        Ok(KvmRunWrapper {
            process_idx: tracer.process_idx,
            process_group: get_process_group(pid)?,
            threads,
            owner: tracer.owner,
        })
    }

    pub fn cont(&self) -> Result<()> {
        for thread in &self.threads {
            thread.ptthread.cont(None)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn main_thread(&self) -> &Thread {
        &self.threads[self.process_idx]
    }

    #[allow(dead_code)]
    fn main_thread_mut(&mut self) -> &mut Thread {
        &mut self.threads[self.process_idx]
    }

    fn check_owner(&self) -> Result<()> {
        if let Some(tracer) = self.owner {
            if current().id() != tracer {
                bail!(
                    "thread was attached from thread {:?}, we are thread {:?}",
                    self.owner,
                    current().id()
                );
            }
        } else {
            bail!("thread is not attached. Call `adopt()` first")
        }
        Ok(())
    }

    // TODO Err if third qemu thread terminates?
    pub fn wait_for_ioctl(&mut self) -> Result<Option<MmioRw>> {
        self.check_owner()?;
        for thread in &mut self.threads {
            if !thread.is_running {
                try_with!(thread.ptthread.syscall(), "ptrace.thread.syscall() failed");
                thread.is_running = true;
            }
        }
        let status = try_with!(self.waitpid(), "cannot waitpid");
        let mmio = try_with!(self.process_status(status), "cannot process status");

        Ok(mmio)
    }

    fn waitpid(&mut self) -> Result<WaitStatus> {
        loop {
            let status = try_with!(
                waitpid(
                    Some(Pid::from_raw(-self.process_group.as_raw())),
                    Some(nix::sys::wait::WaitPidFlag::__WALL)
                ),
                "cannot wait for ioctl syscall"
            );
            if let Some(pid) = status.pid() {
                let res = self
                    .threads
                    .iter_mut()
                    .find(|thread| thread.ptthread.tid == pid);
                if let Some(mut thread) = res {
                    thread.is_running = false;
                    return Ok(status);
                }
            }
        }
    }

    fn process_status(&mut self, status: WaitStatus) -> Result<Option<MmioRw>> {
        match status {
            WaitStatus::PtraceSyscall(pid) => {
                return self.stopped(pid);
            }
            WaitStatus::Exited(tid, status) => {
                warn!("thread {} exited with: {}", tid, status);
                self.drop_thread(tid);
            }
            _ => {}
        }
        Ok(None)
    }

    fn drop_thread(&mut self, tid: Pid) {
        let idx = self
            .threads
            .iter()
            .position(|t| t.ptthread.tid == tid)
            .unwrap_or_else(|| {
                panic!(
                    "BUG! must not drop threads which are not present (tid={})",
                    tid
                )
            });
        // remove and shift others to left
        self.threads.remove(idx);
        // shift idx if it was shifted
        if idx < self.process_idx {
            self.process_idx -= 1;
        }
    }

    fn stopped(&mut self, pid: Pid) -> Result<Option<MmioRw>> {
        let thread: &mut Thread = match self
            .threads
            .iter_mut()
            .find(|thread| thread.ptthread.tid == pid)
        {
            Some(t) => t,
            None => bail!("received stop for unkown process: {}", pid),
        };

        let regs = try_with!(thread.ptthread.getregs(), "cannot syscall results");
        // TODO check for matching ioctlfd
        let (syscall_nr, _ioctl_fd, ioctl_request, _, _, _, _) = regs.get_syscall_params();
        // SYS_ioctl = 16
        if syscall_nr != libc::SYS_ioctl as u64 {
            return Ok(None);
        }

        // TODO check vcpufd and save a mapping for active syscalls from thread to cpu to
        // support multiple cpus
        thread.toggle_in_syscall();
        // KVM_RUN = 0xae80 = ioctl_io_nr!(KVM_RUN, KVMIO, 0x80)
        if ioctl_request != ioctls::KVM_RUN() {
            return Ok(None);
        }

        if thread.in_syscall {
            trace!("kvm-run enter {}", pid);
            return Ok(None);
        } else {
            trace!("kvm-run exit {}", pid);
            let ret = regs.syscall_ret();
            if ret != 0 {
                log::warn!(
                    "wrap_syscall: ioctl(KVM_RUN) failed: {}",
                    Errno::from_i32(ret as i32)
                );
                // hope that hypervisor handles it correctly
                return Ok(None);
            }
        }

        // fulfilled precondition: ioctl(KVM_RUN) just returned
        let map_ptr = thread.vcpu_map.start as *const kvm_bindings::kvm_run;
        let kvm_run: kvm_bindings::kvm_run =
            hypervisor::process_read(pid, map_ptr.cast::<libc::c_void>())?;
        let mmio = MmioRw::from(&kvm_run, thread.ptthread.tid, thread.vcpu_map.clone());

        Ok(mmio)
    }

    fn _check_siginfo(thread: &Thread) -> Result<()> {
        let siginfo = try_with!(
            nix::sys::ptrace::getsiginfo(thread.ptthread.tid),
            "cannot getsiginfo"
        );
        if (siginfo.si_code == libc::SIGTRAP) || (siginfo.si_code == (libc::SIGTRAP | 0x80)) {
            trace!("siginfo.si_code true: {:#x}", siginfo.si_code);
            return Ok(());
        } else {
            trace!("siginfo.si_code false: {:#x}", siginfo.si_code);
        }
        Ok(())
    }
}
