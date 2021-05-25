use simple_error::try_with;
use std::fs::File;
use std::io::Read;

use crate::result::Result;
use crate::sys_ext::prctl;

pub const _LINUX_CAPABILITY_VERSION_1: u32 = 0x1998_0330;
pub const _LINUX_CAPABILITY_VERSION_2: u32 = 0x2007_1026;
pub const _LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

pub const CAP_SYS_CHROOT: u32 = 18;
pub const CAP_SYS_PTRACE: u32 = 19;

#[repr(C)]
struct _vfs_cap_data {
    permitted: u32,
    inheritable: u32,
}

fn last_capability() -> Result<u64> {
    let path = "/proc/sys/kernel/cap_last_cap";
    let mut f = try_with!(File::open(path), "failed to open {}", path);

    let mut contents = String::new();
    try_with!(f.read_to_string(&mut contents), "failed to read {}", path);
    contents.pop(); // remove newline
    Ok(try_with!(
        contents.parse::<u64>(),
        "failed to parse capability, got: '{}'",
        contents
    ))
}

pub fn drop(inheritable_capabilities: u64) -> Result<()> {
    // we need chroot at the moment for `exec` command
    let inheritable = inheritable_capabilities | 1 << CAP_SYS_CHROOT | 1 << CAP_SYS_PTRACE;
    let last_capability = try_with!(last_capability(), "failed to read capability limit");

    for cap in 0..last_capability {
        if (inheritable & (1 << cap)) == 0 {
            // TODO: do not ignore result
            let _ = prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0);
        }
    }
    Ok(())
}
