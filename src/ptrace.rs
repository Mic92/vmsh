use crate::result::Result;
use libc::{c_long, c_void, user_regs_struct};
use nix::sys::ptrace::{self, AddressType};
use nix::unistd::Pid;
use simple_error::try_with;

pub struct Thread {
    tid: Pid,
}

impl Thread {
    pub fn setregs(&self, regs: user_regs_struct) -> Result<()> {
        Ok(try_with!(
            ptrace::setregs(self.tid, regs),
            "cannot set registers with ptrace"
        ))
    }

    pub fn getregs(&self) -> Result<user_regs_struct> {
        Ok(try_with!(
            ptrace::getregs(self.tid),
            "cannot get registers with ptrace"
        ))
    }

    pub fn syscall(&self) -> Result<()> {
        Ok(try_with!(
            ptrace::syscall(self.tid, None),
            "cannot set break on syscall with ptrace"
        ))
    }

    pub fn read(&self, addr: AddressType) -> Result<c_long> {
        Ok(try_with!(
            ptrace::read(self.tid, addr),
            "cannot read with ptrace"
        ))
    }

    pub fn write(&self, addr: AddressType, data: *mut c_void) -> Result<()> {
        unsafe {
            Ok(try_with!(
                ptrace::write(self.tid, addr, data),
                "cannot write with ptrace"
            ))
        }
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
