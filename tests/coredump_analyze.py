#!/usr/bin/env python3

import os
import sys

sys.path.append(os.path.join(os.path.dirname(__file__), "tests"))

import ctypes as ct
import sys
from typing import IO, Tuple

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


class PageTable:
    def __init__(self, mem: Memory) -> None:
        self.entries = PageTableEntries.from_buffer_copy(mem.data)


def page_table(mem: Memory, addr: int) -> PageTable:
    if addr == 0:
        breakpoint()
    end = addr + ct.sizeof(PageTableEntries)
    return PageTable(mem[addr:end])


def dump_page_table_entry(e: int, addr: int, level: int) -> None:
    rw = "W" if (e & _PAGE_RW) else "R"
    user = "U" if (e & _PAGE_USER) else "K"
    pwt = "PWT" if (e & _PAGE_PWT) else ""
    pcd = "PCD" if (e & _PAGE_PCD) else ""
    accessed = "A" if (e & _PAGE_ACCESSED) else ""
    nx = "NX" if (e & _PAGE_NX) else ""
    str_level = "  " * level
    description = list(memory_layout.at(addr))[0].data
    print(
        f"{str_level} 0x{addr:x} {rw} {user} {pwt} {pcd} {accessed} {nx} {description}",
    )


def dump_page_table(pml4: PageTable, memory: Memory) -> None:
    bm = ((1 << (51 - 12 + 1)) - 1) << 12
    for i in range(512):
        if pml4.entries[i] & _PAGE_PRESENT == 0:
            continue

        # sign extend most significant bit
        if (i << 39) & (1 << 47):
            addr = 0xFFFF << 48
        else:
            addr = 0
        addr |= i << (12 + 9 * 3)
        if pml4.entries[i] & _PAGE_PSE:
            dump_page_table_entry(pml4.entries[i], addr, 0)

        pdt = page_table(memory, pml4.entries[i] & bm)
        for j in range(512):
            if pdt.entries[j] & _PAGE_PRESENT == 0:
                continue
            addr |= j << (12 + 9 * 2)
            if pdt.entries[j] & _PAGE_PSE:
                dump_page_table_entry(pdt.entries[j], addr, 1)
                continue
            pd = page_table(memory, pdt.entries[j] & bm)
            for k in range(512):
                if pd.entries[k] & _PAGE_PRESENT == 0:
                    continue
                addr |= k << (12 + 9 * 1)
                if pd.entries[j] & _PAGE_PSE:
                    dump_page_table_entry(pd.entries[k], addr, 2)
                    continue
                pt = page_table(memory, pd.entries[k] & bm)
                for l in range(512):
                    if pt.entries[l] & _PAGE_PRESENT == 0:
                        continue
                    addr |= l << 12
                    dump_page_table_entry(pt.entries[l], addr, 3)


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


def get_page_table_addr(sregs: KVMSRegs) -> int:
    if sregs.cr4 & X86_CR4_PCIDE:
        return sregs.cr3 & 0x000FFFFFFFFFF000
    else:
        return sregs.cr3


def inspect_coredump(fd: IO[bytes]) -> None:
    core = ElfCore(fd)
    vm_segment = core.find_segment_by_addr(PHYS_LOAD_ADDR)
    assert vm_segment is not None, "cannot find physical memory of VM in coredump"
    mem = core.map_segment(vm_segment)
    start_addr, end_addr = find_ksymtab_strings(mem)
    print(f"found ksymtab at 0x{start_addr:x}:0x{end_addr:x}")

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
    pml4 = page_table(mem, pt_addr)
    dump_page_table(pml4, mem)


def main() -> None:
    if len(sys.argv) < 2:
        print(f"USAGE: {sys.argv[0]} coredump", file=sys.stderr)
        sys.exit(1)
    with open(sys.argv[1], "rb") as f:
        inspect_coredump(f)


if __name__ == "__main__":
    main()
