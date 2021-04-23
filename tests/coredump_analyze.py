#!/usr/bin/env python3

import os
import sys

sys.path.append(os.path.join(os.path.dirname(__file__), "tests"))

import ctypes as ct
import sys
from typing import IO, Tuple, Optional, Iterator
from dataclasses import dataclass

from coredump import ElfCore, Memory, KVMSRegs
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


def find_ksymtab_strings(mem: Memory) -> Tuple[int, int]:
    try:
        idx = mem.index(b"init_task")
    except ValueError:
        raise RuntimeError("could not find ksymtab_strings")

    unprintable = 0
    for start_offset, byte in enumerate(reversed(mem[mem.offset : idx])):
        if is_printable(byte):
            unprintable = 0
        else:
            unprintable += 1
        if unprintable == 2:
            break
    start = idx - start_offset + 1

    for end_offset, byte in enumerate(mem[idx : len(mem)]):
        if is_printable(byte):
            unprintable = 0
        else:
            unprintable += 1
        if unprintable == 2:
            break
    end = idx + end_offset - 1
    return (start, end)


def get_kernel_base(kaddr: int) -> int:
    # https://github.com/bcoles/kernel-exploits/blob/master/CVE-2017-1000112/poc.c#L612
    return (kaddr & 0xFFFFFFFFFFF00000) - 0x1000000


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
    value: int
    virt_addr: int
    level: int

    def page_table(self, mem: Memory) -> "PageTable":
        assert self.level >= 0 and self.level < 3 and self.value & _PAGE_PRESENT
        return page_table(mem, self.phys_addr, self.virt_addr, self.level + 1)

    @property
    def phys_addr(self) -> int:
        return self.value & PHYS_ADDR_MASK

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
            if self.level == 4 or e.value & _PAGE_PSE:
                yield e
                continue
            if self.level < 3:
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


def find_linux_kernel_offset(
    pml4: PageTable, mem: Memory, mem_range: Interval
) -> Optional[Interval]:
    # TODO: skip first level in page tables to speed up the search
    # i = get_index(mem_range.begin, 0)
    # pdt = page_table(mem, pml4.entries[i] & PHYS_ADDR_MASK)
    it = iter(pml4)
    first = -1
    for entry in it:
        if entry.virt_addr >= mem_range.begin:
            first = entry.virt_addr
            break
    if first == -1:
        return None
    last = first
    for entry in it:
        if entry.virt_addr > mem_range.end:
            break
        last = entry.virt_addr
    return Interval(first, last)


def inspect_coredump(fd: IO[bytes]) -> None:
    core = ElfCore(fd)
    vm_segment = core.find_segment_by_addr(PHYS_LOAD_ADDR)
    assert vm_segment is not None, "cannot find physical memory of VM in coredump"
    mem = core.map_segment(vm_segment)
    start_addr, end_addr = find_ksymtab_strings(mem)
    print(f"found ksymtab at physical 0x{start_addr:x}:0x{end_addr:x}")

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
    offset = find_linux_kernel_offset(pml4, mem, LINUX_KERNEL_KASLR_RANGE)
    if offset:
        print(f"Found at {offset.begin:x}:{offset.end:x}")
    else:
        print("could not find kernel!")
        sys.exit(1)

    # 0xffffffff80000000–0xffffffffc0000000
    # dump_page_table(pml4, mem)


def main() -> None:
    if len(sys.argv) < 2:
        print(f"USAGE: {sys.argv[0]} coredump", file=sys.stderr)
        sys.exit(1)
    with open(sys.argv[1], "rb") as f:
        inspect_coredump(f)


if __name__ == "__main__":
    main()
