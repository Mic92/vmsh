use crate::cpu::Regs;
use kvm_bindings as kvmb;
use log::debug;
use nix::sys::mman::ProtFlags;
use simple_error::{bail, require_with, try_with};
use std::cmp::{max, Ordering};
use std::ops::Range;
use std::sync::Arc;

use crate::kvm::hypervisor::memory::PhysMem;
use crate::kvm::hypervisor::Hypervisor;
use crate::page_math::huge_page_size;
use crate::page_table::{
    self, PageTable, PageTableFlags, PageTableIteratorValue, PhysAddr, VirtMem,
};
use crate::result::Result;

pub struct GuestMem {
    maps: Arc<PhysHostMap>,
    regs: Regs,
    pml4: PhysAddr,
}

// x86_64 & linux address to load the Linux kernel too
const PHYS_ADDR_MASK: u64 = 0xFFFFFFFFFF000;

// enable PCID support
const X86_CR4_PCIDE: u64 = 0x00020000;

fn get_page_table_addr(sregs: &kvmb::kvm_sregs) -> usize {
    (if sregs.cr4 & X86_CR4_PCIDE != 0 {
        sregs.cr3 & PHYS_ADDR_MASK
    } else {
        sregs.cr3
    }) as usize
}

/// Contineous physical memory that is mapped virtual contineous
#[derive(Clone, Debug)]
pub struct MappedMemory {
    pub phys_start: PhysAddr,
    pub virt_start: usize,
    pub len: usize,
    pub prot: ProtFlags,
}

impl MappedMemory {
    pub fn contains(&self, addr: usize) -> bool {
        self.virt_start < addr && addr < self.virt_start + self.len
    }
}

fn prot_flags(ptflags: PageTableFlags) -> ProtFlags {
    let mut f = ProtFlags::PROT_READ;
    if ptflags.contains(PageTableFlags::WRITABLE) {
        f |= ProtFlags::PROT_WRITE;
    }
    if !ptflags.contains(PageTableFlags::NO_EXECUTE) {
        f |= ProtFlags::PROT_EXEC;
    }
    f
}

fn mapped_memory(e: &PageTableIteratorValue, host_offset: isize) -> MappedMemory {
    MappedMemory {
        phys_start: PhysAddr {
            value: e.entry.addr() as usize,
            host_offset,
        },
        virt_start: e.virt_addr as usize,
        len: huge_page_size(e.level),
        prot: prot_flags(e.entry.flags()),
    }
}

pub struct PhysHostMap {
    memslots: Vec<(Range<usize>, isize)>,
}

impl PhysHostMap {
    pub fn new<I>(memslots: I) -> PhysHostMap
    where
        I: Iterator<Item = (Range<usize>, isize)>,
    {
        let mut vec: Vec<(Range<usize>, isize)> = Vec::new();

        for (range, val) in memslots.into_iter() {
            if let Some(&mut (ref mut last_range, ref last_val)) = vec.last_mut() {
                if range.start <= last_range.end && &val != last_val {
                    panic!("overlapping ranges ({:x}-{:x}) and ({:x}-{:x}) map to values {:x} and {:x}",
                           last_range.start, last_range.end, range.start, range.end, last_val, val);
                }

                if range.start <= last_range.end.saturating_add(1) && &val == last_val {
                    last_range.end = max(range.end, last_range.end);
                    continue;
                }
            }

            vec.push((range.clone(), val));
        }
        PhysHostMap { memslots: vec }
    }

    pub fn last_range(&self) -> Option<Range<usize>> {
        self.memslots.last().map(|v| v.0.clone())
    }

    pub fn get_range(&self, phys_addr: usize) -> Option<(Range<usize>, isize)> {
        self.memslots
            .binary_search_by(|r| {
                if r.0.end < phys_addr {
                    Ordering::Less
                } else if r.0.start > phys_addr {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
            .ok()
            .map(|idx| self.memslots[idx].clone())
    }

    pub fn get(&self, phys_addr: usize) -> Option<isize> {
        self.get_range(phys_addr).map(|v| v.1)
    }
}

impl GuestMem {
    pub fn new(hv: &Hypervisor) -> Result<GuestMem> {
        // We only get maps once. This information could get all if the
        // hypervisor dynamically allocates physical memory. However this is
        // problematic anyway since it could override allocations made by us.
        // To make the design sound we try to allocate memory near the 4 Peta
        // byte limit in the hope that VMs are not getting close to this limit
        // any time soon.
        let mut mappings = try_with!(hv.get_maps(), "cannot vm memory allocations");
        mappings.sort_by_key(|m| m.phys_addr);

        let maps =
            Arc::new(PhysHostMap::new(mappings.iter().map(|m| {
                (m.phys_addr..m.phys_end() - 1, m.phys_to_host_offset())
            })));
        let first_core = &hv.vcpus[0];
        let regs = try_with!(hv.get_regs(first_core), "failed to get vcpu registers");
        let sregs = try_with!(
            hv.get_sregs(first_core),
            "failed to get vcpu special registers"
        );

        let pt_addr = get_page_table_addr(&sregs);

        debug!("pml4: {:#x}\n", pt_addr);

        let host_offset = require_with!(maps.get(pt_addr), "cannot find page table memory");

        Ok(GuestMem {
            maps,
            regs,
            pml4: PhysAddr {
                value: pt_addr,
                host_offset,
            },
        })
    }

    pub fn last_memslot_range(&self) -> Option<Range<usize>> {
        self.maps.last_range()
    }

    pub fn map_memory(
        &mut self,
        hv: Arc<Hypervisor>,
        phys_mem: PhysMem<u8>,
        map: &[MappedMemory],
    ) -> Result<VirtMem> {
        page_table::map_memory(hv, phys_mem, &mut self.pml4, map, &self.maps)
    }

    pub fn find_kernel_sections(
        &self,
        hv: &Hypervisor,
        range: Range<usize>,
    ) -> Result<Vec<MappedMemory>> {
        let cpl = self.regs.cs & 3;
        if cpl == 3 {
            bail!("program stopped in userspace. Linux kernel might be not mapped in thise mode");
        }

        // level/virt_addr is wrong, but does not matter
        let pml4 = try_with!(
            PageTable::read(hv, &self.pml4, 0, 0),
            "cannot read pml4 page table"
        );

        let mut iter = pml4.iter(hv, Arc::clone(&self.maps), range.clone());
        let mut sections: Vec<_> = vec![];
        for e in &mut iter {
            let entry = try_with!(e, "cannot read page table");
            if entry.virt_addr as usize > range.start {
                //info!("{:#x}/{:#x}: {:?}", entry.entry.addr(), entry.virt_addr, &entry.entry.flags());
                let addr = entry.entry.addr();
                let host_offset = require_with!(
                    self.maps.get(addr as usize),
                    "no memslot of physical address {} of page table"
                );
                sections.push(mapped_memory(&entry, host_offset));
                break;
            }
        }
        if sections.is_empty() {
            bail!("no linux kernel found in page table");
        }
        for e in &mut iter {
            let entry = try_with!(e, "cannot read page table");
            //info!("{:#x}/{:#x}: {:?}", entry.entry.addr(), entry.virt_addr, &entry.entry.flags());
            if entry.virt_addr as usize >= range.end {
                break;
            }
            let last = sections.last_mut().unwrap();
            if last.prot == prot_flags(entry.entry.flags()) {
                last.len += huge_page_size(entry.level);
            } else {
                let addr = entry.entry.addr();
                let host_offset = require_with!(
                    self.maps.get(addr as usize),
                    "no memslot of physical address {} of page table"
                );
                sections.push(mapped_memory(&entry, host_offset));
            }
        }
        Ok(sections)
    }
}

#[cfg(test)]
mod tests {
    use crate::guest_mem::PhysHostMap;

    #[test]
    fn range_lookup() {
        let m = PhysHostMap::new(vec![(1..9, 1), (10..15, 2)].into_iter());
        assert_eq!(m.get(1), Some(1));
        assert_eq!(m.get(2), Some(1));
        assert_eq!(m.get(11), Some(2));
        assert_eq!(m.get(16), None);
    }
}
