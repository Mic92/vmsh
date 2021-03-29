#!/usr/bin/env python3
import ctypes as ct
from typing import IO, List, Optional, Union, overload, Iterator, Iterable

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


class Memory(Iterable):
    def __init__(self, data: bytes, offset: int) -> None:
        self.data = data
        self.offset = offset

    def index(self, needle: bytes) -> int:
        return self.data.index(needle) + self.offset

    def __repr__(self) -> str:
        mib = len(self.data) / 1024 / 1024
        return f"Memory<0x{self.offset:x} - 0x{self.offset + len(self.data):x}, {mib:.2f} MiB>"

    def __iter__(self) -> Iterator[int]:
        return iter(self.data)

    def __reversed__(self) -> Iterator[int]:
        return reversed(self.data)

    @overload
    def __getitem__(self, index: int) -> int:
        ...

    @overload
    def __getitem__(self, index: slice) -> "Memory":
        ...

    def __getitem__(self, key: Union[int, slice]) -> Union[int, "Memory"]:
        if isinstance(key, slice):
            d = self.data[key.start - self.offset : key.stop - self.offset : key.step]
            return Memory(d, self.offset)
        elif isinstance(key, int):
            return self.data[key - self.offset]
        else:
            raise TypeError("Expected int or slice got: {}", key)

    def __len__(self) -> int:
        return len(self.data)


class ElfCore:
    """
    Not a general purpose coredump parser, but specialized on what we generate int the
    coredump subcommand.
    """

    regs: List["user_regs_struct"] = []
    fpu_regs: List["user_fpregs_struct"] = []
    special_regs: List["KVMSRegs"] = []
    msrs: List[List["kvm_msr_entry"]] = []

    def map_segment(self, seg: Segment) -> Memory:
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
        data = mm[(file_offset - offset) : (file_size - delta)]
        return Memory(data, seg.header.p_paddr)

    def find_segment_by_addr(self, phys_addr: int) -> Optional[Segment]:
        for seg in self.elf.iter_segments():
            if seg.header["p_type"] != "PT_LOAD":
                continue
            # x86_64-specific
            start = seg.header.p_paddr
            if phys_addr >= seg.header.p_vaddr and phys_addr < (
                start + seg.header.p_memsz
            ):
                return seg
        return None

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
