use crate::cpu::Regs;
use kvm_bindings as kvmb;
use log::debug;
use nix::sys::mman::ProtFlags;
use simple_error::{bail, require_with, try_with};
use std::ops::Range;
use std::sync::Arc;

use crate::kvm::hypervisor::memory::PhysMem;
use crate::kvm::hypervisor::Hypervisor;
use crate::page_math::huge_page_size;
use crate::page_table::{
    self, PageTable, PageTableFlags, PageTableIteratorValue, PhysAddr, VirtMem,
};
use crate::result::Result;
use crate::tracer::proc::Mapping;

pub struct GuestMem {
    maps: Vec<Mapping>,
    regs: Regs,
    sregs: kvmb::kvm_sregs,
    page_table_mapping_idx: usize,
    pml4: PhysAddr,
}

// x86_64 & linux address to load the Linux kernel too
const KERNEL_PHYS_LOAD_ADDR: usize = 0x100000;
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

impl GuestMem {
    pub fn new(hv: &Hypervisor) -> Result<GuestMem> {
        // We only get maps once. This information could get all if the
        // hypervisor dynamically allocates physical memory. However this is
        // problematic anyway since it could override allocations made by us.
        // To make the design sound we try to allocate memory near the 4 Peta
        // byte limit in the hope that VMs are not getting close to this limit
        // any time soon.
        let maps = try_with!(hv.get_maps(), "cannot vm memory allocations");
        let first_core = &hv.vcpus[0];
        let regs = try_with!(hv.get_regs(first_core), "failed to get vcpu registers");
        let sregs = try_with!(
            hv.get_sregs(first_core),
            "failed to get vcpu special registers"
        );

        let pt_addr = get_page_table_addr(&sregs);

        debug!("pml4: {:#x}\n", pt_addr);

        let (idx, pt_mapping) = require_with!(
            maps.iter()
                .enumerate()
                .find(|(_, m)| { m.phys_addr <= pt_addr && pt_addr < m.phys_end() }),
            "cannot find page table memory"
        );

        let host_offset = pt_mapping.phys_to_host_offset();

        Ok(GuestMem {
            maps,
            regs,
            sregs,
            page_table_mapping_idx: idx,
            pml4: PhysAddr {
                value: pt_addr,
                host_offset,
            },
        })
    }

    pub fn last_mapping(&self) -> Option<&Mapping> {
        self.maps.iter().max_by_key(|m| m.phys_addr + m.size())
    }

    fn kernel_mapping(&self) -> Option<&Mapping> {
        self.maps
            .iter()
            .find(|m| m.phys_addr == KERNEL_PHYS_LOAD_ADDR)
    }

    fn page_table_mapping(&self) -> &Mapping {
        &self.maps[self.page_table_mapping_idx]
    }

    pub fn map_memory(
        &mut self,
        hv: Arc<Hypervisor>,
        phys_mem: PhysMem<u8>,
        map: &[MappedMemory],
    ) -> Result<VirtMem> {
        page_table::map_memory(hv, phys_mem, &mut self.pml4, map)
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

        let kernel_mapping = require_with!(self.kernel_mapping(), "cannot find kernel memory");
        let pt_mapping = self.page_table_mapping();
        let pt_addr = get_page_table_addr(&self.sregs);
        if kernel_mapping != pt_mapping {
            bail!("kernel memory and page table ({:#x}) is not in the same physical memory block: {:#x}-{:#x} vs {:#x}-{:#x}", pt_addr, kernel_mapping.phys_addr, kernel_mapping.phys_end(), pt_mapping.phys_addr, pt_mapping.phys_end());
        }

        // level/virt_addr is wrong, but does not matter
        let pml4 = try_with!(
            PageTable::read(hv, &self.pml4, 0, 0),
            "cannot read pml4 page table"
        );

        let mut iter = pml4.iter(hv, range.clone());
        let mut sections: Vec<_> = vec![];
        let host_offset = pt_mapping.phys_to_host_offset();
        for e in &mut iter {
            let entry = try_with!(e, "cannot read page table");
            if entry.virt_addr as usize > range.start {
                //info!("{:#x}/{:#x}: {:?}", entry.entry.addr(), entry.virt_addr, &entry.entry.flags());
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
                sections.push(mapped_memory(&entry, host_offset));
            }
        }
        Ok(sections)
    }
}
