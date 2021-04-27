use libc::{c_long, c_void, pid_t};
use nix::errno::Errno;
use nix::sys::ptrace::{self, AddressType, Request, RequestType};
use nix::sys::wait::waitpid;
use nix::sys::wait::WaitPidFlag;
use nix::unistd::Pid;
use simple_error::try_with;
use std::fs;
use std::{mem, ptr};

use crate::cpu::Regs;
use crate::result::Result;
use crate::tracer::proc;
use crate::tracer::ptrace_syscall_info::{get_syscall_info, SyscallInfo};

pub struct Thread {
    pub tid: Pid,
}

/// Get user registers, as with `ptrace(PTRACE_GETREGS, ...)`
fn getregs(pid: Pid) -> nix::Result<Regs> {
    ptrace_get_data::<Regs>(Request::PTRACE_GETREGS, pid)
}

/// Set user registers, as with `ptrace(PTRACE_SETREGS, ...)`
fn setregs(pid: Pid, regs: &Regs) -> nix::Result<()> {
    let res = unsafe {
        libc::ptrace(
            Request::PTRACE_SETREGS as RequestType,
            libc::pid_t::from(pid),
            ptr::null_mut::<c_void>(),
            regs as *const _ as *const c_void,
        )
    };
    Errno::result(res).map(drop)
}

/// Stop tracee while being attached, as with `ptrace(PTRACE_INTERRUPT, ...)`
fn interrupt(pid: Pid) -> nix::Result<()> {
    let res = unsafe {
        libc::ptrace(
            Request::PTRACE_INTERRUPT as RequestType,
            libc::pid_t::from(pid),
            ptr::null_mut::<c_void>(),
            ptr::null_mut::<c_void>(),
        )
    };
    Errno::result(res).map(drop)
}

/// Function for ptrace requests that return values from the data field.
/// Some ptrace get requests populate structs or larger elements than `c_long`
/// and therefore use the data field to return values. This function handles these
/// requests.
fn ptrace_get_data<T>(request: Request, pid: Pid) -> nix::Result<T> {
    let mut data = mem::MaybeUninit::uninit();
    let res = unsafe {
        libc::ptrace(
            request as RequestType,
            libc::pid_t::from(pid),
            ptr::null_mut::<T>(),
            data.as_mut_ptr() as *const _ as *const c_void,
        )
    };
    Errno::result(res)?;
    Ok(unsafe { data.assume_init() })
}

impl Thread {
    pub fn setregs(&self, regs: &Regs) -> Result<()> {
        try_with!(setregs(self.tid, regs), "cannot set registers with ptrace");
        Ok(())
    }

    pub fn getregs(&self) -> Result<Regs> {
        Ok(try_with!(
            getregs(self.tid),
            "cannot get registers with ptrace"
        ))
    }

    pub fn interrupt(&self) -> Result<()> {
        try_with!(
            interrupt(self.tid),
            "cannot stop/interrupt tracee with ptrace"
        );
        Ok(())
    }

    pub fn syscall(&self) -> Result<()> {
        try_with!(
            ptrace::syscall(self.tid, None),
            "cannot set break on syscall with ptrace"
        );
        Ok(())
    }

    pub fn syscall_info(&self) -> Result<SyscallInfo> {
        let info = try_with!(
            get_syscall_info(self.tid),
            "cannot get syscall info with ptrace"
        );
        Ok(info)
    }

    pub fn cont(&self, sig: Option<nix::sys::signal::Signal>) -> Result<()> {
        try_with!(
            ptrace::cont(self.tid, sig),
            "cannot continue tracee with ptrace"
        );
        Ok(())
    }

    pub fn read(&self, addr: AddressType) -> Result<c_long> {
        Ok(try_with!(
            ptrace::read(self.tid, addr),
            "cannot read with ptrace"
        ))
    }

    /// # Safety
    ///
    /// The `data` argument is passed directly to `ptrace(2)`. Read that man page for guidance.
    pub unsafe fn write(&self, addr: AddressType, data: *mut c_void) -> Result<()> {
        try_with!(
            ptrace::write(self.tid, addr, data),
            "cannot write with ptrace"
        );
        Ok(())
    }
}

pub fn attach_seize(tid: Pid) -> Result<Thread> {
    // seize seems to be more modern and versatile than `ptrace::attach()`: continue, stop and
    // detach from tracees at (almost) any time
    try_with!(
        ptrace::seize(tid, ptrace::Options::empty()),
        "cannot seize the process"
    );
    try_with!(interrupt(tid), "cannot interrupt/stop the tracee");
    Ok(Thread { tid })
}

pub fn attach_all_threads(pid: Pid) -> Result<(Vec<Thread>, usize)> {
    let dir = proc::pid_path(pid).join("task");
    let threads_dir = try_with!(
        fs::read_dir(&dir),
        "failed to open directory {}",
        dir.display()
    );
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
            let thread = attach_seize(tid);
            try_with!(waitpid(tid, Some(WaitPidFlag::WSTOPPED)), "waitpid failed");
            thread
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((threads, process_idx))
}

impl Drop for Thread {
    fn drop(&mut self) {
        if let Err(e) = ptrace::detach(self.tid, None) {
            log::warn!("Cannot ptrace::detach from {}", self.tid);
        }
    }
}
