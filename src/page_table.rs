use std::cmp::max;
use std::mem::{size_of, size_of_val};
use std::ops::Range;
use std::sync::Arc;

use crate::guest_mem::MappedMemory;
use crate::kvm::hypervisor::{process_read, Hypervisor, PhysMem};
use crate::page_math::{is_page_aligned, page_align, page_size};
use crate::result::Result;
use bitflags::bitflags;
use log::error;
use nix::sys::mman::ProtFlags;
use nix::sys::uio::{process_vm_writev, IoVec, RemoteIoVec};
use simple_error::{bail, try_with};
use vm_memory::remote_mem::any_as_bytes;

const ENTRY_COUNT: usize = 512;
const LEVEL_COUNT: usize = 4;

bitflags! {
    /// Possible flags for a page table entry.
    pub struct PageTableFlags: u64 {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT =         1;
        /// Controls whether writes to the mapped frames are allowed.
        ///
        /// If this bit is unset in a level 1 page table entry, the mapped frame is read-only.
        /// If this bit is unset in a higher level page table entry the complete range of mapped
        /// pages is read-only.
        const WRITABLE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
        const USER_ACCESSIBLE = 1 << 2;
        /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
        /// policy is used.
        const WRITE_THROUGH =   1 << 3;
        /// Disables caching for the pointed entry is cacheable.
        const NO_CACHE =        1 << 4;
        /// Set by the CPU when the mapped frame or page table is accessed.
        const ACCESSED =        1 << 5;
        /// Set by the CPU on a write to the mapped frame.
        const DIRTY =           1 << 6;
        /// Specifies that the entry maps a huge frame instead of a page table. Only allowed in
        /// P2 or P3 tables.
        const HUGE_PAGE =       1 << 7;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_9 =           1 << 9;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_10 =          1 << 10;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_11 =          1 << 11;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_52 =          1 << 52;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_53 =          1 << 53;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_54 =          1 << 54;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_55 =          1 << 55;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_56 =          1 << 56;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_57 =          1 << 57;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_58 =          1 << 58;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_59 =          1 << 59;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_60 =          1 << 60;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_61 =          1 << 61;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_62 =          1 << 62;
        /// Forbid code execution from the mapped frames.
        ///
        /// Can be only used when the no-execute page protection feature is enabled in the EFER
        /// register.
        const NO_EXECUTE =      1 << 63;
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(transparent)]
pub struct PageTableEntry {
    entry: u64,
}

impl PageTableEntry {
    /// Returns whether this entry is zero.
    #[inline]
    pub const fn is_unused(&self) -> bool {
        self.entry == 0
    }

    /// Map the entry to the specified physical address with the specified flags.
    pub fn set_addr(&mut self, addr: &PhysAddr, flags: PageTableFlags) {
        assert!(addr.is_page_aligned());
        self.entry = addr.value as u64 | flags.bits();
    }

    /// Sets this entry to zero.
    #[inline]
    pub fn set_unused(&mut self) {
        self.entry = 0;
    }

    /// Returns the flags of this entry.
    #[inline]
    pub const fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.entry)
    }

    /// Returns the physical address mapped by this entry, might be zero.
    #[inline]
    pub fn addr(&self) -> u64 {
        self.entry & 0x000f_ffff_ffff_f000
    }
}

#[repr(C)]
#[derive(Clone, Debug)]
pub struct PageTable {
    entries: [PageTableEntry; ENTRY_COUNT],
    virt_addr: u64,
    phys_addr: PhysAddr,
    level: u8,
}

pub struct PageTableIterator<'a> {
    hv: &'a Hypervisor,
    page_table: PageTable,
    range: Range<usize>,
    count: usize,
    inner: Option<Box<Self>>,
}

impl PageTable {
    pub fn empty(phys_addr: PhysAddr) -> Self {
        PageTable {
            entries: [PageTableEntry::default(); ENTRY_COUNT],
            // we don't care about the virtual address/level here
            virt_addr: 0,
            level: 0,
            phys_addr,
        }
    }

    pub fn read(hv: &Hypervisor, phys_addr: &PhysAddr, virt_addr: u64, level: u8) -> Result<Self> {
        let host_addr = phys_addr.host_addr();
        let entries = process_read(hv.pid, host_addr as *const libc::c_void)?;

        Ok(PageTable {
            phys_addr: phys_addr.clone(),
            entries,
            virt_addr,
            level,
        })
    }

    pub fn phys_addr(&self, e: PageTableEntry) -> PhysAddr {
        PhysAddr {
            value: e.addr() as usize,
            host_offset: self.phys_addr.host_offset,
        }
    }

    pub fn iter(self, hv: &Hypervisor, range: Range<usize>) -> PageTableIterator {
        PageTableIterator {
            hv,
            range,
            page_table: self,
            count: 0,
            inner: None,
        }
    }
}

fn get_shift(level: u8) -> u8 {
    assert!(level <= 3);
    12 + 9 * (3 - level)
}

fn get_index(virt: u64, level: u8) -> u64 {
    virt >> get_shift(level) & 0x1FF
}

pub fn table_align(pages: usize) -> usize {
    (pages + (ENTRY_COUNT - 1)) & !(ENTRY_COUNT - 1)
}

/// Upper bound of page tables memory we need to map physical memory of given size
pub fn estimate_page_table_size(size: usize) -> usize {
    let pages = page_align(size as usize) / page_size();
    let mut tables = pages;
    let mut total_tables = 0;
    for _ in 0..LEVEL_COUNT {
        tables = table_align(tables) / ENTRY_COUNT;
        total_tables += tables;
    }
    total_tables * size_of::<u64>() * ENTRY_COUNT
}

pub struct VirtMem {
    hv: Arc<Hypervisor>,
    /// List of tables we need to restore to their old state before exiting
    old_tables: Vec<PageTable>,
    /// physical memory used to hold page tables and bake virtual memory
    #[allow(unused)]
    phys_mem: PhysMem<u8>,
    /// Mapping between virtual and physical memory
    pub mappings: Vec<MappedMemory>,
}

impl Drop for VirtMem {
    fn drop(&mut self) {
        if let Err(e) = commit_page_tables(&self.hv, &self.old_tables) {
            error!("cannot restore old page tables: {}", e);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysAddr {
    /// The actual physical address
    pub value: usize,
    /// Offset to virtual host memory in the hypervisor
    pub host_offset: isize,
}

impl PhysAddr {
    pub fn host_addr(&self) -> usize {
        if self.host_offset < 0 {
            self.value - self.host_offset.wrapping_abs() as usize
        } else {
            self.value + self.host_offset as usize
        }
    }
    pub fn is_page_aligned(&self) -> bool {
        is_page_aligned(self.value)
    }
    pub fn add(&self, offset: usize) -> PhysAddr {
        let mut a = self.clone();
        a.value += offset;
        a
    }
}

fn allocate_page_table(entry: &mut PageTableEntry, phys_addr: &mut PhysAddr) -> PageTable {
    let table = PageTable::empty(phys_addr.clone());
    // we just use the same flags the linux kernel expects for page tables
    entry.set_addr(
        &table.phys_addr,
        PageTableFlags::PRESENT
            | PageTableFlags::ACCESSED
            | PageTableFlags::DIRTY
            | PageTableFlags::WRITABLE,
    );
    phys_addr.value += size_of_val(&table.entries);
    table
}

fn read_page_table(
    hv: &Hypervisor,
    entry: &mut PageTableEntry,
    host_offset: isize,
    old_tables: &mut Vec<PageTable>,
) -> Result<PageTable> {
    let addr = PhysAddr {
        value: entry.addr() as usize,
        host_offset,
    };
    // level/virt_addr is wrong, but does not matter
    let pt = try_with!(
        PageTable::read(hv, &addr, 0, 0),
        "cannot read page table at 0x{:x} (host_addr: 0x{:x})",
        addr.value,
        addr.host_addr()
    );
    old_tables.push(pt.clone());
    Ok(pt)
}

fn get_page_table(
    hv: &Hypervisor,
    pt_host_offset: isize,
    entry: &mut PageTableEntry,
    phys_addr: &mut PhysAddr,
    old_tables: &mut Vec<PageTable>,
) -> Result<PageTable> {
    // should be empty
    if entry
        .flags()
        .contains(PageTableFlags::HUGE_PAGE | PageTableFlags::PRESENT)
    {
        bail!("found huge table in page table");
    }

    if entry.flags().contains(PageTableFlags::PRESENT) {
        read_page_table(hv, entry, pt_host_offset, old_tables)
    } else {
        Ok(allocate_page_table(entry, phys_addr))
    }
}

fn get_start_idx(virt_addr: usize, start: usize, idx: usize, level: u8) -> usize {
    if idx > start {
        0
    } else {
        get_index(virt_addr as u64, level) as usize
    }
}

fn commit_page_tables(hv: &Hypervisor, tables: &[PageTable]) -> Result<()> {
    let local_iovec = tables
        .iter()
        .map(|table| {
            let bytes = unsafe { any_as_bytes(&table.entries) };
            IoVec::from_slice(bytes)
        })
        .collect::<Vec<_>>();
    let remote_iovec = tables
        .iter()
        .map(|table| RemoteIoVec {
            base: table.phys_addr.host_addr(),
            len: page_size(),
        })
        .collect::<Vec<_>>();

    let written = try_with!(
        process_vm_writev(hv.pid, local_iovec.as_slice(), remote_iovec.as_slice()),
        "cannot write to process"
    );
    let expected = remote_iovec.len() * page_size();
    if written != expected {
        bail!("short write, expected {}, written: {}", expected, written);
    }
    Ok(())
}

fn page_table_flags(p: ProtFlags) -> PageTableFlags {
    // we need both present/accessed for a valid page table entry
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::ACCESSED;
    if p.contains(ProtFlags::PROT_WRITE) {
        flags |= PageTableFlags::WRITABLE;
    }
    if !p.contains(ProtFlags::PROT_EXEC) {
        flags |= PageTableFlags::NO_EXECUTE;
    }
    flags
}

fn map_memory_single(
    hv: &Hypervisor,
    pml4: &mut PageTable,
    m: &MappedMemory,
    upsert_tables: &mut Vec<PageTable>,
    old_tables: &mut Vec<PageTable>,
    pt_addr: &mut PhysAddr,
) -> Result<()> {
    let mut phys_addr = m.phys_start.clone();
    let mut len = m.len;
    let pt_host_offset = pml4.phys_addr.host_offset;
    let start0 = get_index(m.virt_start as u64, 0) as usize;
    for (i0, entry0) in pml4.entries[start0..].iter_mut().enumerate() {
        let start1 = get_start_idx(m.virt_start, start0, i0, 1);
        let mut pt1 = get_page_table(hv, pt_host_offset, entry0, pt_addr, old_tables)?;

        for (i1, entry1) in pt1.entries[start1..].iter_mut().enumerate() {
            let start2 = get_start_idx(m.virt_start, start1, i1, 2);
            let mut pt2 = get_page_table(hv, pt_host_offset, entry1, pt_addr, old_tables)?;

            for (i2, entry2) in pt2.entries[start2..].iter_mut().enumerate() {
                let start3 = get_start_idx(m.virt_start, start2, i2, 3);
                let mut pt3 = get_page_table(hv, pt_host_offset, entry2, pt_addr, old_tables)?;

                for entry3 in pt3.entries[start3..].iter_mut() {
                    if entry3.flags().contains(PageTableFlags::PRESENT) {
                        bail!("found already mapped page in page table");
                    }
                    entry3.set_addr(&phys_addr, page_table_flags(m.prot));
                    phys_addr.value += page_size();
                    len -= page_size();
                    if len == 0 {
                        break;
                    }
                }
                upsert_tables.push(pt3.clone());
                if len == 0 {
                    break;
                }
            }
            upsert_tables.push(pt2.clone());
            if len == 0 {
                break;
            }
        }
        upsert_tables.push(pt1.clone());
        if len == 0 {
            break;
        }
    }
    upsert_tables.push(pml4.clone());
    Ok(())
}

/// Maps a list of physical memory chunks at phys_addr with a length of u64 to virt_addr.
/// The list must to be physical continous and sorted.
/// To allocate page tables it uses space at the end of given physical memory address.
/// There must be enough space after the last mapping to store these pagetable.
pub fn map_memory(
    hv: Arc<Hypervisor>,
    phys_mem: PhysMem<u8>,
    pml4: &mut PageTable,
    mappings: &[MappedMemory],
) -> Result<VirtMem> {
    // New/modified tables to be written to guest
    let mut upsert_tables: Vec<PageTable> = vec![];
    // Tables that we need to revert to their old content
    let mut old_tables: Vec<PageTable> = vec![pml4.clone()];

    assert!(pml4.level == 0);
    for (i, m) in mappings.iter().enumerate() {
        assert!(is_page_aligned(m.len as usize));
        assert!(is_page_aligned(m.virt_start as usize));
        assert!(m.phys_start.is_page_aligned());
        if i > 0 {
            assert!(mappings[i - 1].phys_start.add(mappings[i - 1].len) == m.phys_start)
        }
    }
    let last_mapping = &mappings[mappings.len() - 1];
    let mut pt_addr = last_mapping.phys_start.add(last_mapping.len);

    for mapping in mappings {
        map_memory_single(
            &hv,
            pml4,
            mapping,
            &mut upsert_tables,
            &mut old_tables,
            &mut pt_addr,
        )?
    }

    // TODO: invalidate TLB cache
    try_with!(
        commit_page_tables(&hv, &upsert_tables),
        "cannot write page tables"
    );

    Ok(VirtMem {
        hv,
        old_tables,
        phys_mem,
        mappings: mappings.to_vec(),
    })
}

#[derive(Copy, Clone)]
pub struct PageTableIteratorValue {
    pub virt_addr: u64,
    pub level: u8,
    pub entry: PageTableEntry,
}

impl<'a> Iterator for PageTableIterator<'a> {
    type Item = Result<PageTableIteratorValue>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(inner) = &mut self.inner {
            let item = inner.next();
            if item.is_some() {
                return item;
            } else {
                self.inner.take();
            }
        }
        let pt = &self.page_table;
        let start = get_index(self.range.start as u64, pt.level) as usize;
        self.count = max(self.count, start);
        let end = get_index(self.range.end as u64, pt.level) as usize + 1;
        for entry in pt.entries[self.count..end].iter() {
            let idx = self.count as u64;
            self.count += 1;
            let mut virt_addr = pt.virt_addr + (idx << get_shift(pt.level));
            // sign extend most significant bit
            if virt_addr >> 47 != 0 {
                virt_addr |= 0xFFFF << 48
            }
            if !entry.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }

            if pt.level == 3 || entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                return Some(Ok(PageTableIteratorValue {
                    virt_addr,
                    level: pt.level,
                    entry: *entry,
                }));
            }
            let next_pt =
                PageTable::read(self.hv, &pt.phys_addr(*entry), virt_addr, pt.level + 1).ok()?;

            let start = if idx as usize > start {
                0
            } else {
                self.range.start
            };
            let end = if (idx as usize) < end - 1 {
                usize::MAX
            } else {
                self.range.end
            };

            let mut inner = next_pt.iter(self.hv, start..end);
            if let Some(next) = &inner.next() {
                self.inner = Some(Box::new(inner));
                return Some(next.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::page_math::page_size;

    use super::{estimate_page_table_size, ENTRY_COUNT, LEVEL_COUNT};
    #[test]
    fn test_page_table_size() {
        assert_eq!(estimate_page_table_size(1), page_size() * LEVEL_COUNT);
        assert_eq!(estimate_page_table_size(4096), page_size() * LEVEL_COUNT);
        assert_eq!(
            estimate_page_table_size(4096 * ENTRY_COUNT + 1),
            page_size() + page_size() * LEVEL_COUNT
        );
    }
}
