use crate::tracer::Tracer;
use kvm_bindings as kvmb;
use log::*;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::try_with;
use std::fmt;

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

    pub fn from(kvm_run: &kvmb::kvm_run, pid: Pid, vcpu_map: Mapping) -> Option<MmioRw> {
        match kvm_run.exit_reason {
            kvmb::KVM_EXIT_MMIO => {
                // Safe because the exit_reason (which comes from the kernel) told us which
                // union field to use.
                let mmio: &MmioRwRaw = unsafe { &kvm_run.__bindgen_anon_1.mmio };
                Some(MmioRw::new(&mmio, pid, vcpu_map))
            }
            _ => None,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }

    fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    /// # Safety of the tracee
    ///
    /// Do not run this function when the traced process has continued since
    /// the last KvmRunWrapper.wait_for_ioctl()! Additionally it assumes that the last
    /// wait_for_ioctl() has returned with Some(_)!
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
        hypervisor::process_write(self.pid, data_ptr as *mut libc::c_void, &self.data)?;

        // guess who will never know that this was a mmio read
        let is_totally_write = 1u8;
        let is_write_ptr: *mut u8 = unsafe { &mut ((*mmio_ptr).is_write) };
        hypervisor::process_write(
            self.pid,
            is_write_ptr as *mut libc::c_void,
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
                "MmioRw{{ write {:?} to guest phys @ 0x{:x} }}",
                self.data(),
                self.addr
            )
        } else {
            write!(
                f,
                "MmioRw{{ read {}b from guest phys @ 0x{:x} }}",
                self.len, self.addr
            )
        }
    }
}

/// Contains the state of the thread running a vcpu.
/// TODO in theory vcpus could change threads which they are run on
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
            match status {
                WaitStatus::PtraceEvent(pid, _signal, event) => {
                    if pid != self.ptthread.tid {
                        continue;
                    }
                    if event == libc::PTRACE_EVENT_STOP {
                        break;
                    }
                }
                WaitStatus::Stopped(pid, _signal) => {
                    if pid != self.ptthread.tid {
                        continue;
                    }
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// TODO respect and handle newly spawned threads as well
pub struct KvmRunWrapper {
    process_idx: usize,
    threads: Vec<Thread>,
}

impl Drop for KvmRunWrapper {
    fn drop(&mut self) {
        if let Err(e) = self.prepare_detach() {
            log::warn!("cannot drop KvmRunWrapper: {}", e);
        }
    }
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
        })
    }

    /// Should be called before or during dropping a KvmRunWrapper
    pub fn prepare_detach(&mut self) -> Result<()> {
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
        })
    }

    pub fn from_tracer(tracer: Tracer) -> Result<Self> {
        let vcpu_map = tracer.vcpu_map;
        let threads: Vec<Thread> = tracer
            .threads
            .into_iter()
            .map(|t| Thread::new(t, vcpu_map.clone()))
            .collect();

        Ok(KvmRunWrapper {
            process_idx: tracer.process_idx,
            threads,
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

    // TODO Err if third qemu thread terminates?
    pub fn wait_for_ioctl(&mut self) -> Result<Option<MmioRw>> {
        for thread in &mut self.threads {
            if !thread.is_running {
                thread.ptthread.syscall()?;
                thread.is_running = true;
            }
        }
        let status = self.waitpid()?;
        let mmio = self.process_status(status)?;

        Ok(mmio)
    }

    /// busy polling on all thread tids
    fn waitpid(&mut self) -> Result<WaitStatus> {
        // Options to waitpid on many pids at the same time:
        //
        // - wait with multiple threads. Events into queue and poll on queue.
        //
        // - waitpid(WNOHANG): async waitpid, busy polling
        //
        // - linux waitid() P_PIDFD pidfd_open(): maybe (e)poll() on this fd? dunno
        //
        // - waitpid(pid = -pgid): Use linux default flag of __WALL: wait for main_thread and all
        //   kinds of children. To wait for all children, use -pgid.
        //   pid = -self.main_thread().tid fails with ECHILD though because it requires pgid (not
        //   parent id):
        //   setpgid(): waitpid on the pgid. Grouping could destroy existing Hypervisor groups and
        //   requires all group members to be in the same session (whatever that means). Also if
        //   the group owner (pid==pgid) dies, the enire group orphans (will it be killed as
        //   zombies?)
        //   => sounds a bit dangerous, doesn't it?
        loop {
            for thread in &mut self.threads {
                let status = try_with!(
                    waitpid(
                        thread.ptthread.tid,
                        Some(nix::sys::wait::WaitPidFlag::WNOHANG)
                    ),
                    "cannot wait for ioctl syscall"
                );
                if WaitStatus::StillAlive != status {
                    thread.is_running = false;
                    return Ok(status);
                }
            }
        }
    }

    fn process_status(&mut self, status: WaitStatus) -> Result<Option<MmioRw>> {
        match status {
            WaitStatus::PtraceEvent(_, _, _) => {
                bail!("got unexpected ptrace event")
            }
            WaitStatus::PtraceSyscall(_) => {
                bail!("got unexpected ptrace syscall event")
            }
            WaitStatus::StillAlive => {
                bail!("got unexpected still-alive waitpid() event")
            }
            WaitStatus::Continued(_) => {
                warn!("WaitStatus::Continued");
            } // noop
            WaitStatus::Stopped(pid, signal) => {
                return self.stopped(pid, &signal);
            }
            WaitStatus::Exited(_, status) => bail!("process exited with: {}", status),
            WaitStatus::Signaled(_, signal, _) => {
                bail!("process was stopped by signal: {}", signal)
            }
        }
        Ok(None)
    }

    fn stopped(&mut self, pid: Pid, _signal: &Signal) -> Result<Option<MmioRw>> {
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
            if regs.syscall_ret() != 0 {
                log::warn!("wrap_syscall: ioctl(KVM_RUN) failed.");
                // hope that hypervisor handles it correctly
                return Ok(None);
            }
        }

        // fulfilled precondition: ioctl(KVM_RUN) just returned
        let map_ptr = thread.vcpu_map.start as *const kvm_bindings::kvm_run;
        let kvm_run: kvm_bindings::kvm_run =
            hypervisor::process_read(pid, map_ptr as *const libc::c_void)?;
        let mmio = MmioRw::from(&kvm_run, thread.ptthread.tid, thread.vcpu_map.clone());

        Ok(mmio)
    }

    fn _check_siginfo(&self, thread: &Thread) -> Result<()> {
        let siginfo = try_with!(
            nix::sys::ptrace::getsiginfo(thread.ptthread.tid),
            "cannot getsiginfo"
        );
        if (siginfo.si_code == libc::SIGTRAP) || (siginfo.si_code == (libc::SIGTRAP | 0x80)) {
            trace!("siginfo.si_code true: 0x{:x}", siginfo.si_code);
            return Ok(());
        } else {
            trace!("siginfo.si_code false: 0x{:x}", siginfo.si_code);
        }
        Ok(())
    }
}
