#!/usr/bin/env python3
import ctypes as ct
import mmap
import resource
from typing import IO, Iterable, Iterator, List, Optional, Union, overload

from coredump_structs import (
    KVMSRegs,
    core_user,
    elf_fpregset_t,
    elf_prstatus,
    kvm_msr_entry,
    user_fpregs_struct,
    user_regs_struct,
)
from elftools.elf.elffile import ELFFile
from elftools.elf.segments import NoteSegment, Segment

NT_PRXFPREG = 1189489535


def page_start(v: int) -> int:
    return v & ~(resource.getpagesize() - 1)


def page_offset(v: int) -> int:
    return v & (resource.getpagesize() - 1)


def page_align(v: int) -> int:
    return (v + resource.getpagesize() - 1) & ~(resource.getpagesize() - 1)


class Memory(Iterable):
    """
    A continous block of memory. Offset can be physical or virtual
    """

    def __init__(self, data: bytes, offset: int) -> None:
        self.data = data
        self.offset = offset

    @property
    def start(self) -> int:
        """
        Address where the memory start
        """
        return self.offset

    @property
    def end(self) -> int:
        """
        Address where the memory end
        """
        return self.offset + len(self.data)

    def index(self, needle: bytes) -> int:
        """
        Search bytes in memory. Returns address if found, otherwise raises IndexError
        """
        return self.data.index(needle) + self.offset

    def inrange(self, addr: int) -> bool:
        """
        True if address is contained in memory range
        """
        return addr >= self.start and addr < self.end

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
            assert key.start >= self.start, f"{key.start=} >= {self.start=}"
            assert key.start <= key.stop, f"{key.start=} <= {key.stop=}"
            assert key.stop <= self.end, f"{key.stop=} <= {self.end=}"
            d = self.data[key.start - self.offset : key.stop - self.offset : key.step]
            return Memory(d, key.start)
        elif isinstance(key, int):
            assert key >= self.start
            assert key < self.end
            return self.data[key - self.offset]
        else:
            raise TypeError("Expected int or slice got: {}", key)

    def __len__(self) -> int:
        return len(self.data)

    def map(self, virt_offset: int) -> "MappedMemory":
        return MappedMemory(self.data, self.offset, virt_offset)


class MappedMemory(Memory):
    """
    Contineous physical memory that is mapped virtual contineous
    """

    def __init__(self, data: bytes, phys_offset: int, virt_offset: int) -> None:
        super().__init__(data, phys_offset)
        self.virt_offset = virt_offset

    @overload
    def __getitem__(self, index: int) -> int:
        ...

    @overload
    def __getitem__(self, index: slice) -> "MappedMemory":
        ...

    def __getitem__(self, key: Union[int, slice]) -> Union[int, "MappedMemory"]:
        r = super().__getitem__(key)
        if isinstance(r, Memory):
            return r.map(self.virt_offset + r.offset - self.offset)
        return r

    def phys_addr(self, virt_addr: int) -> int:
        "translate virtual address to physical address"
        assert virt_addr >= self.virt_offset
        assert virt_addr < (self.virt_offset + len(self.data))
        return virt_addr + (self.offset - self.virt_offset)

    def virt_addr(self, phys_addr: int) -> int:
        "translate physical address to physical address"
        assert phys_addr >= self.offset
        assert phys_addr < (self.offset + len(self.data))
        return phys_addr + (self.virt_offset - self.offset)


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
