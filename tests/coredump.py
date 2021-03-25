#!/usr/bin/env python3
import ctypes as ct
from typing import IO, List

from elftools.elf.elffile import ELFFile
from elftools.elf.segments import NoteSegment

NT_PRXFPREG = 1189489535


class ElfCore:
    """
    Not a general purpose coredump parser, but specialized on what we generate int the
    coredump subcommand.
    """

    regs: List["user_regs_struct"] = []
    fpu_regs: List["user_fpregs_struct"] = []
    special_regs: List["KVMSRegs"] = []
    msrs: List[List["kvm_msr_entry"]] = []

    def __init__(self, fd: IO[bytes]) -> None:
        self.elf = ELFFile(fd)
        note_segment = next(self.elf.iter_segments())
        assert isinstance(note_segment, NoteSegment)
        for note in note_segment.iter_notes():
            if note.n_type == "NT_PRSTATUS":
                assert note.n_descsz == ct.sizeof(elf_prstatus)
                self.regs.append(
                    elf_prstatus.from_buffer_copy(note.n_desc.encode("latin-1")).pr_reg
                )
            elif note.n_type == NT_PRXFPREG:
                assert note.n_descsz == ct.sizeof(elf_fpregset_t)
                self.fpu_regs.append(
                    elf_fpregset_t.from_buffer_copy(note.n_desc.encode("latin-1"))
                )
            # actually not NT_TASKSTRUCT but elftools detect it as such
            elif note.n_type == "NT_TASKSTRUCT":
                assert note.n_descsz == ct.sizeof(core_user)
                custom = core_user.from_buffer_copy(note.n_desc.encode("latin1"))
                self.special_regs.append(custom.sregs)
                self.msrs.append(custom.msrs)


# elf_prstatus related constants.
# Signal info.
class elf_siginfo(ct.Structure):  # struct elf_siginfo
    _fields_ = [
        (
            "si_signo",
            ct.c_int,
        ),
        (
            "si_code",
            ct.c_int,
        ),
        ("si_errno", ct.c_int),
    ]


# A time value that is accurate to the nearest
# microsecond but also has a range of years.
class timeval(ct.Structure):
    _fields_ = [
        ("tv_sec", ct.c_long),
        ("tv_usec", ct.c_long),
    ]


class user_regs_struct(ct.Structure):
    _fields_ = [
        ("r15", ct.c_ulonglong),
        ("r14", ct.c_ulonglong),
        ("r13", ct.c_ulonglong),
        ("r12", ct.c_ulonglong),
        ("rbp", ct.c_ulonglong),
        ("rbx", ct.c_ulonglong),
        ("r11", ct.c_ulonglong),
        ("r10", ct.c_ulonglong),
        ("r9", ct.c_ulonglong),
        ("r8", ct.c_ulonglong),
        ("rax", ct.c_ulonglong),
        ("rcx", ct.c_ulonglong),
        ("rdx", ct.c_ulonglong),
        ("rsi", ct.c_ulonglong),
        ("rdi", ct.c_ulonglong),
        (
            "orig_rax",
            ct.c_ulonglong,
        ),
        ("rip", ct.c_ulonglong),
        ("cs", ct.c_ulonglong),
        ("eflags", ct.c_ulonglong),
        ("rsp", ct.c_ulonglong),
        ("ss", ct.c_ulonglong),
        ("fs_base", ct.c_ulonglong),
        ("gs_base", ct.c_ulonglong),
        ("ds", ct.c_ulonglong),
        ("es", ct.c_ulonglong),
        ("fs", ct.c_ulonglong),
        ("gs", ct.c_ulonglong),
    ]


# elf_greg_t	= ct.c_ulonglong
# ELF_NGREG = ct.sizeof(user_regs_struct)/ctypes.sizeof(elf_greg_t)
# elf_gregset_t = elf_greg_t*ELF_NGREG
elf_gregset_t = user_regs_struct


class elf_prstatus(ct.Structure):
    _fields_ = [
        (
            "pr_info",
            elf_siginfo,
        ),
        (
            "pr_cursig",
            ct.c_short,
        ),
        (
            "pr_sigpend",
            ct.c_ulong,
        ),
        (
            "pr_sighold",
            ct.c_ulong,
        ),
        ("pr_pid", ct.c_int),
        ("pr_ppid", ct.c_int),
        ("pr_pgrp", ct.c_int),
        ("pr_sid", ct.c_int),
        (
            "pr_utime",
            timeval,
        ),
        (
            "pr_stime",
            timeval,
        ),
        (
            "pr_cutime",
            timeval,
        ),
        (
            "pr_cstime",
            timeval,
        ),
        (
            "pr_reg",
            elf_gregset_t,
        ),
        (
            "pr_fpvalid",
            ct.c_int,
        ),
    ]


# elf_prpsinfo related constants.

ELF_PRARGSZ = 80  # #define ELF_PRARGSZ     (80)    /* Number of chars for args.  */


class elf_prpsinfo(ct.Structure):
    _fields_ = [
        (
            "pr_state",
            ct.c_byte,
        ),
        (
            "pr_sname",
            ct.c_char,
        ),
        (
            "pr_zomb",
            ct.c_byte,
        ),
        (
            "pr_nice",
            ct.c_byte,
        ),
        (
            "pr_flag",
            ct.c_ulong,
        ),
        ("pr_uid", ct.c_uint),
        ("pr_gid", ct.c_uint),
        ("pr_pid", ct.c_int),
        ("pr_ppid", ct.c_int),
        ("pr_pgrp", ct.c_int),
        ("pr_sid", ct.c_int),
        (
            "pr_fname",
            ct.c_char * 16,
        ),
        (
            "pr_psargs",
            ct.c_char * ELF_PRARGSZ,
        ),
    ]


class user_fpregs_struct(ct.Structure):
    _fields_ = [
        ("cwd", ct.c_ushort),
        ("swd", ct.c_ushort),
        ("ftw", ct.c_ushort),
        ("fop", ct.c_ushort),
        ("rip", ct.c_ulonglong),
        ("rdp", ct.c_ulonglong),
        ("mxcsr", ct.c_uint),
        ("mxcr_mask", ct.c_uint),
        (
            "st_space",
            ct.c_uint * 32,
        ),
        (
            "xmm_space",
            ct.c_uint * 64,
        ),
        ("padding", ct.c_uint * 24),
    ]


elf_fpregset_t = user_fpregs_struct


# siginfo_t related constants.

_SI_MAX_SIZE = 128
_SI_PAD_SIZE = (_SI_MAX_SIZE // ct.sizeof(ct.c_int)) - 4


#          /* kill().  */
class _siginfo_t_U_kill(ct.Structure):
    _fields_ = [
        (
            "si_pid",
            ct.c_int,
        ),
        (
            "si_uid",
            ct.c_uint,
        ),
    ]


# Type for data associated with a signal.
class sigval_t(ct.Union):
    _fields_ = [
        ("sival_int", ct.c_int),
        ("sical_ptr", ct.c_void_p),
    ]


class _siginfo_t_U_timer(ct.Structure):
    _fields_ = [
        ("si_tid", ct.c_int),
        (
            "si_overrun",
            ct.c_int,
        ),
        ("si_sigval", sigval_t),
    ]


class _siginfo_t_U_rt(ct.Structure):
    _fields_ = [
        (
            "si_pid",
            ct.c_int,
        ),
        (
            "si_uid",
            ct.c_uint,
        ),
        ("si_sigval", sigval_t),
    ]


class _siginfo_t_U_sigchld(ct.Structure):
    _fields_ = [
        ("si_pid", ct.c_int),
        (
            "si_uid",
            ct.c_uint,
        ),
        (
            "si_status",
            ct.c_int,
        ),
        ("si_utime", ct.c_long),
        ("si_stime", ct.c_long),
    ]


class _siginfo_t_U_sigfault(ct.Structure):
    _fields_ = [
        (
            "si_addr",
            ct.c_void_p,
        ),
        (
            "si_addr_lsb",
            ct.c_short,
        ),
    ]


class _siginfo_t_U_sigpoll(ct.Structure):
    _fields_ = [
        (
            "si_band",
            ct.c_long,
        ),
        ("si_fd", ct.c_int),
    ]


class _siginfo_t_U_sigsys(ct.Structure):
    _fields_ = [
        (
            "_call_addr",
            ct.c_void_p,
        ),
        (
            "_syscall",
            ct.c_int,
        ),
        (
            "_arch",
            ct.c_uint,
        ),
    ]


class _siginfo_t_U(ct.Union):
    _fields_ = [
        ("_pad", ct.c_int * _SI_PAD_SIZE),
        ("_kill", _siginfo_t_U_kill),
        ("_timer", _siginfo_t_U_timer),
        ("_rt", _siginfo_t_U_rt),
        ("_sigchld", _siginfo_t_U_sigchld),
        ("_sigfault", _siginfo_t_U_sigfault),
        ("_sigpoll", _siginfo_t_U_sigpoll),
        ("_sigsys", _siginfo_t_U_sigpoll),
    ]


class siginfo_t(ct.Structure):
    _fields_ = [
        ("si_signo", ct.c_int),
        (
            "si_errno",
            ct.c_int,
        ),
        ("si_code", ct.c_int),
        ("_sifields", _siginfo_t_U),
    ]


class ymmh_struct(ct.Structure):
    _fields_ = [
        (
            "ymmh_space",
            64 * ct.c_uint,
        )
    ]


class xsave_hdr_struct(ct.Structure):
    _fields_ = [
        (
            "xstate_bv",
            ct.c_ulonglong,
        ),
        (
            "reserved1",
            ct.c_ulonglong * 2,
        ),
        (
            "reserved2",
            ct.c_ulonglong * 5,
        ),
    ]


class i387_fxsave_struct(ct.Structure):
    _fields_ = [
        (
            "cwd",
            ct.c_ushort,
        ),
        (
            "swd",
            ct.c_ushort,
        ),
        (
            "twd",
            ct.c_ushort,
        ),
        (
            "fop",
            ct.c_ushort,
        ),
        (
            "rip",
            ct.c_ulonglong,
        ),
        (
            "rdp",
            ct.c_ulonglong,
        ),
        (
            "mxcsr",
            ct.c_uint,
        ),
        (
            "mxcsr_mask",
            ct.c_uint,
        ),
        (
            "st_space",
            ct.c_uint * 32,
        ),
        (
            "xmm_space",
            ct.c_uint * 64,
        ),
        (
            "padding",
            ct.c_uint * 12,
        ),
        (
            "padding1",
            ct.c_uint * 12,
        ),
    ]


class elf_xsave_struct(ct.Structure):
    _fields_ = [
        ("i387", i387_fxsave_struct),
        (
            "xsave_hdr",
            xsave_hdr_struct,
        ),
        ("ymmh", ymmh_struct),
    ]


class KVMSegment(ct.Structure):
    _fields_ = [
        ("base", ct.c_uint64),
        ("limit", ct.c_uint32),
        ("selector", ct.c_uint16),
        ("type", ct.c_uint8),
        ("present", ct.c_uint8),
        ("dpl", ct.c_uint8),
        # Default operation size (1 = 32bit, 0 = 16bit)
        ("db", ct.c_uint8),
        # 0 = system segment, 1 = data/code segment
        ("s", ct.c_uint8),
        # 1 = 64-bit
        ("l", ct.c_uint8),
        # Granularity, 1 = 4KB, 0 = 1 byte
        ("g", ct.c_uint8),
        ("avl", ct.c_uint8),
        ("unusable", ct.c_uint8),
        ("padding", ct.c_uint8),
    ]


class KVMDTable(ct.Structure):
    _fields_ = [
        ("base", ct.c_uint64),
        ("limit", ct.c_uint16),
        ("padding", ct.c_uint16 * 3),
    ]


KVM_NR_INTERRUPTS = 256


class KVMSRegs(ct.Structure):
    _fields_ = [
        ("cs", KVMSegment),
        ("ds", KVMSegment),
        ("es", KVMSegment),
        ("fs", KVMSegment),
        ("gs", KVMSegment),
        ("ss", KVMSegment),
        ("tr", KVMSegment),
        ("ldt", KVMSegment),
        ("gdt", KVMDTable),
        ("idt", KVMDTable),
        ("cr0", ct.c_uint64),
        ("cr2", ct.c_uint64),
        ("cr3", ct.c_uint64),
        ("cr4", ct.c_uint64),
        ("cr8", ct.c_uint64),
        ("efer", ct.c_uint64),
        ("apic_base", ct.c_uint64),
        ("interrupt_bitmap", ct.c_uint64 * ((KVM_NR_INTERRUPTS + 63) // 64)),
    ]


class kvm_msr_entry(ct.Structure):
    _fields_ = [
        ("index", ct.c_uint32),
        ("reserved", ct.c_uint32),
        ("data", ct.c_uint64),
    ]


class core_user(ct.Structure):
    _fields_ = [
        ("vpu", ct.c_size_t),
        ("sregs", KVMSRegs),
        ("msrs", kvm_msr_entry * 1),
    ]
