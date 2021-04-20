use crate::kvm::hypervisor::VCPU;
use kvm_bindings as kvmb;
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

use crate::cpu::{FpuRegs, Regs};
use crate::elf::{
    elf_prpsinfo, elf_prstatus, elf_siginfo, Ehdr, Elf_Addr, Elf_Half, Elf_Off, Elf_Word, Nhdr,
    Phdr, Shdr, ELFARCH, ELFCLASS, ELFDATA2, ELFMAG0, ELFMAG1, ELFMAG2, ELFMAG3, ELF_NGREG,
    ET_CORE, EV_CURRENT, NT_PRPSINFO, NT_PRSTATUS, NT_PRXREG, PF_W, PF_X, SHN_UNDEF,
};
use crate::kvm::hypervisor::Hypervisor;
use crate::page_math::{page_align, page_size};
use crate::result::Result;
use crate::{kvm, tracer::proc::Mapping};

pub struct CoredumpOptions {
    pub pid: Pid,
    pub path: PathBuf,
}

#[repr(C)]
#[derive(Clone)]
pub struct core_user {
    vcpu: usize,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    sregs: kvmb::kvm_sregs,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    msrs: [kvmb::kvm_msr_entry; 1],
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

fn pt_note_header(core_size: Elf_Off, file_size: Elf_Off) -> Phdr {
    Phdr {
        p_type: PT_NOTE,
        p_flags: 0,
        p_offset: core_size,
        p_vaddr: 0,
        p_paddr: 0,
        p_filesz: file_size,
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

fn write_note_section<T: Sized>(core_file: &mut File, ntype: Elf_Word, payload: &T) -> Result<()> {
    let hdr = &Nhdr {
        n_namesz: 5,
        n_descsz: size_of::<T>() as Elf_Word,
        n_type: ntype,
    };
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

#[cfg(target_arch = "x86_64")]
fn write_fpu_registers(core_file: &mut File, regs: &FpuRegs) -> Result<()> {
    use crate::elf::NT_PRXFPREG;
    let hdr = &Nhdr {
        n_namesz: 5,
        n_descsz: size_of::<FpuRegs>() as Elf_Word,
        n_type: NT_PRXFPREG,
    };
    try_with!(
        core_file.write_all(unsafe { any_as_bytes(hdr) }),
        "cannot write elf note header"
    );
    try_with!(
        core_file.write_all(b"LINUX\0\0\0"),
        "cannot write note name"
    );
    try_with!(
        core_file.write_all(unsafe { any_as_bytes(regs) }),
        "cannot write elf note header"
    );
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
fn write_fpu_registers(core_file: &mut File, regs: &FpuRegs) -> Result<()> {
    use crate::elf::NT_PRFPREG;
    try_with!(
        write_note_section(
            core_file,
            NT_PRFPREG
            regs
        ),
        "failed to write NT_PRFPREG"
    );
    Ok(())
}

fn write_note_sections(core_file: &mut File, vcpus: &[VcpuState]) -> Result<()> {
    try_with!(
        write_note_section(
            core_file,
            NT_PRPSINFO,
            &elf_prpsinfo {
                pr_state: 0,
                pr_sname: 0,
                pr_zomb: 0,
                pr_nice: 0,
                pr_flag: 0,
                pr_uid: 0,
                pr_gid: 0,
                pr_pid: 1,
                pr_ppid: 1,
                pr_pgrp: 0,
                pr_sid: 0,
                pr_fname: *b"qemu-system-x86_",
                pr_psargs: [0; 80],
            },
        ),
        "failed to write NT_PRPSINFO"
    );

    let zero = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    for (i, vcpu) in vcpus.iter().enumerate() {
        let pr_reg = unsafe { ptr::read(&vcpu.regs as *const Regs as *const [u64; ELF_NGREG]) };
        try_with!(
            write_note_section(
                core_file,
                NT_PRSTATUS,
                &elf_prstatus {
                    pr_info: elf_siginfo {
                        si_signo: 0,
                        si_code: 0,
                        si_errno: 0,
                    },
                    pr_cursig: 0,
                    pr_sigpend: 0,
                    pr_sighold: 0,
                    pr_pid: (i + 1) as i32,
                    pr_ppid: 1,
                    pr_pgrp: 0,
                    pr_sid: 0,
                    pr_utime: zero,
                    pr_stime: zero,
                    pr_cutime: zero,
                    pr_cstime: zero,
                    pr_reg,
                    pr_fpvalid: 1,
                }
            ),
            "failed to write NT_PRSTATUS"
        );

        try_with!(
            write_note_section(
                core_file,
                NT_PRXREG,
                &core_user {
                    vcpu: i,
                    sregs: vcpu.sregs,
                    msrs: vcpu.msrs
                }
            ),
            "failed to write NT_PRXREG"
        );

        write_fpu_registers(core_file, &vcpu.fpu_regs)?;
    }
    Ok(())
}

pub fn note_size<T>() -> usize {
    // name is 4 bit aligned, we write CORE\0 to it
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

    let metadata_size = size_of::<Ehdr>() + (size_of::<Phdr>() * ehdr.e_phnum as usize);
    let mut core_size = metadata_size;

    let pt_note_size = note_size::<elf_prpsinfo>()
        + vcpus.len()
            * (note_size::<core_user>() + note_size::<elf_prstatus>() + note_size::<FpuRegs>());
    let mut section_headers = vec![pt_note_header(core_size as Elf_Off, pt_note_size as u64)];
    core_size += pt_note_size;
    core_size = page_align(core_size);

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
    write_note_sections(core_file, vcpus)?;

    try_with!(core_file.flush(), "cannot flush core file");

    dump_mappings(
        pid,
        core_file,
        core_size as off_t,
        page_align(metadata_size + pt_note_size) as off_t,
        maps,
    )
}

const MSR_EFER: u32 = 0xc0000080;
struct VcpuState {
    regs: Regs,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    sregs: kvmb::kvm_sregs,
    fpu_regs: FpuRegs,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    msrs: [kvmb::kvm_msr_entry; 1],
}

impl VcpuState {
    /// Requires the hypervisor to be stopped.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn new(vcpu: &VCPU, hv: &Hypervisor) -> Result<VcpuState> {
        let regs = hv.get_regs(vcpu)?;
        let sregs = hv.get_sregs(vcpu)?;
        let fpu_regs = hv.get_fpu_regs(vcpu)?;
        let entry = kvmb::kvm_msr_entry {
            index: MSR_EFER,
            ..Default::default()
        };
        let msr = hv.get_msr(vcpu, &entry)?;
        Ok(VcpuState {
            regs,
            sregs,
            fpu_regs,
            msrs: [msr],
        })
    }
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
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    );
    vm.stop()?;
    let maps = vm.get_maps()?;
    let res = vm
        .vcpus
        .iter()
        .map(|vcpu| VcpuState::new(&vcpu, &vm))
        .collect::<Result<Vec<VcpuState>>>();
    let vcpu_states = try_with!(res, "fail to dump vcpu registers");
    try_with!(
        write_corefile(opts.pid, &mut core_file, &maps, vcpu_states.as_slice()),
        "cannot write core file"
    );
    Ok(())
}
