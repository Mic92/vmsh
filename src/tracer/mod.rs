pub mod inject_syscall;
pub mod proc;
pub mod ptrace;
/// This module provides a safe wrapper for ptrace(PTRACE_GET_SYSCALL_INFO) but only for linux
/// v5.3-v5.10. This function exists since linux 5.3 but changed the binary layout of its output
/// (struct ptrace_syscall_info) between 5.10 and 5.11.
///
/// Note:
///
/// While SyscallInfo could provide amazing information in its `op` field, this field is (as of
/// v5.4.106) always empty (SyscallOp::None) - which makes this function kind of useless.
pub mod ptrace_syscall_info;
pub mod wrap_syscall;

use proc::Mapping;

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
