#![allow(dead_code, non_camel_case_types)]
use libc::{c_char, c_int, c_short, c_uchar, c_uint, c_ulong, pid_t, timeval};

// EI_CLASS
const ELFCLASSNONE: u8 = 0;
const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;

// e_type
/// No file type
pub const ET_NONE: Elf_Half = 0;
/// Relocatable file (object file)
pub const ET_REL: Elf_Half = 1;
/// Executable file
pub const ET_EXEC: Elf_Half = 2;
/// Shared object file
pub const ET_DYN: Elf_Half = 3;
/// Core file
pub const ET_CORE: Elf_Half = 4;

// e_machine
const EM_386: Elf_Half = 3;
const EM_PPC: Elf_Half = 20;
const EM_PPC64: Elf_Half = 21;
const EM_X86_64: Elf_Half = 62;
const EM_MIPS: Elf_Half = 8;
const EM_ARM: Elf_Half = 40;
const EM_AARCH64: Elf_Half = 183;
const EM_RISCV: Elf_Half = 243;

// n_type
pub const NT_PRSTATUS: Elf_Word = 1;
pub const NT_PRFPREG: Elf_Word = 2;
pub const NT_PRPSINFO: Elf_Word = 3;
pub const NT_PRXREG: Elf_Word = 4;
pub const NT_TASKSTRUCT: Elf_Word = 4;
pub const NT_PLATFORM: Elf_Word = 5;
pub const NT_AUXV: Elf_Word = 6;
pub const NT_GWINDOWS: Elf_Word = 7;
pub const NT_ASRS: Elf_Word = 8;
pub const NT_PSTATUS: Elf_Word = 10;
pub const NT_PSINFO: Elf_Word = 13;
pub const NT_PRCRED: Elf_Word = 14;
pub const NT_UTSNAME: Elf_Word = 15;
pub const NT_LWPSTATUS: Elf_Word = 16;
pub const NT_LWPSINFO: Elf_Word = 17;
pub const NT_PRFPXREG: Elf_Word = 20;
pub const NT_SIGINFO: Elf_Word = 0x53494749;
pub const NT_FILE: Elf_Word = 0x46494c45;
#[cfg(target_arch = "x86_64")]
pub const NT_PRXFPREG: Elf_Word = 0x46e62b7f;

// e_version
pub const EV_NONE: Elf_Word = 0;
pub const EV_CURRENT: Elf_Word = 1;
pub const EV_NUM: Elf_Word = 2;

// e_shstrndx
pub const SHN_UNDEF: Elf_Half = 0;

// e_type
pub const PF_X: Elf_Word = 1 << 0;
pub const PF_W: Elf_Word = 1 << 1;
pub const PF_R: Elf_Word = 1 << 2;

pub const ELFMAG0: u8 = 0x7f;
pub const ELFMAG1: u8 = b'E';
pub const ELFMAG2: u8 = b'L';
pub const ELFMAG3: u8 = b'F';

/// Invalid data encoding
const ELFDATANONE: u8 = 0;
/// Little endian
const ELFDATA2LSB: u8 = 1;
/// Big endian
const ELFDATA2MSB: u8 = 2;

#[cfg(target_pointer_width = "32")]
mod headers {
    pub use libc::Elf32_Addr as Elf_Addr;
    pub use libc::Elf32_Ehdr as Ehdr;
    pub use libc::Elf32_Half as Elf_Half;
    pub use libc::Elf32_Off as Elf_Off;
    pub use libc::Elf32_Phdr as Phdr;
    pub use libc::Elf32_Shdr as Shdr;
    pub use libc::Elf32_Word as Elf_Word;
    pub const ELFCLASS: u8 = super::ELFCLASS32;
}
#[cfg(target_pointer_width = "64")]
mod headers {
    pub use libc::Elf64_Addr as Elf_Addr;
    pub use libc::Elf64_Ehdr as Ehdr;
    pub use libc::Elf64_Half as Elf_Half;
    pub use libc::Elf64_Off as Elf_Off;
    pub use libc::Elf64_Phdr as Phdr;
    pub use libc::Elf64_Shdr as Shdr;
    pub use libc::Elf64_Word as Elf_Word;
    pub const ELFCLASS: u8 = super::ELFCLASS64;
}

pub struct Nhdr {
    pub n_namesz: Elf_Word,
    pub n_descsz: Elf_Word,
    pub n_type: Elf_Word,
}

#[cfg(target_pointer_width = "32")]
pub type elf_gregset_t = [u32; ELF_NGREG];

#[cfg(target_pointer_width = "64")]
pub type elf_gregset_t = [u64; ELF_NGREG];

#[cfg(target_arch = "powerpc")]
mod arch {
    pub const ELF_NGREG: usize = 48;
    pub const ELFARCH: super::Elf_Half = super::EM_PPC;
}

#[cfg(target_arch = "powerpc64")]
mod arch {
    pub const ELF_NGREG: usize = 48;
    pub const ELFARCH: super::Elf_Half = super::EM_PPC64;
}

#[cfg(target_arch = "arm")]
mod arch {
    pub const ELF_NGREG: usize = 18;
    pub const ELFARCH: super::Elf_Half = super::EM_ARM;
}

#[cfg(target_arch = "aarch64")]
mod arch {
    pub const ELF_NGREG: usize = 34;
    pub const ELFARCH: super::Elf_Half = super::EM_AARCH64;
}

#[cfg(target_arch = "mips")]
mod arch {
    pub const ELF_NGREG: usize = 45;
    pub const ELFARCH: super::Elf_Half = super::EM_MIPS;
}

#[cfg(target_arch = "riscv")]
mod arch {
    pub const ELF_NGREG: usize = 32;
    pub const ELFARCH: super::Elf_Half = super::EM_RISCV;
}

#[cfg(target_arch = "x86")]
mod arch {
    pub const ELF_NGREG: usize = 17;
    pub const ELFARCH: super::Elf_Half = super::EM_386;
}

#[cfg(target_arch = "x86_64")]
mod arch {
    pub const ELF_NGREG: usize = 27;
    pub const ELFARCH: super::Elf_Half = super::EM_X86_64;
}

#[cfg(target_endian = "big")]
pub const ELFDATA2: u8 = ELFDATA2MSB;

#[cfg(not(target_endian = "big"))]
pub const ELFDATA2: u8 = ELFDATA2LSB;

#[repr(C)]
#[derive(Clone)]
pub struct elf_siginfo {
    pub si_signo: c_int,
    pub si_code: c_int,
    pub si_errno: c_int,
}

const ELF_PRARGSZ: usize = 80;

#[repr(C)]
#[derive(Clone)]
pub struct elf_prstatus {
    pub pr_info: elf_siginfo,
    pub pr_cursig: c_short,
    pub pr_sigpend: c_ulong,
    pub pr_sighold: c_ulong,
    pub pr_pid: pid_t,
    pub pr_ppid: pid_t,
    pub pr_pgrp: pid_t,
    pub pr_sid: pid_t,
    pub pr_utime: timeval,
    pub pr_stime: timeval,
    pub pr_cutime: timeval,
    pub pr_cstime: timeval,
    pub pr_reg: elf_gregset_t,
    pub pr_fpvalid: c_int,
}

#[repr(C)]
#[derive(Clone)]
pub struct elf_prpsinfo {
    pub pr_state: c_char,
    pub pr_sname: c_char,
    pub pr_zomb: c_char,
    pub pr_nice: c_char,
    pub pr_flag: c_ulong,

    #[cfg(target_pointer_width = "64")]
    pub pr_uid: c_uint,
    #[cfg(target_pointer_width = "32")]
    pub pr_uid: libc::c_ushort,

    #[cfg(target_pointer_width = "64")]
    pub pr_gid: c_uint,
    #[cfg(target_pointer_width = "32")]
    pub pr_gid: c_ushort,

    pub pr_pid: c_int,
    pub pr_ppid: c_int,
    pub pr_pgrp: c_int,
    pub pr_sid: c_int,
    pub pr_fname: [c_uchar; 16],
    pub pr_psargs: [c_char; ELF_PRARGSZ],
}

pub use arch::*;
pub use headers::*;
