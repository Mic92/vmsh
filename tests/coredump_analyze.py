#!/usr/bin/env python3

import os
import sys

sys.path.append(os.path.join(os.path.dirname(__file__), "tests"))

import ctypes as ct
import sys
from enum import IntEnum
from typing import IO, Optional, Tuple

from coredump import ElfCore, Memory
from cpu_flags import (
    _PAGE_ACCESSED,
    _PAGE_NX,
    _PAGE_PCD,
    _PAGE_PRESENT,
    _PAGE_PSE,
    _PAGE_PWT,
    _PAGE_RW,
    _PAGE_USER,
    EFER_LME,
    EFER_NX,
    X86_CR0_PG,
    X86_CR3_PCD,
    X86_CR3_PWT,
    X86_CR4_PAE,
    X86_CR4_PCIDE,
    X86_CR4_PKE,
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


NT_PRXFPREG = 1189489535


def bitmask_numbits(numbits: int) -> int:
    return (1 << numbits) - 1


def entry_extract_flags(entry: int) -> int:
    # TODO dump more flags?
    return entry & (
        _PAGE_NX | _PAGE_RW | _PAGE_USER | _PAGE_PWT | _PAGE_PCD | _PAGE_ACCESSED
    )


# Might be wrong, kernel uses cpuid to figure this out
x86_phys_bits = 36  # my kvm had 28 max phys bits?
pte_reserved_flags = bitmask_numbits(
    51 - x86_phys_bits + 1
)  # bitmap with up to 51 bit set


def is_page_aligned(num: int) -> bool:
    return num & (4096 - 1) == num


# A 4k intel page table with 512 64bit entries.
PAGE_TABLE_SIZE = 512
PageTableEntries = ct.c_uint64 * PAGE_TABLE_SIZE


class PageTable:
    def __init__(self, mem: Memory) -> None:
        self.entries = PageTableEntries.from_buffer_copy(mem.data)
        # current index into the pagetable
        self._i = 0
        self.baddr = mem.offset

    @property
    def i(self) -> int:
        """
        virtual memory base address mapped by current pml4e
        """
        return self._i

    @i.setter
    def i(self, val: int) -> None:
        assert val >= 0 and val < 512
        self._i = val


class State:
    def __init__(self, mem: Memory, cr3: int) -> None:
        self.mem = mem
        self.pml4 = self.page_table(cr3)
        self.pdpt: Optional[PageTable] = None
        self.pd: Optional[PageTable] = None
        self.pt: Optional[PageTable] = None

        # compress output, don't print same entries all over
        self.last_addr: int = 0
        self.last_flags: int = 0
        self.skipped: int = 0
        self.entries: int = 0

    def page_table(self, addr: int) -> PageTable:
        assert addr > 0
        end = addr + ct.sizeof(PageTableEntries)
        return PageTable(self.mem[addr:end])

    def next_page_table(self, pagetbl_entry: int) -> PageTable:
        # pagetble_entry bits 51:12 contains the physical address of the next page
        # table level
        bm = bitmask_numbits(51 - 12 + 1) << 12
        # physical addr of page table entry
        paddr = pagetbl_entry & bm
        # Actually, 51:.. is too generous, there are some reserved bits which must be
        # zero
        # assert (pagetbl_entry & pte_reserved_flags) == 0

        if is_page_aligned(paddr):
            raise RuntimeError("CRITICAL: invalid addr {paddr}\n")

        return self.page_table(paddr)


# Each level of page tables is responsible for 9 bits of the virtual address.
# PML4 39:47 (inclusive)
# PDPT 30:38 (inclusive)
# PD   21:29 (inclusive)
# PT   12:20 (inclusive)
# (0:11 are the offset in a 4K page)
# The position of the bit of the virtual memory address that the page table
# level refers to.
class pt_addr_bit(IntEnum):
    PML4 = 39
    PDPT = 30
    PD = 21
    PT = 12


# The part of the virtual address defined by the page table entry at index for
# the page table level indicated by bitpos
def pte_addr_part(index: int, bitpos: pt_addr_bit) -> int:
    assert index < 512
    assert (
        bitpos == pt_addr_bit.PML4
        or bitpos == pt_addr_bit.PDPT
        or bitpos == pt_addr_bit.PD
        or bitpos == pt_addr_bit.PT
    )
    return index << bitpos


def check_entry(e: int) -> bool:
    # 51:M reserved, must be 0
    # if e & pte_reserved_flags:
    #    print("invalid entry!\n")
    #    breakpoint()
    #    return False
    if e & _PAGE_PSE:
        # TODO references a page directly, probably sanity check address?
        pass
    if not (e & _PAGE_PRESENT) and e:
        pass
        # print("strange entry!\n")
        # return False

    return True


def string_page_size(bitpos: pt_addr_bit) -> str:
    assert (
        bitpos == pt_addr_bit.PDPT
        or bitpos == pt_addr_bit.PD
        or bitpos == pt_addr_bit.PT
    )
    pagesize = 1 << bitpos
    if pagesize == 1024 * 1024 * 1024:
        return "1GB"
    elif pagesize == 2 * 1024 * 1024:
        return "2MB"
    elif pagesize == 4 * 1024:
        return "4KB"
    else:
        raise RuntimeError("BUG PAGESIZE")


def dump_entry(state: State, bitpos: pt_addr_bit) -> bool:
    str_level = ""
    ret = True  # continue descending

    baddr = 0  # pointer to state struct with base address of current entry. To be set in this function
    addr_max = 0  # maximum virtual address described by the current page table entry

    _direct_mapping = False  # entry maps a page directly

    if bitpos == pt_addr_bit.PML4:  # outer level
        table = state.pml4
        str_level = "pml4"
        outer_baddr = 0
        if pte_addr_part(table.i, pt_addr_bit.PML4) & (1 << 47):  # highest bit set
            outer_baddr = 0xFFFF << 48
    elif bitpos == pt_addr_bit.PDPT:
        assert state.pdpt is not None
        table = state.pdpt
        str_level = "  pdpt"
        outer_baddr = state.pml4.baddr
    elif bitpos == pt_addr_bit.PD:
        assert state.pd is not None and state.pdpt is not None
        table = state.pd
        str_level = "      pd"
        assert (state.pml4.baddr | state.pdpt.baddr) == state.pdpt.baddr
        outer_baddr = state.pdpt.baddr
    elif bitpos == pt_addr_bit.PT:  # final level
        assert (
            state.pml4.baddr is not None
            and state.pdpt is not None
            and state.pd is not None
            and state.pt is not None
        )
        table = state.pt
        str_level = "        pt"
        assert (state.pml4.baddr | state.pdpt.baddr | state.pd.baddr) == state.pd.baddr
        outer_baddr = state.pd.baddr

    e = table.entries[table.i]

    assert check_entry(e)

    if not (e & _PAGE_PRESENT):
        # skip page which is marked not present. Do not emit any output.
        return False

    table.baddr = outer_baddr | pte_addr_part(table.i, bitpos)
    assert outer_baddr & pte_addr_part(table.i, bitpos) == 0  # no overlapping bits
    # assert (
    #    state.pdpt is not None
    #    and (state.pml4.baddr | state.pdpt.baddr) & state.pml4.baddr == state.pml4.baddr
    # )  # no overlapping bits
    addr_max = bitmask_numbits(bitpos)
    # assert (state.pml4.baddr & addr_max) == 0  # no overlapping bits
    # assert (state.pdpt.baddr & addr_max) == 0  # no overlapping bits
    addr_max |= table.baddr

    if (e & _PAGE_PSE) or bitpos == pt_addr_bit.PT:
        # PSE for 2MB or 1GB direct mapping bitpos == PT, then the _PAGE_PSE bit
        # is the PAT bit. But for 4k pages, we always have a direct mapping
        _direct_mapping = True
        ret = False  # do not descend to any deeper page tables!

    if state.last_addr + 1 == table.baddr and state.last_flags == entry_extract_flags(
        e
    ):
        # don't print consecutive similar mappings
        state.skipped += 1
    else:
        if state.skipped:
            print(
                f"{str_level} skipped {state.skipped} following entries (til {state.last_addr:x})"
            )
        state.skipped = 0
        rw = "W" if (e & _PAGE_RW) else "R"
        user = "U" if (e & _PAGE_USER) else "K"
        pwt = "PWT" if (e & _PAGE_PWT) else ""
        pcd = "PCD" if (e & _PAGE_PCD) else ""
        accessed = "A" if (e & _PAGE_ACCESSED) else ""
        nx = "NX" if (e & _PAGE_NX) else ""
        print(
            f"{str_level} {baddr} {addr_max:x} {rw} {user} {pwt} {pcd} {accessed} {nx}",
            end="",
        )
        if _direct_mapping:
            print(f" -> {string_page_size(bitpos)} page", end="")
        print()
    state.entries += 1

    state.last_addr = addr_max
    state.last_flags = entry_extract_flags(e)

    return ret


def dump_page_table(core: ElfCore, memory: Memory) -> None:
    sregs = core.special_regs[0]
    # https://software.intel.com/sites/default/files/managed/39/c5/325462-sdm-vol-1-2abcd-3abcd.pdf pp.2783 March 2017, version 062

    # Chap 3.2.1 64-Bit Mode Execution Environment. Control registers expand to 64 bits
    cr0 = sregs.cr0
    cr3 = sregs.cr3
    cr4 = sregs.cr4
    ia32_efer = core.msrs[0][0].data

    if not ia32_efer & EFER_NX:
        print("No IA32_EFER.NXE?????")

    print(f"cr3: 0x{cr3:x}")
    #  reserved
    if cr3 & 0xFFFFFFFFFFFFFFFF << x86_phys_bits:
        print("c3 looks shady")

    if cr3 & X86_CR3_PWT or cr3 & X86_CR3_PCD:
        print("unexpected options in cr3")

    if (
        cr0 & X86_CR0_PG
        or cr4 & X86_CR4_PAE
        and ia32_efer & EFER_LME
        or not (cr4 & X86_CR4_PCIDE)
    ):
        print("paging according to Tab 4-12 p. 2783 intel dev manual (March 2017)\n")
    else:
        raise RuntimeError("unknown paging setup")
    if not (cr4 & X86_CR4_PKE):
        print("No protection keys enabled (this is normal)")

    state = State(memory, cr3)
    print(f"page table in virtual memory at 0x{state.pml4.baddr:x}")
    if is_page_aligned(cr3):  # TODO ?
        raise RuntimeError("invalid addr!")

    #  4 nested for loops to walk 4 levels of pages walk the outermost page table
    for i in range(0, PAGE_TABLE_SIZE):
        state.pml4.i = i
        if dump_entry(state, pt_addr_bit.PML4):
            # walk next level
            state.pdpt = state.next_page_table(state.pml4.entries[state.pml4.i])
            state.pdpt.i = i
            for i in range(0, PAGE_TABLE_SIZE):
                # walk next level
                state.pdpt.i = i
                if dump_entry(state, pt_addr_bit.PDPT):
                    state.pd = state.next_page_table(state.pdpt.entries[state.pdpt.i])
                    for i in range(0, PAGE_TABLE_SIZE):
                        state.pd.i = i
                        if dump_entry(state, pt_addr_bit.PD):
                            # print final level here
                            state.pt = state.next_page_table(
                                state.pd.entries[state.pd.i]
                            )
                            for i in range(0, PAGE_TABLE_SIZE):
                                state.pt.i = i
                                if dump_entry(state, pt_addr_bit.PT):
                                    raise RuntimeError("we cannot go deeper!")
                            #  reset pt entries in state for assertions
                            state.pt = None
                    # reset pd entries in state for assertions
                    state.pd = None
            # reset pdpt entries in state for assertions
            state.pdpt = None
    print(f"entries: {state.entries}")


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


def inspect_coredump(fd: IO[bytes]) -> None:
    core = ElfCore(fd)
    vm_segment = core.find_segment_by_addr(PHYS_LOAD_ADDR)
    assert vm_segment is not None, "cannot find physical memory of VM in coredump"
    mem = core.map_segment(vm_segment)
    start_addr, end_addr = find_ksymtab_strings(mem)
    print(f"found ksymtab at 0x{start_addr:x}:0x{end_addr:x}")
    sregs = core.special_regs[0]
    cr3 = sregs.cr3
    page_table_segment = core.find_segment_by_addr(cr3)
    assert page_table_segment is not None
    # for simplicity we assume that the page table is also in this main vm allocation
    assert vm_segment.header == page_table_segment.header
    dump_page_table(core, mem)


def main() -> None:
    if len(sys.argv) < 2:
        print(f"USAGE: {sys.argv[0]} coredump", file=sys.stderr)
        sys.exit(1)
    with open(sys.argv[1], "rb") as f:
        inspect_coredump(f)


if __name__ == "__main__":
    main()
