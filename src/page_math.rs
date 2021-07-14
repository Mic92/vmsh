use nix::unistd::{sysconf, SysconfVar};

pub fn page_size() -> usize {
    sysconf(SysconfVar::PAGE_SIZE).unwrap().unwrap() as usize
}

pub fn page_align(v: usize) -> usize {
    (v + page_size() - 1) & !(page_size() - 1)
}

pub fn add_offset(addr: usize, offset: isize) -> usize {
    if offset < 0 {
        addr - offset.wrapping_abs() as usize
    } else {
        addr + offset as usize
    }
}
