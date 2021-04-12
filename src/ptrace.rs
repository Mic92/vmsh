use libc::{c_long, c_void};
use nix::errno::Errno;
use nix::sys::ptrace::{self, AddressType, Request, RequestType};
use nix::unistd::Pid;
use simple_error::try_with;
use std::{mem, ptr};

use crate::cpu::Regs;
use crate::elf;
use crate::result::Result;

pub struct Thread {
    pub tid: Pid,
}

/// Get user registers, as with `ptrace(PTRACE_GETREGS, ...)`
fn getregs(pid: Pid) -> nix::Result<Regs> {
    ptrace_get_data::<Regs>(Request::PTRACE_GETREGSET, pid)
}

/// Set user registers, as with `ptrace(PTRACE_SETREGS, ...)`
fn setregs(pid: Pid, regs: &Regs) -> nix::Result<()> {
    let iov = libc::iovec {
        iov_base: regs as *const _ as *mut _,
        iov_len: mem::size_of::<Regs>(),
    };
    let res = unsafe {
        libc::ptrace(
            Request::PTRACE_SETREGSET as RequestType,
            libc::pid_t::from(pid),
            elf::NT_PRSTATUS,
            &iov,
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
    let iov = libc::iovec {
        iov_base: data.as_mut_ptr() as *mut _,
        iov_len: mem::size_of::<T>(),
    };
    let res = unsafe {
        libc::ptrace(
            request as RequestType,
            libc::pid_t::from(pid),
            elf::NT_PRSTATUS,
            &iov,
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

    pub fn syscall(&self) -> Result<()> {
        try_with!(
            ptrace::syscall(self.tid, None),
            "cannot set break on syscall with ptrace"
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

pub fn attach(tid: Pid) -> Result<Thread> {
    try_with!(ptrace::attach(tid), "cannot attach to process");
    Ok(Thread { tid })
}

impl Drop for Thread {
    fn drop(&mut self) {
        let _ = ptrace::detach(self.tid, None);
    }
}
