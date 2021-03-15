use libc::{c_void, off_t, timeval, PT_LOAD, PT_NOTE};
use nix::sys::{
    mman::{mmap, MapFlags, ProtFlags},
    uio::{process_vm_readv, IoVec, RemoteIoVec},
};
use nix::unistd::Pid;
use simple_error::try_with;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::{fs::File, io::Write, ptr, slice::from_raw_parts_mut};
use std::{mem::size_of, os::unix::prelude::AsRawFd};

use crate::cpu::{elf_fpregset_t, Regs};
use crate::elf::{
    elf_prpsinfo, elf_prstatus, elf_siginfo, Ehdr, Elf_Addr, Elf_Half, Elf_Off, Elf_Word, Nhdr,
    Phdr, Shdr, ELFARCH, ELFCLASS, ELFDATA2, ELFMAG0, ELFMAG1, ELFMAG2, ELFMAG3, ELF_NGREG,
    ET_CORE, EV_CURRENT, NT_PRFPREG, NT_PRPSINFO, NT_PRSTATUS, NT_PRXREG, PF_W, PF_X, SHN_UNDEF,
};
use crate::page_math::{page_align, page_size};
use crate::result::Result;
use crate::{kvm, proc::Mapping};

pub struct CoredumpOptions {
    pub pid: Pid,
    pub path: PathBuf,
}

#[repr(C)]
#[derive(Clone)]
pub struct core_user {
    magic: u64,
}

fn protection_flags(f: &ProtFlags) -> Elf_Word {
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

fn dump_mappings(
    pid: Pid,
    core_file: &mut File,
    core_size: off_t,
    file_offset: off_t,
    maps: &[Mapping],
) -> Result<()> {
    let buf_size = core_size - file_offset;
    let res = unsafe {
        mmap(
            ptr::null_mut::<c_void>(),
            buf_size as usize,
            ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            core_file.as_raw_fd(),
            file_offset,
        )
    };
    let raw_buf = try_with!(res, "cannot mmap core file");
    let buf = unsafe { from_raw_parts_mut(raw_buf as *mut u8, buf_size as usize) };

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

fn elf_header(phnum: Elf_Half) -> Ehdr {
    Ehdr {
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
        e_phnum: phnum,
        e_shentsize: size_of::<Shdr>() as Elf_Half,
        e_shnum: 0,
        e_shstrndx: SHN_UNDEF,
    }
}

fn pt_note_header(core_size: Elf_Off, filesize: Elf_Off) -> Phdr {
    Phdr {
        p_type: PT_NOTE,
        p_flags: 0,
        p_offset: core_size,
        p_vaddr: 0,
        p_paddr: 0,
        p_filesz: filesize,
        p_memsz: 0,
        p_align: 0,
    }
}

fn pt_load_header(m: &Mapping, offset: Elf_Off) -> Phdr {
    Phdr {
        p_type: PT_LOAD,
        p_flags: protection_flags(&m.prot_flags),
        p_offset: offset,
        p_vaddr: m.phys_addr as Elf_Addr,
        p_paddr: m.phys_addr as Elf_Addr,
        p_filesz: m.size() as Elf_Addr,
        p_memsz: m.size() as Elf_Addr,
        p_align: page_size() as Elf_Addr,
    }
}

fn write_note_section<T: Sized>(core_file: &mut File, hdr: &Nhdr, payload: &T) -> Result<()> {
    try_with!(
        core_file.write_all(unsafe { any_as_bytes(hdr) }),
        "cannot write elf note header"
    );
    try_with!(
        core_file.write_all(b"CORE\0\0\0\0"),
        "cannot write note name"
    );
    try_with!(
        core_file.write_all(unsafe { any_as_bytes(payload) }),
        "cannot write elf note header"
    );
    Ok(())
}

fn write_note_sections(core_file: &mut File, regs: &[VcpuState]) -> Result<()> {
    try_with!(
        write_note_section(
            core_file,
            &Nhdr {
                n_namesz: 5,
                n_descsz: size_of::<elf_prpsinfo>() as Elf_Word,
                n_type: NT_PRPSINFO,
            },
            &elf_prpsinfo {
                pr_state: 0,
                pr_sname: 0,
                pr_zomb: 0,
                pr_nice: 0,
                pr_flag: 0,
                pr_uid: 0,
                pr_gid: 0,
                pr_pid: 0,
                pr_ppid: 0,
                pr_pgrp: 0,
                pr_sid: 0,
                pr_fname: *b"qemu-system-x86_",
                pr_psargs: [0; 80],
            },
        ),
        "failed to write NT_PRPSINFO"
    );

    try_with!(
        write_note_section(
            core_file,
            &Nhdr {
                n_namesz: 5,
                n_descsz: size_of::<core_user>() as Elf_Word,
                n_type: NT_PRXREG,
            },
            &core_user { magic: 4242 }
        ),
        "failed to write NT_PRXREG"
    );

    let zero = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    for reg in regs {
        try_with!(
            write_note_section(
                core_file,
                &Nhdr {
                    n_namesz: 5,

                    n_descsz: size_of::<elf_prstatus>() as Elf_Word,
                    n_type: NT_PRSTATUS,
                },
                &elf_prstatus {
                    pr_info: elf_siginfo {
                        si_signo: 0,
                        si_code: 0,
                        si_errno: 0,
                    },
                    pr_cursig: 0,
                    pr_sigpend: 0,
                    pr_sighold: 0,
                    pr_pid: 0,
                    pr_ppid: 0,
                    pr_pgrp: 0,
                    pr_sid: 0,
                    pr_utime: zero,
                    pr_stime: zero,
                    pr_cutime: zero,
                    pr_cstime: zero,
                    pr_reg: [0; ELF_NGREG],
                    pr_fpvalid: 1,
                }
            ),
            "failed to write NT_PRSTATUS"
        );
        try_with!(
            write_note_section(
                core_file,
                &Nhdr {
                    n_namesz: 5,
                    n_descsz: size_of::<elf_prstatus>() as Elf_Word,
                    n_type: NT_PRFPREG,
                },
                &elf_fpregset_t {
                    cwd: 0,
                    swd: 0,
                    ftw: 0,
                    fop: 0,
                    rip: 0,
                    rdp: 0,
                    mxcsr: 0,
                    mxcr_mask: 0,
                    st_space: [0; 32],
                    xmm_space: [0; 64],
                    padding: [0; 24],
                }
            ),
            "failed to write NT_PRSTATUS"
        );
    }
    Ok(())
}

pub fn note_size<T>() -> usize {
    let name_size = 8;
    size_of::<Nhdr>() + name_size + size_of::<T>()
}

fn write_corefile(
    pid: Pid,
    core_file: &mut File,
    maps: &[Mapping],
    vcpus: &[VcpuState],
) -> Result<()> {
    // +1 == PT_NOTE section
    let ehdr = elf_header((maps.len() + 1) as Elf_Half);

    let pt_note_size = note_size::<elf_prpsinfo>()
        + note_size::<core_user>()
        + vcpus.len() * (note_size::<elf_prstatus>() + note_size::<elf_fpregset_t>());

    let metadata_size =
        page_align(size_of::<Ehdr>() + (size_of::<Phdr>() * ehdr.e_phnum as usize + pt_note_size));
    let mut core_size = metadata_size;

    let mut section_headers = vec![pt_note_header(
        core_size as Elf_Off,
        (vcpus.len() * size_of::<Nhdr>()) as u64,
    )];

    for m in maps {
        let phdr = pt_load_header(m, core_size as Elf_Off);
        core_size += m.size();
        section_headers.push(phdr);
    }

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
    try_with!(
        core_file.write_all(unsafe { any_as_bytes(&ehdr) }),
        "cannot write pt note section"
    );

    write_note_sections(core_file, vcpus)?;

    try_with!(core_file.flush(), "cannot flush core file");

    dump_mappings(
        pid,
        core_file,
        core_size as off_t,
        metadata_size as off_t,
        maps,
    )
}

struct VcpuState {
    num: usize,
    regs: Regs,
    fregs: elf_fpregset_t,
}

pub fn generate_coredump(opts: &CoredumpOptions) -> Result<()> {
    println!("Write {}", opts.path.display());
    let mut core_file = try_with!(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&opts.path),
        "cannot open core_file: {}",
        opts.path.display()
    );
    let vm = try_with!(
        kvm::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    let tracee = vm.attach()?;
    let maps = tracee.get_maps()?;
    let res = vm
        .vcpus
        .iter()
        .enumerate()
        .map(|(i, vcpu)| {
            let regs = tracee.get_regs(vcpu)?;
            let fregs = tracee.get_fregs(vcpu)?;
            Ok(VcpuState {
                num: i,
                regs,
                fregs,
            })
        })
        .collect::<Result<Vec<VcpuState>>>();
    let vcpu_states = try_with!(res, "fail to dump vcpu registers");
    try_with!(
        write_corefile(opts.pid, &mut core_file, &maps, vcpu_states.as_slice()),
        "cannot write core file"
    );
    Ok(())
}
