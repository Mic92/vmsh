use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::try_with;
use nix::sys::wait::WaitPidFlag;
use nix::sys::mman::ProtFlags;

use crate::cpu::{self, Regs};
use crate::ptrace;
use crate::result::Result;
use crate::kvm::hypervisor::MonMem;

struct Thread {
    ptthread: ptrace::Thread,
    is_running: bool,
}

impl Thread {
    pub fn new(ptthread: ptrace::Thread) -> Thread {
        Thread {
            ptthread,
            is_running: false,
        }
    }
}

/// TODO respect and handle newly spawned threads as well
pub struct SignalCatcher {
    process_idx: usize,
    threads: Vec<Thread>,
}

impl SignalCatcher {
    pub fn attach(pid: Pid) -> Result<SignalCatcher> {
        //let threads = vec![try_with!(ptrace::attach(pid), "foo")];
        //let process_idx = 0;
        let (threads, process_idx) = try_with!(
            ptrace::attach_all_threads(pid),
            "cannot attach KvmRunWrapper to all threads of {} via ptrace",
            pid
        );
        let threads = threads.into_iter().map(|t| Thread::new(t)).collect();
        let sc = SignalCatcher {
            process_idx,
            threads,
        };
        sc.cont()?;
        Ok(sc)
    }

    pub fn cont(&self) -> Result<()> {
        for thread in &self.threads {
            thread.ptthread.cont(None)?;
        }
        Ok(())
    }

    fn main_thread(&self) -> &Thread {
        &self.threads[self.process_idx]
    }

    fn main_thread_mut(&mut self) -> &mut Thread {
        &mut self.threads[self.process_idx]
    }

    // -> Err if third qemu thread terminates
    pub fn wait_for_signal<T: Copy>(&mut self, monmem: &MonMem<T>) -> Result<()> {
        println!("signal");
        //for thread in &mut self.threads {
        //    if !thread.is_running {
        //        thread.ptthread.syscall()?;
        //        thread.is_running = true;
        //    }
        //}
        //println!("syscall {}", self.threads[0].tid);
        //if !self.main_thread().is_running {
        //    try_with!(self.main_thread().ptthread.syscall(), "fii");
        //    self.main_thread_mut().is_running = true;
        //}

        //
        // Further options to waitpid on many Ps at the same time:
        //
        // - waitpid(WNOHANG): async waitpid, busy polling
        //
        // - linux waitid() P_PIDFD pidfd_open(): maybe (e)poll() on this fd? dunno
        //
        // - setpgid(): waitpid on the pgid. Grouping could destroy existing Hypervisor groups and
        //   requires all group members to be in the same session (whatever that means). Also if
        //   the group owner (pid==pgid) dies, the enire group orphans (will it be killed as
        //   zombies?)
        //   => sounds a bit dangerous, doesn't it?

        // use linux default flag of __WALL: wait for main_thread and all kinds of children
        // to wait for all children, use -gid
        //println!("wait {}", self.threads[0].tid);
        println!("waitpid");
        let status = self.waitpid_busy()?;
        //let status = try_with!(
        //waitpid(Pid::from_raw(-self.main_thread().tid.as_raw()), None),
        //"cannot wait for ioctl syscall"
        //);
        self.process_status(status, monmem)?;

        Ok(())
    }

    fn waitpid_busy(&mut self) -> Result<WaitStatus> {
        loop {
            for thread in &mut self.threads {
                let status = try_with!(
                    waitpid(
                        thread.ptthread.tid,
                        Some(WaitPidFlag::WNOHANG) // | WaitPidFlag::WSTOPPED)
                    ),
                    "cannot wait for ioctl syscall"
                );
                if WaitStatus::StillAlive != status {
                    println!("waipid still alive: {}", thread.ptthread.tid);
                    thread.is_running = false;
                    return Ok(status);
                }
            }
        }
    }

    fn process_status<T: Copy>(&mut self, status: WaitStatus, monmem: &MonMem<T>) -> Result<()> {
        match status {
            WaitStatus::PtraceEvent(_, _, _) => {
                println!("got unexpected ptrace event")
            }
            WaitStatus::PtraceSyscall(_) => {
                println!("got unexpected ptrace syscall event")
            }
            WaitStatus::StillAlive => {
                println!("got unexpected still-alive waitpid() event")
            }
            WaitStatus::Continued(_) => {
                println!("WaitStatus::Continued");
            } // noop
            //WaitStatus::Stopped(_, Signal::SIGTRAP) => {
            //let regs =
            //try_with!(self.main_thread().getregs(), "cannot syscall results");
            //println!("syscall: eax {:x} ebx {:x}", regs.rax, regs.rbx);

            //return Ok(());
            //}
            WaitStatus::Stopped(pid, signal) => {
                println!("process {} was stopped by signal: {}", pid, signal);
                let thread: &ptrace::Thread = match self
                   .threads
                   .iter()
                   .find(|thread| thread.ptthread.tid == pid)
                {
                   Some(t) => &t.ptthread,
                   None => bail!("received stop for unkown process: {}", pid),
                };

                //let regs = try_with!(thread.getregs(), "cannot syscall results");
                //println!(
                //    "syscall: eax {:x} ebx {:x} cs {:x}",
                //    regs.rax, regs.rbx, regs.cs
                //);
                let siginfo = try_with!(
                   nix::sys::ptrace::getsiginfo(pid),
                   "cannot getsiginfo"
                );
                println!("siginfo: addr {:?} value {:?}", unsafe { siginfo.si_addr() }, unsafe { siginfo.si_value() } );
                //if (siginfo.si_code == libc::SIGTRAP) || (siginfo.si_code == (libc::SIGTRAP | 0x80))
                //{
                //    println!("siginfo.si_code true: 0x{:x}", siginfo.si_code);
                //    return Ok(());
                //} else {
                //    println!("siginfo.si_code false: 0x{:x}", siginfo.si_code);
                //    //try_with!(nix::sys::ptrace::syscall(self.main_thread().tid, None), "cannot ptrace::syscall");
                //}
                ////bail!("process was stopped by by signal: {}", signal);
                ////self.main_thread().cont(Some(signal))?;
                ////return self.await_syscall();

                //monmem.set_prot(ProtFlags::all())?;
                thread.cont(Some(signal))?;
            }
            WaitStatus::Exited(tid, status) => {
                println!("process {} exited with: {}", tid, status);
                let thread_idx = match self
                    .threads
                    .iter()
                    .position(|thread| thread.ptthread.tid == tid)
                {
                    Some(t) => t,
                    None => bail!("received stop for unkown thread: {}", tid),
                };
                self.threads.remove(thread_idx);
            }
            WaitStatus::Signaled(tid, signal, _) => {
                println!("process {} was stopped (signaled) by signal: {}", tid, signal); // <-- SIGUSR1?
                let siginfo = try_with!(
                   nix::sys::ptrace::getsiginfo(tid),
                   "cannot getsiginfo"
                );
                println!("siginfo: addr {:?} value {:?}", unsafe { siginfo.si_addr() }, unsafe { siginfo.si_value() } );
            }
        }
        Ok(())
    }
}
