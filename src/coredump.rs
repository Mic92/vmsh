use libc::{c_void, off_t, PT_LOAD};
use nix::{
    sys::{
        mman::{mmap, MapFlags, ProtFlags},
        uio::{process_vm_readv, IoVec, RemoteIoVec},
    },
    unistd::Pid,
};
use simple_error::try_with;
use std::{fs::File, io::Write, ptr, slice::from_raw_parts_mut};
use std::{mem::size_of, os::unix::prelude::AsRawFd};

use crate::elf::{
    Ehdr, Elf_Addr, Elf_Half, Elf_Off, Elf_Word, Phdr, Shdr, ELFARCH, ELFCLASS, ELFDATA2, ELFMAG0,
    ELFMAG1, ELFMAG2, ELFMAG3, ET_CORE, EV_CURRENT, PF_W, PF_X, SHN_UNDEF,
};
use crate::inspect::InspectOptions;
use crate::page_math::{page_align, page_size};
use crate::result::Result;
use crate::{kvm, proc::Mapping};

fn p_flags(f: &ProtFlags) -> Elf_Word {
    (if f.contains(ProtFlags::PROT_READ) {
        PF_X
    } else {
        0
    }) | (if f.contains(ProtFlags::PROT_WRITE) {
        PF_W
    } else {
        0
    }) | (if f.contains(ProtFlags::PROT_EXEC) {
        PF_X
    } else {
        0
    })
}

unsafe fn any_as_bytes<T: Sized>(p: &T) -> &[u8] {
    std::slice::from_raw_parts((p as *const T) as *const u8, size_of::<T>())
}

fn write_corefile(pid: Pid, core_file: &mut File, maps: &[Mapping]) -> Result<()> {
    let ehdr = Ehdr {
        e_ident: [
            ELFMAG0, ELFMAG1, ELFMAG2, ELFMAG3, ELFCLASS, ELFDATA2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
        e_type: ET_CORE,
        e_machine: ELFARCH,
        e_version: EV_CURRENT,
        e_entry: 0,
        e_phoff: size_of::<Ehdr>() as Elf_Off,
        e_shoff: 0,
        e_flags: 0,
        e_ehsize: size_of::<Ehdr>() as Elf_Half,
        e_phentsize: size_of::<Phdr>() as Elf_Half,
        e_phnum: maps.len() as Elf_Half,
        e_shentsize: size_of::<Shdr> as Elf_Half,
        e_shnum: 0,
        e_shstrndx: SHN_UNDEF,
    };

    let offset = page_align(size_of::<Ehdr>() + (size_of::<Phdr>() * ehdr.e_phnum as usize));
    let mut core_size = offset;

    let section_headers: Vec<_> = maps
        .iter()
        .map(|m| -> Phdr {
            let phdr = Phdr {
                p_type: PT_LOAD,
                p_flags: p_flags(&m.prot_flags),
                p_offset: core_size as Elf_Off,
                p_vaddr: m.start as Elf_Addr,
                p_paddr: m.phys_addr as Elf_Addr,
                p_filesz: m.size() as Elf_Addr,
                p_memsz: m.size() as Elf_Addr,
                p_align: page_size() as Elf_Addr,
            };
            core_size += m.size();
            phdr
        })
        .collect();

    try_with!(
        core_file.set_len(core_size as u64),
        "cannot truncate core file"
    );
    try_with!(
        core_file.write_all(unsafe { any_as_bytes(&ehdr) }),
        "cannot write elf header"
    );
    for header in section_headers {
        try_with!(
            core_file.write_all(unsafe { any_as_bytes(&header) }),
            "cannot write elf header"
        );
    }
    try_with!(core_file.flush(), "cannot flush core file");

    let buf_size = core_size - offset;
    let raw_buf = try_with!(
        unsafe {
            mmap(
                ptr::null_mut::<c_void>(),
                buf_size,
                ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                core_file.as_raw_fd(),
                offset as off_t,
            )
        },
        "cannot mmap core file"
    );
    let buf = unsafe { from_raw_parts_mut(raw_buf as *mut u8, buf_size) };

    let dst_iovs = vec![IoVec::from_mut_slice(buf)];
    let src_iovs = maps
        .iter()
        .map(|m| RemoteIoVec {
            base: m.start,
            len: m.size(),
        })
        .collect::<Vec<_>>();

    try_with!(
        process_vm_readv(pid, dst_iovs.as_slice(), src_iovs.as_slice()),
        "cannot read hypervisor memory"
    );

    Ok(())
}

pub fn generate_coredump(opts: &InspectOptions) -> Result<()> {
    let core_path = format!("core.{}", opts.pid);
    println!("Write {}", core_path);
    let mut core_file = try_with!(
        File::create(&core_path),
        "cannot open core_file: {}",
        &core_path
    );
    let vm = try_with!(
        kvm::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    let maps = vm.get_maps()?;
    try_with!(
        write_corefile(opts.pid, &mut core_file, &maps),
        "cannot write core file"
    );
    Ok(())
}
