pub mod attach;
pub mod coredump;
pub mod cpu;
pub mod device;
pub mod elf;
pub mod gdb_break;
pub mod inject_syscall;

pub mod inspect;
pub mod kvm;
pub mod page_math;
pub mod proc;
pub mod ptrace;
/// This module provides a safe wrapper for ptrace(PTRACE_GET_SYSCALL_INFO). This function exists
/// since linux 5.3 but changed the binary layout of its output (struct ptrace_syscall_info)
/// between 5.10 and 5.11.
///
/// Note:
///
/// While SyscallInfo could provide amazing information in its `op` field, this field is (as of
/// v5.4.106) always empty (SyscallOp::None) - which makes this function kind of useless.
pub mod ptrace_syscall_info;
pub mod result;
pub mod wrap_syscall;
