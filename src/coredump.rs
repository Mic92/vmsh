use libc::PT_LOAD;
use nix::{sys::mman::ProtFlags, unistd::Pid};
use simple_error::try_with;
use std::fs::File;
use std::mem::size_of;

use crate::elf::{
    Ehdr, Elf_Addr, Elf_Half, Elf_Off, Elf_Word, Phdr, Shdr, ELFARCH, ELFCLASS, ELFDATA2, ELFMAG0,
    ELFMAG1, ELFMAG2, ELFMAG3, ET_CORE, EV_CURRENT, PF_W, PF_X, SHN_UNDEF,
};
use crate::inspect::InspectOptions;
use crate::page_math::{page_align, page_size};
use crate::{kvm, proc::Mapping, result::Result};

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

fn write_corefile(pid: Pid, core_file: &mut File, maps: &[Mapping]) {
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

    //src_iovecs = (iovec * len(slots))()
    //dst_iovec = iovec()

    //core_file.truncate(core_size)
    //core_file.write(bytearray(ehdr))
    //core_file.write(bytearray(section_headers))
    //core_file.flush()

    //buf = mmap.mmap(
    //    core_file.fileno(),
    //    core_size - offset,
    //    mmap.MAP_SHARED,
    //    mmap.PROT_WRITE,
    //    offset=offset,
    //)
    //try:
    //    c_void = ctypes.c_void_p.from_buffer(buf)  # type: ignore
    //    ptr = ctypes.addressof(c_void)
    //    dst_iovec.iov_base = ptr
    //    dst_iovec.iov_len = core_size - offset
    //    for iov, slot in zip(src_iovecs, slots):
    //        iov.iov_base = slot.start
    //        iov.iov_len = slot.size
    //    libc.process_vm_readv(pid, dst_iovec, 1, src_iovecs, len(src_iovecs), 0)
    //finally:
    //    # gc references to buf so we can close it
    //    del ptr
    //    del c_void
    //    buf.close()
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
    write_corefile(opts.pid, &mut core_file, &maps);
    Ok(())
}
