use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::try_with;

use crate::cpu::{self, Regs};
use crate::ptrace;
use crate::result::Result;

pub struct KvmRunWrapper {
    process_idx: usize,
    threads: Vec<ptrace::Thread>,
}

impl KvmRunWrapper {
    pub fn attach(pid: Pid) -> Result<KvmRunWrapper> {
        let (threads, process_idx) = try_with!(
            ptrace::attach_all_threads(pid),
            "cannot attach KvmRunWrapper to all threads of {} via ptrace",
            pid
        );
        Ok(KvmRunWrapper {
            process_idx,
            threads,
        })
    }

    fn main_thread(&self) -> &ptrace::Thread {
        &self.threads[self.process_idx]
    }

    pub fn wait_for_ioctl(&self) -> Result<()> {
        //for thread in &self.threads {
            //thread.syscall()?;
        //}
        self.main_thread().syscall()?;

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

        // use linux default flag of __WALL: wait for main_thread and all its children
        let status = try_with!(
            waitpid(self.main_thread().tid, None),
            "cannot wait for ioctl syscall"
        );
        self.process_status(status)?;

        Ok(())
    }

    fn process_status(&self, status: WaitStatus) -> Result<()> {
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
                println!("WaitStatus::Continued");
            } // noop
            //WaitStatus::Stopped(_, Signal::SIGTRAP) => {
            //let regs =
            //try_with!(self.main_thread().getregs(), "cannot syscall results");
            //println!("syscall: eax {:x} ebx {:x}", regs.rax, regs.rbx);

            //return Ok(());
            //}
            WaitStatus::Stopped(pid, signal) => {
                println!("process {} was stopped by by signal: {}", pid, signal);
                let thread: &ptrace::Thread =
                    match self.threads.iter().find(|thread| thread.tid == pid) {
                        Some(t) => &t,
                        None => bail!("received stop for unkown process: {}", pid),
                };

                let regs = try_with!(thread.getregs(), "cannot syscall results");
                println!(
                    "syscall: eax {:x} ebx {:x} cs {:x}",
                    regs.rax, regs.rbx, regs.cs
                );
                let siginfo = try_with!(
                    nix::sys::ptrace::getsiginfo(thread.tid),
                    "cannot getsiginfo"
                );
                if (siginfo.si_code == libc::SIGTRAP) || (siginfo.si_code == (libc::SIGTRAP | 0x80))
                {
                    println!("siginfo.si_code true: 0x{:x}", siginfo.si_code);
                    return Ok(());
                } else {
                    println!("siginfo.si_code false: 0x{:x}", siginfo.si_code);
                    //try_with!(nix::sys::ptrace::syscall(self.main_thread().tid, None), "cannot ptrace::syscall");
                }
                //bail!("process was stopped by by signal: {}", signal);
                //self.main_thread().cont(Some(signal))?;
                //return self.await_syscall();
            }
            WaitStatus::Exited(_, status) => bail!("process exited with: {}", status),
            WaitStatus::Signaled(_, signal, _) => {
                bail!("process was stopped by signal: {}", signal)
            }
        }
        Ok(())
    }
}
