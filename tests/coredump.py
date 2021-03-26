#!/usr/bin/env python3
import ctypes as ct
from typing import IO, List

import mmap
import resource
from elftools.elf.elffile import ELFFile
from elftools.elf.segments import NoteSegment, Segment

from coredump_structs import (
    elf_prstatus,
    elf_fpregset_t,
    core_user,
    user_regs_struct,
    user_fpregs_struct,
    kvm_msr_entry,
    KVMSRegs,
)

NT_PRXFPREG = 1189489535


def page_start(v: int) -> int:
    return v & ~(resource.getpagesize() - 1)


def page_offset(v: int) -> int:
    return v & (resource.getpagesize() - 1)


def page_align(v: int) -> int:
    return (v + resource.getpagesize() - 1) & ~(resource.getpagesize() - 1)


class ElfCore:
    """
    Not a general purpose coredump parser, but specialized on what we generate int the
    coredump subcommand.
    """

    regs: List["user_regs_struct"] = []
    fpu_regs: List["user_fpregs_struct"] = []
    special_regs: List["KVMSRegs"] = []
    msrs: List[List["kvm_msr_entry"]] = []

    def map_segment(self, seg: Segment) -> bytes:
        file_offset = seg.header.p_offset
        file_size = seg.header.p_filesz
        offset = page_start(file_offset)
        delta = file_offset - offset
        mm = mmap.mmap(
            self.fd.fileno(),
            length=seg.header.p_filesz + delta,
            prot=mmap.PROT_READ,
            offset=offset,
        )
        # remove page alignments needed for mmap again
        return mm[(file_offset - offset) : (file_size - delta)]

    def __init__(self, fd: IO[bytes]) -> None:
        self.fd = fd
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
