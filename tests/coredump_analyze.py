#!/usr/bin/env python3

import os
import sys

sys.path.append(os.path.join(os.path.dirname(__file__), "tests"))

import ctypes as ct
import sys
from typing import IO, Tuple, Optional, Iterator, Dict
from dataclasses import dataclass

from coredump import ElfCore, Memory, MappedMemory, KVMSRegs
from cpu_flags import (
    _PAGE_ACCESSED,
    _PAGE_NX,
    _PAGE_PCD,
    _PAGE_PRESENT,
    _PAGE_PSE,
    _PAGE_PWT,
    _PAGE_RW,
    _PAGE_USER,
    X86_CR4_PCIDE,
)
from intervaltree import Interval, IntervalTree


def is_printable(byte: int) -> bool:
    return 0x20 < byte < 0x7E


def round_up(num: int, align: int) -> int:
    return ((num + align - 1) // align) * align


def find_ksymtab_strings_section(mem: Memory) -> Tuple[int, int]:
    """
    Find where in the memory the ksymtab_strings section is loaded.
    """
    try:
        idx = mem.index(b"init_task")
    except ValueError:
        raise RuntimeError("could not find ksymtab_strings")

    unprintable = 0
    for start_offset, byte in enumerate(reversed(mem[mem.start : idx])):
        if is_printable(byte):
            unprintable = 0
        else:
            unprintable += 1
        if unprintable == 2:
            break

    # In case we mis-detect the start of this section (for example if an data is printable by chance),
    # we round up the start of the string to the nearest 4 bytes,
    # as ksymtab or kcrctab are always four byte aligned
    start = round_up(idx - start_offset + 1, 4)

    for end_offset, byte in enumerate(mem[idx : mem.end]):
        if is_printable(byte):
            unprintable = 0
        else:
            unprintable += 1
            if unprintable == 2:
                break
    end = idx + end_offset
    return (start, end)


def count_ksymtab_strings(mem: Memory) -> int:
    count = 0
    for byte in mem:
        if byte == 0x0:
            count += 1
    return count


# A 4k intel page table with 512 64bit entries.
PAGE_TABLE_SIZE = 512
PageTableEntries = ct.c_uint64 * PAGE_TABLE_SIZE


def page_table(
    mem: Memory, addr: int, virt_addr: int = 0, level: int = 0
) -> "PageTable":
    end = addr + ct.sizeof(PageTableEntries)
    entries = PageTableEntries.from_buffer_copy(mem[addr:end].data)
    return PageTable(mem, entries, virt_addr, level)


@dataclass
class PageTableEntry:
    # raw value of the page table entry
    value: int
    # virtual address prefix
    virt_addr: int
    # Page directory level
    level: int

    def page_table(self, mem: Memory) -> "PageTable":
        """
        Page table of the next page directory level
        """
        assert self.level >= 0 and self.level < 3 and self.value & _PAGE_PRESENT
        return page_table(mem, self.phys_addr, self.virt_addr, self.level + 1)

    @property
    def phys_addr(self) -> int:
        """
        Physical address of the page table entry
        """
        return self.value & PHYS_ADDR_MASK

    @property
    def size(self) -> int:
        """
        Size of the page this page table entry points to
        """
        return 1 << get_shift(self.level)

    def __repr__(self) -> str:
        v = self.value
        rw = "W" if (v & _PAGE_RW) else "R"
        user = "U" if (v & _PAGE_USER) else "K"
        pwt = "PWT" if (v & _PAGE_PWT) else ""
        pcd = "PCD" if (v & _PAGE_PCD) else ""
        accessed = "A" if (v & _PAGE_ACCESSED) else ""
        nx = "NX" if (v & _PAGE_NX) else ""

        ranges = list(memory_layout.at(self.virt_addr))
        description = ranges[0].data

        return f"0x{self.phys_addr:x} -> 0x{self.virt_addr:x} {rw} {user} {pwt} {pcd} {accessed} {nx} {description}"


def get_shift(level: int) -> int:
    assert level >= 0 and level <= 3
    return 12 + 9 * (3 - level)


def get_index(virt: int, level: int) -> int:
    return virt >> get_shift(level) & 0x1FF


class PageTable:
    def __init__(
        self,
        mem: Memory,
        entries: "ct.Array[ct.c_uint64]",
        virt_addr_prefix: int,
        level: int,
    ) -> None:
        self.mem = mem
        self.entries = entries
        self.level = level
        self.virt_addr_prefix = virt_addr_prefix

    def __getitem__(self, idx: int) -> Optional[PageTableEntry]:
        e = self.entries[idx]
        if e & _PAGE_PRESENT == 0:
            return None
        virt_addr = self.virt_addr_prefix + (idx << get_shift(self.level))

        # sign extend most significant bit
        if virt_addr >> 47:
            virt_addr |= 0xFFFF << 48
        return PageTableEntry(e, virt_addr, self.level)

    def __iter__(self) -> Iterator[PageTableEntry]:
        for i in range(512):
            e = self[i]
            if not e:
                continue
            if self.level == 3 or e.value & _PAGE_PSE:
                yield e
                continue
            for e in e.page_table(self.mem):
                yield e


PHYS_ADDR_MASK = 0xFFFFFFFFFF000


def dump_page_table(pml4: PageTable) -> None:
    for e in pml4:
        print("  " * e.level + str(e))


# TODO: x86_64 specific
PHYS_LOAD_ADDR = 0x100000

# computed with kernel.py from https://github.com/Mic92/x86_64-linux-cheatsheats/
FIXADDR_START = 0xFFFFFFFFFF57A000


# https://www.kernel.org/doc/Documentation/x86/x86_64/mm.txt
# TODO: this is for kalsr disabled only afaik
memory_layout = IntervalTree(
    [
        Interval(0x0000000000000000, 0x00007FFFFFFFFFFF, "userspace"),
        Interval(0x0000800000000000, 0xFFFF7FFFFFFFFFFF, "hole 1"),
        Interval(0xFFFF800000000000, 0xFFFF87FFFFFFFFFF, "guard hole (for hypervisor)"),
        Interval(0xFFFF880000000000, 0xFFFF887FFFFFFFFF, "LDT remap for PTI"),
        Interval(
            0xFFFF888000000000,
            0xFFFFC87FFFFFFFFF,
            "direct mapping of all physical memory",
        ),
        Interval(0xFFFFC88000000000, 0xFFFFC8FFFFFFFFFF, "hole 2"),
        Interval(
            0xFFFFC90000000000,
            0xFFFFE8FFFFFFFFFF,
            "vmalloc/ioremap space (vmalloc_base)",
        ),
        Interval(0xFFFFE90000000000, 0xFFFFE9FFFFFFFFFF, "hole 3"),
        Interval(
            0xFFFFEA0000000000, 0xFFFFEAFFFFFFFFFF, "virtual memory map (vmemmap_base)"
        ),
        Interval(0xFFFFEB0000000000, 0xFFFFEBFFFFFFFFFF, "hole 4"),
        Interval(0xFFFFEC0000000000, 0xFFFFFBFFFFFFFFFF, "KASAN shadow memory"),
        Interval(0xFFFFFC0000000000, 0xFFFFFDFFFFFFFFFF, "hole 5"),
        Interval(0xFFFFFE0000000000, 0xFFFFFE7FFFFFFFFF, "cpu_entry_area mapping"),
        Interval(0xFFFFFE8000000000, 0xFFFFFEFFFFFFFFFF, "hole 6"),
        Interval(0xFFFFFF0000000000, 0xFFFFFF7FFFFFFFFF, "%esp fixup stacks"),
        Interval(0xFFFFFF8000000000, 0xFFFFFFEEFFFFFFFF, "hole 7"),
        Interval(0xFFFFFFEF00000000, 0xFFFFFFFEFFFFFFFF, "EFI region mapping space"),
        Interval(0xFFFFFFFF00000000, 0xFFFFFFFF7FFFFFFF, "hole 8"),
        Interval(
            0xFFFFFFFF80000000,
            0xFFFFFFFF9FFFFFFF,
            "kernel text mapping, mapped to physical address 0",
        ),
        Interval(0xFFFFFFFF9FFFFFFF, 0xFFFFFFFFA0000000, "hole 9"),
        Interval(0xFFFFFFFFA0000000, 0xFFFFFFFFFEFFFFFF, "module mapping space"),
        Interval(0xFFFFFFFFFF000000, FIXADDR_START, "hole 10"),
        Interval(
            FIXADDR_START,
            0xFFFFFFFFFF5FFFFF,
            "kernel-internal fixmap range, variable size and offset",
        ),
        Interval(0xFFFFFFFFFF600000, 0xFFFFFFFFFF600FFF, "legacy vsyscall ABI"),
        Interval(0xFFFFFFFFFFE00000, 0xFFFFFFFFFFFFFFFF, "hole 11"),
    ]
)

LINUX_KERNEL_KASLR_RANGE = Interval(0xFFFFFFFF80000000, 0xFFFFFFFFC0000000)


def get_page_table_addr(sregs: KVMSRegs) -> int:
    if sregs.cr4 & X86_CR4_PCIDE:
        return sregs.cr3 & PHYS_ADDR_MASK
    else:
        return sregs.cr3


def find_linux_kernel_memory(
    pml4: PageTable, mem: Memory, mem_range: Interval
) -> Optional[MappedMemory]:
    """
    Return virtual and physical memory
    """
    # TODO: skip first level in page tables to speed up the search
    # i = get_index(mem_range.begin, 0)
    # pdt = page_table(mem, pml4.entries[i] & PHYS_ADDR_MASK)
    it = iter(pml4)
    first: Optional[PageTableEntry] = None
    for entry in it:
        if entry.virt_addr >= mem_range.begin:
            first = entry
            break
    if first is None:
        return None
    last: PageTableEntry = first
    for entry in it:
        if entry.virt_addr > mem_range.end:
            break
        if last.phys_addr + last.size != entry.phys_addr:
            raise Exception(
                "Kernel is not in physical-continous memory. This is not implemented."
            )
        last = entry
    phys_mem = mem[first.phys_addr : last.phys_addr + last.size]
    return phys_mem.map(first.virt_addr)


# FIXME: on many archs, especially 32-bit ones, this layout is used!
# struct kernel_symbol {
#   unsigned long value;
#   const char *name;
#   const char *namespace;
# };

# From include/linux/export.h
class kernel_symbol(ct.Structure):
    _fields_ = [
        ("value_offset", ct.c_uint),
        ("name_offset", ct.c_uint),
        ("namespace_offset", ct.c_uint),
    ]


def get_name_addr(mem: MappedMemory, idx: int) -> int:
    sym_size = ct.sizeof(kernel_symbol)
    addr = idx - sym_size
    sym = kernel_symbol.from_buffer_copy(mem[addr:idx].data)
    name_addr = sym.name_offset + kernel_symbol.name_offset.offset + addr
    print(f"0x{mem.virt_addr(addr):x} - 0x{name_addr:x} ({sym.name_offset:=})")
    return name_addr


def get_ksymtab_start(
    mem: MappedMemory, ksymtab_strings: MappedMemory
) -> Optional[int]:
    """
    Skips over kcrctab if present and retrieves actual ksymtab offset. This is
    done by casting each offset to a kernel_symbole and check if its name_offset
    would fall into the ksymtab_string address range.
    """
    sym_size = ct.sizeof(kernel_symbol)
    # each entry in kcrctab is 32 bytes
    step_size = ct.sizeof(ct.c_int32)
    for ii in range(ksymtab_strings.start, mem.start, -step_size):
        name_addr = get_name_addr(mem, ii)

        if ksymtab_strings.inrange(name_addr) and ii - mem.start > step_size:
            name_addr_2 = get_name_addr(mem, ii - sym_size)
            if ksymtab_strings.inrange(name_addr_2):
                return ii
    return None


# algorithm for reconstructing function pointers;
# - assuming we have HAVE_ARCH_PREL32_RELOCATIONS (is the case on arm64 and x86_64)
# - Get Address of last symbol in __ksymtab_string
# - if CONFIG_MODVERSIONS we have __kcrctab, otherwise no (seems to be the case on Ubuntu, probably not in most microvm kernels i.e. kata containers)
# - Maybe not implement crc for but add a check if symbols in ksymtab_string have weird subfixes: i.e. printk_R1b7d4074 instead of printk ?
# - Is __ksymtab seems not to be at a predictable offsets?
#
# 0xffffffff80000000–0xffffffffc0000000
# dump_page_table(pml4, mem)
#
# Layout of __ksymtab,  __ksymtab_gpl
# struct kernel_symbol {
#    int value_offset;
#    int name_offset;
#    int namespace_offset;
# };
#
# To convert an offset to its pointer: ptr = (unsigned long)&sym.offset + sym.offset
#
# Layout of __ksymtab,  __ksymtab_gpl
# __ksymtab_strings
# null terminated, strings
def get_kernel_symbols(
    mem: MappedMemory, ksymtab_strings: MappedMemory
) -> Dict[str, int]:
    # We validate kernel symbols here by checking if the name_offset
    # points into the ksymtab_strings range.
    sym_size = ct.sizeof(kernel_symbol)
    syms: Dict[str, int] = {}
    # skip kcrctab if there
    ksymtab_start = get_ksymtab_start(mem, ksymtab_strings)
    if ksymtab_start is None:
        return {}

    print(
        f"found ksymtab at physical address: 0x{ksymtab_start:x} / virtual address: 0x{mem.virt_addr(ksymtab_start):x}, {ksymtab_strings.start - ksymtab_start} bytes before ksymtab_strings"
    )

    for ii in range(ksymtab_start, mem.start, -ct.sizeof(kernel_symbol)):
        addr = ii - sym_size
        sym = kernel_symbol.from_buffer_copy(mem[addr:ii].data)

        name_addr = sym.name_offset + kernel_symbol.name_offset.offset + addr
        value_addr = sym.value_offset + kernel_symbol.value_offset.offset + addr
        # namespace_addr = sym.namespace_offset + kernel_symbol.namespace_offset.offset + addr

        name_len = 0
        if not ksymtab_strings.inrange(name_addr):
            # print(name_addr)
            break
        for b in ksymtab_strings[name_addr : ksymtab_strings.end]:
            if b == 0x0:
                break
            name_len += 1

        name = ksymtab_strings[name_addr : name_addr + name_len].data.decode("ascii")
        syms[name] = value_addr
        # print(f"{name} @ 0x{value_addr:x}")
    return syms


def inspect_coredump(fd: IO[bytes]) -> None:
    core = ElfCore(fd)
    vm_segment = core.find_segment_by_addr(PHYS_LOAD_ADDR)
    assert vm_segment is not None, "cannot find physical memory of VM in coredump"
    mem = core.map_segment(vm_segment)

    pt_addr = get_page_table_addr(core.special_regs[0])
    pt_segment = core.find_segment_by_addr(pt_addr)
    assert pt_segment is not None
    # for simplicity we assume that the page table is also in this main vm allocation
    assert vm_segment.header == pt_segment.header
    cpl = core.regs[0].cs & 3
    if cpl == 0 or cpl == 1:
        print("program run in privileged mode")
    else:  # cpl == 3:
        print("program runs userspace")
    print(f"rip=0x{core.regs[0].rip:x}")
    pml4 = page_table(mem, pt_addr)
    print("look for kernel in...")
    kernel_memory = find_linux_kernel_memory(pml4, mem, LINUX_KERNEL_KASLR_RANGE)
    if kernel_memory:
        print(f"Found at {kernel_memory.start:x}:{kernel_memory.end:x}")
    else:
        print("could not find kernel!")
        sys.exit(1)

    strings_start_addr, strings_end_addr = find_ksymtab_strings_section(kernel_memory)
    ksymtab_strings_section = kernel_memory[strings_start_addr:strings_end_addr]
    string_num = count_ksymtab_strings(ksymtab_strings_section)
    print(
        f"found ksymtab_string at physical 0x{strings_start_addr:x}:0x{strings_end_addr:x} with {string_num} strings"
    )
    symbols = get_kernel_symbols(kernel_memory, ksymtab_strings_section)
    print(f"found ksymtab with {len(symbols)} functions")


def main() -> None:
    if len(sys.argv) < 2:
        print(f"USAGE: {sys.argv[0]} coredump", file=sys.stderr)
        sys.exit(1)
    with open(sys.argv[1], "rb") as f:
        inspect_coredump(f)


if __name__ == "__main__":
    main()
