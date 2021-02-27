use nix::unistd::Pid;
use simple_error::try_with;
use std::fs::File;
use std::mem::{size_of, size_of_val};

use crate::inspect::InspectOptions;
use crate::{
    elf::{
        Ehdr, Elf_Half, Elf_Off, Phdr, Shdr, ELFARCH, ELFCLASS, ELFDATA2, ELFMAG0, ELFMAG1,
        ELFMAG2, ELFMAG3, ET_CORE, EV_CURRENT, SHN_UNDEF,
    },
    page_math::page_align,
};
use crate::{kvm, proc::Mapping, result::Result};

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

    let section_headers: Vec<Phdr> = Vec::with_capacity(ehdr.e_phnum as usize);
    let offset = page_align(size_of::<Ehdr>() + size_of_val(&section_headers));
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
