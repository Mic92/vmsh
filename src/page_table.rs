use std::cmp::max;

use bitflags::bitflags;

use crate::kvm::hypervisor::{process_read, Hypervisor};
use crate::page_math::add_offset;
use crate::result::Result;

const ENTRY_COUNT: usize = 512;

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

#[derive(Clone, Copy, Debug)]
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
    level: u8,
}

pub struct PageTableIterator<'a> {
    hv: &'a Hypervisor,
    page_table: PageTable,
    phys_to_host_offset: isize,
    start: usize,
    end: usize,
    count: usize,
    inner: Option<Box<Self>>,
}

impl PageTable {
    pub fn new(hv: &Hypervisor, host_addr: u64, virt_addr: u64, level: u8) -> Result<Self> {
        let pid = hv.pid;
        let entries = process_read(pid, host_addr as *const libc::c_void)?;

        Ok(PageTable {
            entries,
            virt_addr,
            level,
        })
    }
    pub fn iter(
        self,
        hv: &Hypervisor,
        phys_to_host_offset: isize,
        start: usize,
        end: usize,
    ) -> PageTableIterator {
        PageTableIterator {
            hv,
            phys_to_host_offset,
            start,
            end,
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

impl<'a> Iterator for PageTableIterator<'a> {
    type Item = Result<(u64, PageTableEntry)>;
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
        let start = get_index(self.start as u64, pt.level) as usize;
        self.count = max(self.count, start);
        let end = get_index(self.end as u64, pt.level) as usize + 1;
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
                return Some(Ok((virt_addr, *entry)));
            }
            let host_addr = add_offset(entry.addr() as usize, self.phys_to_host_offset) as u64;
            let next_pt = PageTable::new(self.hv, host_addr, virt_addr, pt.level + 1).ok()?;

            let start = if idx as usize > start { 0 } else { self.start };
            let end = if (idx as usize) < end - 1 {
                usize::MAX
            } else {
                self.end
            };

            let mut inner = next_pt.iter(self.hv, self.phys_to_host_offset, start, end);
            if let Some(next) = &inner.next() {
                self.inner = Some(Box::new(inner));
                return Some(next.clone());
            }
        }
        None
    }
}
