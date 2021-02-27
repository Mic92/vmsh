use libc::c_ulong;
use nix::unistd::{sysconf, SysconfVar};

pub fn page_size() -> usize {
    sysconf(SysconfVar::PAGE_SIZE).unwrap().unwrap() as usize
}

pub fn page_align(v: usize) -> usize {
    (v + page_size() - 1) & !(page_size() - 1)
}
