use log::{info, trace};
use nix::sys::mman::ProtFlags;
use simple_error::{require_with, try_with, SimpleError};
use std::collections::HashMap;
use std::ffi::CStr;
use std::mem::{self, size_of};
use std::ops::Range;
use vm_memory::remote_mem::process_read_bytes;

use crate::guest_mem::{GuestMem, MappedMemory};
use crate::kvm::hypervisor::Hypervisor;
use crate::result::Result;

/// Kernel range on x86_64
pub const LINUX_KERNEL_KASLR_RANGE: Range<usize> = 0xFFFFFFFF80000000..0xFFFFFFFFC0000000;

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn not_printable(byte: u8) -> bool {
    !(0x20 < byte && byte < 0x7E)
}

fn round_up(num: usize, align: usize) -> usize {
    ((num + align - 1) / align) * align
}

fn find_ksymtab_strings_section(mem: &[u8]) -> Option<Range<usize>> {
    let idx = find_subsequence(mem, b"init_task")?;

    let start_offset = mem[..idx]
        .windows(2)
        .rev()
        .position(|b| not_printable(b[0]) && not_printable(b[1]))?;

    // In case we mis-detect the start of this section (for example if an data is printable by chance),
    // we round up the start of the string to the nearest 4 bytes,
    // as ksymtab or kcrctab are always four byte aligned

    //let start = round_up(idx - start_offset + 1, 4);
    let start = round_up(idx - start_offset, 4);

    let end_offset = mem[idx..]
        .windows(2)
        .position(|b| not_printable(b[0]) && not_printable(b[1]))?;

    let end = idx + end_offset + 1;

    //dbg!(String::from_utf8_lossy(&mem[start..end]));

    Some(start..end)
}

/// From include/linux/export.h
/// FIXME: on many archs, especially 32-bit ones, this layout is used!
/// struct kernel_symbol {
///   unsigned long value;
///   const char *name;
///   const char *namespace;
/// };
#[repr(C)]
#[derive(Debug)]
pub struct kernel_symbol {
    pub value_offset: libc::c_int,
    pub name_offset: libc::c_int,
    pub namespace_offset: libc::c_int,
}

// 5.3 did not have namespace_offset yet, we simply check for both
#[repr(C)]
#[derive(Debug)]
pub struct kernel_symbol_5_3 {
    pub value_offset: libc::c_int,
    pub name_offset: libc::c_int,
}

unsafe fn cast_kernel_sym(mem: &[u8]) -> &kernel_symbol {
    &*mem::transmute::<_, *const kernel_symbol>(mem.as_ptr())
}

fn check_kernel_sym(
    mem: &[u8],
    strings_range: &Range<usize>,
    sym_size: usize,
    ii: usize,
) -> Option<usize> {
    let sym = unsafe { cast_kernel_sym(&mem[ii - sym_size..ii]) };
    let field_offset =
        &sym.name_offset as *const i32 as usize - sym as *const kernel_symbol as usize;
    let name_idx = ii + (sym.name_offset as usize) + field_offset;
    if strings_range.contains(&name_idx) && ii > sym_size {
        let name_idx_2 = ii + (sym.name_offset as usize) + field_offset - sym_size;
        if strings_range.contains(&name_idx_2) {
            return Some(sym_size);
        }
    }
    return None;
}

/// Skips over kcrctab if present and retrieves actual ksymtab offset. This is
/// done by casting each offset to a kernel_symbole and check if its name_offset
/// would fall into the ksymtab_string address range.
fn get_ksymtab_start(mem: &[u8], strings_range: &Range<usize>) -> Option<(usize, usize)> {
    // each entry in kcrctab is 32 bytes
    let step_size = size_of::<u32>();
    for ii in (0..strings_range.start + 1).rev().step_by(step_size) {
        let sym_size = check_kernel_sym(mem, strings_range, size_of::<kernel_symbol>(), ii)
            .or_else(|| check_kernel_sym(mem, strings_range, size_of::<kernel_symbol_5_3>(), ii));
        if let Some(sym_size) = sym_size {
            return Some((ii, sym_size));
        }
    }

    None
}

/// algorithm for reconstructing function pointers;
/// - assuming we have HAVE_ARCH_PREL32_RELOCATIONS (is the case on arm64 and x86_64)
/// - Get Address of last symbol in __ksymtab_string
/// - if CONFIG_MODVERSIONS we have __kcrctab, otherwise no (seems to be the case on Ubuntu, probably not in most microvm kernels i.e. kata containers)
/// - Maybe not implement crc for but add a check if symbols in ksymtab_string have weird subfixes: i.e. printk_R1b7d4074 instead of printk ?
/// - Is __ksymtab seems not to be at a predictable offsets?
///
/// 0xffffffff80000000–0xffffffffc0000000
/// dump_page_table(pml4, mem)
///
/// Layout of __ksymtab,  __ksymtab_gpl
/// struct kernel_symbol {
///    int value_offset;
///    int name_offset;
///    int namespace_offset;
/// };
///
/// To convert an offset to its pointer: ptr = (unsigned long)&sym.offset + sym.offset
///
/// Layout of __ksymtab,  __ksymtab_gpl
/// __ksymtab_strings
/// null terminated, strings

fn apply_offset(addr: usize, offset: libc::c_int) -> usize {
    if offset < 0 {
        addr - (-offset as usize)
    } else {
        // Why do we substract this?
        // All symbols seems to be in data section
        addr - offset as usize
    }
}

fn get_kernel_symbols(
    mem: &[u8],
    mem_base: usize,
    ksymtab_strings: Range<usize>,
) -> Result<HashMap<String, usize>> {
    let mut syms = HashMap::new();
    let (start, sym_size) =
        require_with!(get_ksymtab_start(mem, &ksymtab_strings), "no ksymtab found");

    info!(
        "found ksymtab {} bytes before ksymtab_strings",
        ksymtab_strings.start - start
    );

    for ii in (0..start + 1).rev().step_by(sym_size) {
        let sym_start = ii - sym_size;
        let sym = unsafe { cast_kernel_sym(&mem[sym_start..ii]) };
        let name_offset =
            &sym.name_offset as *const i32 as usize - sym as *const kernel_symbol as usize;
        let value_offset =
            &sym.value_offset as *const i32 as usize - sym as *const kernel_symbol as usize;
        let name_idx = sym_start + name_offset + sym.name_offset as usize;
        let value_ptr = apply_offset(mem_base + sym_start + value_offset, sym.value_offset);
        if !ksymtab_strings.contains(&name_idx) {
            break;
        }
        let name_len = require_with!(
            mem[name_idx..].iter().position(|c| *c == 0),
            "symbol name does not end"
        );
        let name = try_with!(
            CStr::from_bytes_with_nul(&mem[name_idx..name_idx + name_len + 1]),
            "invalid symbol name"
        );
        let name = try_with!(name.to_str(), "invalid encoding for symbol name");
        trace!("{} @ {:x}", name, value_ptr);
        syms.insert(name.to_owned(), value_ptr);
    }
    Ok(syms)
}

pub struct Kernel {
    pub range: Range<usize>,
    pub memory_sections: Vec<MappedMemory>,
    pub symbols: HashMap<String, usize>,
}

impl Kernel {
    pub fn space_before(&self) -> usize {
        self.range.start - LINUX_KERNEL_KASLR_RANGE.start
    }
    pub fn space_after(&self) -> usize {
        LINUX_KERNEL_KASLR_RANGE.end - self.range.end
    }
}

pub fn find_kernel(guest_mem: &GuestMem, hv: &Hypervisor) -> Result<Kernel> {
    let memory_sections = try_with!(
        guest_mem.find_kernel_sections(hv, LINUX_KERNEL_KASLR_RANGE),
        "could not find Linux kernel in VM memory"
    );
    let kernel_last = memory_sections.last().unwrap();
    let kernel_start = memory_sections.first().unwrap().virt_start;
    let kernel_end = kernel_last.virt_start + kernel_last.len;
    info!(
        "found linux kernel at {:#x}-{:#x}",
        kernel_start, kernel_end
    );
    let symbols = memory_sections.iter().find_map(|s| {
        if s.prot != ProtFlags::PROT_READ {
            return None;
        }
        let mut mem = vec![0; s.len];
        let mem_base = s.phys_start.host_addr() as *const libc::c_void;
        if let Err(e) = process_read_bytes(hv.pid, &mut mem, mem_base) {
            return Some(Err(SimpleError::new(format!(
                "failed to read linux kernel from hypervisor memory: {}",
                e
            ))));
        }
        let strings_range = find_ksymtab_strings_section(&mem)?;

        let from_addr = s.phys_start.add(strings_range.start);
        let to_addr = s.phys_start.add(strings_range.end - 1);
        let string_num = mem[strings_range.clone()]
            .iter()
            .filter(|c| **c == 0)
            .count();
        info!(
            "found ksymtab_string at physical {:#x}:{:#x} with {} strings",
            from_addr.value, to_addr.value, string_num
        );
        match get_kernel_symbols(&mem, s.virt_start, strings_range) {
            Err(e) => Some(Err(SimpleError::new(format!(
                "failed to parse kernel symbols: {}",
                e
            )))),
            Ok(syms) => Some(Ok(syms)),
        }
    });

    let symbols = require_with!(symbols, "could not find section with kernel symbols")?;
    Ok(Kernel {
        range: kernel_start..kernel_end,
        memory_sections,
        symbols,
    })
}
