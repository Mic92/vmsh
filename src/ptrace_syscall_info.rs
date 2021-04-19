use crate::result::Result;
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::try_with;
use std::mem::size_of;
use std::mem::MaybeUninit;

const PTRACE_GET_SYSCALL_INFO: u32 = 0x420e;

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
enum OpType {
    PTRACE_SYSCALL_INFO_NONE = 0,
    PTRACE_SYSCALL_INFO_ENTRY = 1,
    PTRACE_SYSCALL_INFO_EXIT = 2,
    PTRACE_SYSCALL_INFO_SECCOMP = 3,
    unknown = 4,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct Entry {
    nr: u64,
    args: [u64; 6],
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct Exit {
    rval: i64,
    is_error: u8,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct Seccomp {
    nr: u64,
    args: [u64; 6],
    ret_data: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
union RawData {
    entry: Entry,
    exit: Exit,
    seccomp: Seccomp,
}

/// equivalent to ptrace_syscall_info
#[repr(C)]
#[derive(Copy, Clone)]
pub struct RawInfo {
    op: OpType,
    arch: u32,
    instruction_pointer: u64,
    stack_pointer: u64,
    data: RawData,
}

/// See man ptrace (linux) for reference.
#[derive(Copy, Clone, Debug)]
pub struct SyscallInfo {
    pub arch: u32,
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub op: SyscallOp,
}

#[derive(Copy, Clone, Debug)]
pub enum SyscallOp {
    Entry {
        nr: u64,
        args: [u64; 6],
    },
    Exit {
        rval: i64,
        is_error: u8,
    },
    Seccomp {
        nr: u64,
        args: [u64; 6],
        ret_data: u32,
    },
    None,
}

fn parse_raw_data(info: RawInfo) -> Result<SyscallOp> {
    let op = unsafe {
        match info.op {
            OpType::PTRACE_SYSCALL_INFO_NONE => SyscallOp::None,
            OpType::PTRACE_SYSCALL_INFO_ENTRY => SyscallOp::Entry {
                nr: info.data.entry.nr,
                args: info.data.entry.args,
            },
            OpType::PTRACE_SYSCALL_INFO_EXIT => SyscallOp::Exit {
                rval: info.data.exit.rval,
                is_error: info.data.exit.is_error,
            },
            OpType::PTRACE_SYSCALL_INFO_SECCOMP => SyscallOp::Seccomp {
                nr: info.data.seccomp.nr,
                args: info.data.seccomp.args,
                ret_data: info.data.seccomp.ret_data,
            },
            _ => bail!("unknown ptrace_syscall_info.op: {:?}", info.op),
        }
    };

    Ok(op)
}

fn parse_raw_info(raw: RawInfo) -> Result<SyscallInfo> {
    let info = SyscallInfo {
        arch: raw.arch,
        instruction_pointer: raw.instruction_pointer,
        stack_pointer: raw.stack_pointer,
        op: parse_raw_data(raw)?,
    };
    Ok(info)
}

pub fn get_syscall_info(pid: Pid) -> Result<SyscallInfo> {
    let mut info = MaybeUninit::<RawInfo>::zeroed();
    // Safe, because the kernel writes at most size_of::<RawInfo>() bytes and at least `ret` bytes.
    // We check he has written size_of::<RawInfo>() bytes. We also allow him to omit the trailing
    // `data: RawData` field if he marks its absence in the op field, because in that case the
    // parser (`parse_raw_info()`) will ignore the data and never access it.
    let ret = unsafe {
        libc::ptrace(
            PTRACE_GET_SYSCALL_INFO,
            pid,
            size_of::<RawInfo>(),
            info.as_mut_ptr(),
        )
    };
    if ret <= 0 {
        bail!("ptrace get syscall info error: {}", ret);
    }
    let info = unsafe { info.assume_init() };
    if !((info.op == OpType::PTRACE_SYSCALL_INFO_NONE
        && size_of::<RawInfo>() - size_of::<RawData>() == ret as usize)
        || (size_of::<RawInfo>() == ret as usize))
    {
        bail!("ptrace wrote unexpected number of bytes");
    }
    let info = try_with!(
        parse_raw_info(info),
        "cannot understand ptrace(PTRACE_GET_SYSCALL_INFO) response"
    );
    Ok(info)
}

#[cfg(test)]
mod test {
    #[test]
    fn assert_struct_sizes() {
        use super::*;
        //assert_eq!(size_of::<RawInfo>(), 0); // for linux <= v5.2

        assert_eq!(size_of::<RawInfo>(), 88); // for linux <= v5.10

        //assert_eq!(size_of::<RawInfo>(), 84); // for linux >= v5.11
    }

    #[test]
    fn check_linux_version() {
        // TODO add a build.rs script which uses
        // https://docs.rs/linux-version/0.1.1/linux_version/
        // to detect linux version and enables the corresponding feature via
        // https://doc.rust-lang.org/cargo/reference/build-scripts.html#rustc-cfg
    }
}
