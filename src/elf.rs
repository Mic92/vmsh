#![allow(dead_code)]

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

// e_version
pub const EV_NONE: Elf_Word = 0;
pub const EV_CURRENT: Elf_Word = 1;
pub const EV_NUM: Elf_Word = 2;

// e_shstrndx
pub const SHN_UNDEF: Elf_Half = 0;

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
    pub use libc::Elf64_Ehdr as Ehdr;
    pub use libc::Elf64_Half as Elf_Half;
    pub use libc::Elf64_Off as Elf_Off;
    pub use libc::Elf64_Phdr as Phdr;
    pub use libc::Elf64_Shdr as Shdr;
    pub use libc::Elf64_Word as Elf_Word;
    pub const ELFCLASS: u8 = super::ELFCLASS64;
}

#[cfg(target_arch = "powerpc")]
pub const ELFARCH: Elf_Half = EM_PPC;

#[cfg(target_arch = "powerpc64")]
pub const ELFARCH: Elf_Half = EM_PPC64;

#[cfg(target_arch = "arm")]
pub const ELFARCH: Elf_Half = EM_ARM;

#[cfg(target_arch = "mips")]
pub const ELFARCH: Elf_Half = EM_MIPS;

#[cfg(target_arch = "riscv")]
pub const ELFARCH: Elf_Half = EM_RISCV;

#[cfg(target_arch = "x86")]
pub const ELFARCH: Elf_Half = EM_386;

#[cfg(target_arch = "x86_64")]
pub const ELFARCH: Elf_Half = EM_X86_64;

#[cfg(target_endian = "big")]
pub const ELFDATA2: u8 = ELFDATA2MSB;

#[cfg(not(target_endian = "big"))]
pub const ELFDATA2: u8 = ELFDATA2LSB;

pub use headers::*;
