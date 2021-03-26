#!/usr/bin/env python3

import sys
import os

sys.path.append(os.path.join(os.path.dirname(__file__), "tests"))

from elftools.elf.segments import Segment
import sys
from typing import Optional, Tuple, IO
from enum import IntEnum
import ctypes as ct

from coredump import ElfCore
from cpu_flags import (
    X86_CR3_PWT,
    X86_CR3_PCD,
    X86_CR0_PG,
    X86_CR4_PAE,
    X86_CR4_PCIDE,
    X86_CR4_PKE,
    EFER_NX,
    EFER_LME,
    _PAGE_PRESENT,
    _PAGE_PSE,
    _PAGE_RW,
    _PAGE_USER,
    _PAGE_PWT,
    _PAGE_PCD,
    _PAGE_ACCESSED,
    _PAGE_NX,
)


def find_vm_segment(core: ElfCore) -> Optional[Segment]:
    for seg in core.elf.iter_segments():
        if seg.header["p_type"] != "PT_LOAD":
            continue
        # x86_64-specific
        if seg.header.p_vaddr == 0x100000:
            return seg
    return None


def find_page_table(core: ElfCore, cr3: int) -> Optional[Segment]:
    for seg in core.elf.iter_segments():
        if seg.header["p_type"] != "PT_LOAD":
            continue
        start = seg.header.p_paddr
        # x86_64-specific
        if cr3 >= start and cr3 < (start + seg.header.p_memsz):
            return seg
    return None


def is_printable(byte: int) -> bool:
    return 0x20 < byte < 0x7E


def find_ksymtab_strings(data: bytes) -> Tuple[int, int]:
    try:
        idx = data.index(b"init_task")
    except ValueError:
        raise RuntimeError("could not find ksymtab_strings")

    unprintable = 0
    for start_offset, byte in enumerate(reversed(data[0:idx])):
        if is_printable(byte):
            unprintable = 0
        else:
            unprintable += 1
        if unprintable == 2:
            break
    start = idx - start_offset + 1

    for end_offset, byte in enumerate(data[idx:-1]):
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
x86_phys_bits = 36
pte_reserved_flags = bitmask_numbits(
    51 - x86_phys_bits + 1
)  # bitmap with up to 51 bit set


def is_page_aligned(num: int) -> bool:
    return num & (4096 - 1) == num


# A 4k intel page table with 512 64bit entries.
PAGE_TABLE_SIZE = 512
PageTableEntries = ct.c_uint64 * PAGE_TABLE_SIZE


class PageTable:
    def __init__(self, mem: bytes, baddr: int) -> None:
        breakpoint()
        self.entries = PageTableEntries.from_buffer_copy(mem)
        # current index into the pagetable
        self._i = 0
        self.baddr = baddr

    @property
    def i(self) -> int:
        """
        virtual memory base address mapped by current pml4e
        """
        return self._i

    @i.setter
    def i(self, val: int) -> None:
        assert val > 0 and val < 512
        self._i = val


class State:
    def __init__(self, mem: bytes, mem_offset: int, cr3: int) -> None:
        self.mem = mem
        self.mem_offset = mem_offset
        self.pml4 = self.page_table(cr3)
        self.pdpt: Optional[PageTable] = None
        self.pd: Optional[PageTable] = None
        self.pt: Optional[PageTable] = None

        # compress output, don't print same entries all over
        self.last_addr: int = 0
        self.last_flags: int = 0
        self.skipped: int = 0

    def page_table(self, offset: int) -> PageTable:
        start = offset - self.mem_offset
        assert start > 0
        end = start + ct.sizeof(PageTableEntries)
        breakpoint()
        return PageTable(self.mem[start:end], start)

    def next_page_table(self, pagetbl_entry: int) -> PageTable:
        # pagetble_entry bits 51:12 contains the physical address of the next page
        # table level
        bm = bitmask_numbits(51 - 12 + 1) << 12
        # physical addr of page table entry
        paddr = pagetbl_entry & bm
        # Actually, 51:.. is too generous, there are some reserved bits which must be
        # zero
        assert (pagetbl_entry & pte_reserved_flags) == 0

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
    if e & pte_reserved_flags:
        print("invalid entry!\n")
        return False
    if e & _PAGE_PSE:
        # TODO references a page directly, probably sanity check address?
        pass
    if not (e & _PAGE_PRESENT) and e:
        print("strange entry!\n")
        return False

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
    assert (
        state.pdpt is not None
        and (state.pml4.baddr | state.pdpt.baddr) & state.pml4.baddr == state.pml4.baddr
    )  # no overlapping bits
    addr_max = bitmask_numbits(bitpos)
    assert (state.pml4.baddr & addr_max) == 0  # no overlapping bits
    assert (state.pdpt.baddr & addr_max) == 0  # no overlapping bits
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
                "{str_level} skipped {state.skipped} following entries (til {state.last_addr})"
            )
        state.skipped = 0
        rw = "W" if (e & _PAGE_RW) else "R"
        user = "U" if (e & _PAGE_USER) else "K"
        pwt = "PWT" if (e & _PAGE_PWT) else ""
        pcd = "PCD" if (e & _PAGE_PCD) else ""
        accessed = "A" if (e & _PAGE_ACCESSED) else ""
        nx = "NX" if (e & _PAGE_NX) else ""
        print(
            f"{str_level} {baddr} {addr_max} {rw} {user} {pwt} {pcd} {accessed} {nx}",
            end="",
        )
        if _direct_mapping:
            print(" -> {string_page_size(bitpos)} page", end="")
        print()

    state.last_addr = addr_max
    state.last_flags = entry_extract_flags(e)

    return ret


def dump_page_table(core: ElfCore, memory: bytes, memory_offset: int) -> None:
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

    state = State(memory, memory_offset, cr3)
    breakpoint()
    print(f"page table in virtual memory at 0x{state.pml4.baddr:x}")
    if is_page_aligned(cr3):  # TODO ?
        raise RuntimeError("invalid addr!")

    #  4 nested for loops to walk 4 levels of pages walk the outermost page table
    for i in range(0, PAGE_TABLE_SIZE):
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


def inspect_coredump(fd: IO[bytes]) -> None:
    core = ElfCore(fd)
    vm_segment = find_vm_segment(core)
    assert vm_segment is not None, "cannot find physical memory of VM in coredump"
    data = core.map_segment(vm_segment)
    start_off, end_off = find_ksymtab_strings(data)
    start_addr = start_off + vm_segment.header.p_paddr
    end_addr = end_off + vm_segment.header.p_paddr
    print(f"found ksymtab at {start_addr:x}:{end_addr}")
    sregs = core.special_regs[0]
    cr3 = sregs.cr3
    page_table_segment = find_page_table(core, cr3)
    # for simplicity we assume that the page table is also in this main vm allocation
    assert (
        page_table_segment is not None
        and vm_segment.header == page_table_segment.header
    )
    dump_page_table(core, data, vm_segment.header.p_paddr)


def main() -> None:
    if len(sys.argv) < 2:
        print(f"USAGE: {sys.argv[0]} coredump", file=sys.stderr)
        sys.exit(1)
    with open(sys.argv[1], "rb") as f:
        inspect_coredump(f)


if __name__ == "__main__":
    main()