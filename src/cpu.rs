// TODO user_regs_struct is only available for x86/x86_64 right now in libc crate
use libc::user_regs_struct;

#[cfg(target_arch = "arm")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.r15
}

#[cfg(target_arch = "aarch64")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.pc
}

#[cfg(target_arch = "x86")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.eip
}

#[cfg(target_arch = "x86_64")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.rip
}

#[cfg(target_arch = "mips")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.epc
}

#[cfg(target_arch = "powerpc")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.ssr0 as u64
}

#[cfg(target_arch = "powerpc64")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.ssr0
}

#[cfg(target_arch = "sparc")]
pub fn get_ip(regs: &user_regs_struct) -> u64 {
    regs.pc
}
