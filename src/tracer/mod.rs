use libc::{c_int, c_long, c_ulong, c_void, off_t, pid_t, size_t, ssize_t, SYS_munmap};
use libc::{SYS_getpid, SYS_ioctl, SYS_mmap};
use nix::unistd::Pid;
use simple_error::bail;

use crate::inject_syscall::Process as InjectSyscall;
use crate::ptrace;
use crate::result::Result;
//#[macro_use] use crate::syscall_args;
use crate::inject_syscall;
use crate::proc::Mapping;
use crate::wrap_syscall::{KvmRunWrapper, MmioRw};

pub struct Tracer {
    pub process_idx: usize,
    pub threads: Vec<ptrace::Thread>,
    pub vcpu_map: Mapping, // TODO support multiple cpus
}

impl Tracer {
    #[allow(dead_code)]
    fn main_thread(&self) -> &ptrace::Thread {
        &self.threads[self.process_idx]
    }

    #[allow(dead_code)]
    fn main_thread_mut(&mut self) -> &mut ptrace::Thread {
        &mut self.threads[self.process_idx]
    }
}
