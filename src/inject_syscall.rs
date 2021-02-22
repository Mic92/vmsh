use libc::SYS_ioctl;
use libc::{c_int, c_long, c_ulong, c_void, pid_t};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::try_with;
use std::fs;
use std::mem;
use std::os::unix::prelude::RawFd;
use std::path::PathBuf;

use crate::cpu::{self, Regs};
use crate::ptrace;
use crate::result::Result;

pub struct Process {
    process_idx: usize,
    saved_regs: Regs,
    saved_text: c_long,
    threads: Vec<ptrace::Thread>,
}

pub fn attach(pid: Pid) -> Result<Process> {
    let dir = PathBuf::from("/proc")
        .join(pid.as_raw().to_string())
        .join("tasks");
    let threads_dir = try_with!(fs::read_dir(&dir), "failed to open directory /proc/self/ns");
    let mut process_idx = 0;

    let threads = threads_dir
        .enumerate()
        .map(|(i, thread_name)| {
            let entry = try_with!(thread_name, "failed to read directory {}", dir.display());
            let _file_name = entry.file_name();
            let file_name = _file_name.to_str().unwrap();
            let raw_tid = try_with!(file_name.parse::<pid_t>(), "invalid tid {}", file_name);
            let tid = Pid::from_raw(raw_tid);
            if tid == pid {
                process_idx = i;
            }
            ptrace::attach(tid)
        })
        .collect::<Result<Vec<_>>>()?;

    let saved_regs = try_with!(
        threads[process_idx].getregs(),
        "cannot get registers for main process"
    );
    let ip = saved_regs.ip();
    let saved_text = try_with!(
        threads[process_idx].read(ip as *mut c_void),
        "cannot get text for main process"
    );
    try_with!(
        threads[process_idx].write(ip as *mut c_void, cpu::SYSCALL_TEXT as *mut c_void),
        "cannot patch syscall instruction"
    );

    Ok(Process {
        process_idx,
        saved_regs,
        saved_text,
        threads,
    })
}

macro_rules! syscall_args {
    ($regs:expr, $nr:expr) => {
        ($regs).prepare_syscall(&[$nr, 0, 0, 0, 0, 0, 0])
    };

    ($regs:expr, $nr:expr, $a1:expr) => {
        ($regs).prepare_syscall(&[$nr, $a1 as c_ulong, 0, 0, 0, 0, 0])
    };

    ($regs:expr, $nr:expr, $a1:expr, $a2:expr) => {
        ($regs).prepare_syscall(&[$nr, $a1 as c_ulong, $a2 as c_ulong, 0, 0, 0, 0])
    };

    ($regs:expr, $nr:expr, $a1:expr, $a2:expr, $a3:expr) => {
        $regs.prepare_syscall(&[$nr, $a1 as c_ulong, $a2 as c_ulong, $a3 as c_ulong, 0, 0, 0])
    };

    ($regs:expr, $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr) => {
        $regs.prepare_syscall(&[
            $nr,
            $a1 as c_ulong,
            $a2 as c_ulong,
            $a3 as c_ulong,
            $a4 as c_ulong,
            0,
            0,
        ])
    };

    ($regs:expr, $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr) => {
        $regs.prepare_syscall(&[
            $nr,
            $a1 as c_ulong,
            $a2 as c_ulong,
            $a3 as c_ulong,
            $a4 as c_ulong,
            $a5 as c_ulong,
            0,
        ])
    };

    ($regs:expr, $nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr, $a6:expr) => {
        $regs.prepare_syscall(&[
            $nr,
            $a1 as c_ulong,
            $a2 as c_ulong,
            $a3 as c_ulong,
            $a4 as c_ulong,
            $a5 as c_ulong,
            $a6 as c_ulong,
        ])
    };
}

impl Process {
    pub fn ioctl(&self, fd: RawFd, request: c_ulong, arg: c_int) -> Result<c_int> {
        let args = syscall_args!(
            self.saved_regs,
            SYS_ioctl as c_ulong,
            fd as c_ulong,
            request,
            arg
        );

        return self.syscall(&args).map(|v| v as c_int);
    }

    fn syscall(&self, regs: &Regs) -> Result<isize> {
        try_with!(
            self.main_thread().setregs(regs),
            "cannot set system call args"
        );
        // FIXME: on arm we would need PTRACE_SET_SYSCALL
        loop {
            try_with!(self.main_thread().syscall(), "cannot run syscall in thread");

            let mut status = try_with!(waitpid(self.main_thread().tid, None), "waitpid failed");

            // why do we need this one?
            if let WaitStatus::Stopped(_, Signal::SIGTRAP) = status {
                try_with!(self.main_thread().syscall(), "cannot run syscall in thread");
                status = try_with!(waitpid(self.main_thread().tid, None), "waitpid failed");
            }

            match status {
                WaitStatus::PtraceEvent(_, _, _) => {
                    let result_regs =
                        try_with!(self.main_thread().getregs(), "cannot syscall results");
                    assert!(
                        self.saved_regs.ip()
                            == result_regs.ip() - mem::size_of_val(&cpu::SYSCALL_TEXT) as u64
                    );
                    try_with!(
                        self.main_thread().setregs(&self.saved_regs),
                        "failed to restore old syscalls"
                    );
                    return Ok(result_regs.syscall_ret() as isize);
                }
                WaitStatus::PtraceSyscall(_) => {
                    return bail!("got unexpected ptrace syscall event")
                }
                WaitStatus::StillAlive => {
                    return bail!("got unexpected still-alive waitpid() event")
                }
                WaitStatus::Continued(_) => {} // noop
                WaitStatus::Stopped(_, signal) => {
                    // should not happen usually, so log it
                    return bail!("process was stopped by by signal: {}", signal);
                }
                WaitStatus::Exited(_, status) => return bail!("process exited with: {}", status),
                WaitStatus::Signaled(_, signal, coredumped) => {
                    return bail!("process was stopped by signal: {}", signal)
                }
            }
        }
    }

    fn main_thread(&self) -> &ptrace::Thread {
        &self.threads[self.process_idx]
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.main_thread().write(
            self.saved_regs.ip() as *mut c_void,
            self.saved_text as *mut c_void,
        );
        let _ = self.main_thread().setregs(&self.saved_regs);
    }
}
