use nix::unistd::{sysconf, SysconfVar};

pub fn page_size() -> usize {
    sysconf(SysconfVar::PAGE_SIZE).unwrap().unwrap() as usize
}

pub fn huge_page_size(level: u8) -> usize {
    page_size() << (9 * (3 - level))
}

pub fn page_start(v: usize) -> usize {
    v & !(page_size() - 1)
}

pub fn page_align(v: usize) -> usize {
    (v + page_size() - 1) & !(page_size() - 1)
}

pub fn is_page_aligned(v: usize) -> bool {
    v & (page_size() - 1) == 0
}

pub fn compute_host_offset(host_addr: usize, phys_addr: usize) -> isize {
    if host_addr > phys_addr {
        (host_addr - phys_addr) as isize
    } else {
        -((phys_addr - host_addr) as isize)
    }
}
