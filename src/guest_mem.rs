use crate::page_math::add_offset;
use crate::page_table::PageTable;
use crate::tracer::proc::Mapping;
use crate::{cpu::Regs, kvm::hypervisor::Hypervisor};
use kvm_bindings as kvmb;
use log::info;
use simple_error::{bail, require_with, try_with};

use crate::result::Result;

pub struct GuestMem {
    maps: Vec<Mapping>,
    regs: Regs,
    sregs: kvmb::kvm_sregs,
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

const LINUX_KERNEL_KASLR_RANGE_START: usize = 0xFFFFFFFF80000000;
const LINUX_KERNEL_KASLR_RANGE_END: usize = 0xFFFFFFFFC0000000;

/// Contineous physical memory that is mapped virtual contineous
#[derive(Debug)]
pub struct MappedMemory {
    pub phys_start: usize,
    pub virt_start: usize,
    pub len: usize,
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

        Ok(GuestMem { maps, regs, sregs })
    }

    pub fn last_mapping(&self) -> Option<&Mapping> {
        self.maps.iter().max_by_key(|m| m.phys_addr + m.size())
    }

    fn kernel_mapping(&self) -> Option<&Mapping> {
        self.maps
            .iter()
            .find(|m| m.phys_addr == KERNEL_PHYS_LOAD_ADDR)
    }

    pub fn find_kernel(&self, hv: &Hypervisor) -> Result<MappedMemory> {
        let cpl = self.regs.cs & 3;
        if cpl == 3 {
            bail!("program stopped in userspace. Linux kernel might be not mapped in thise mode");
        }

        let kernel_mapping = require_with!(self.kernel_mapping(), "cannot find kernel memory");
        let pt_addr = get_page_table_addr(&self.sregs);
        let pt_mapping = require_with!(
            self.maps
                .iter()
                .find(|m| { m.phys_addr <= pt_addr && pt_addr < m.phys_end() }),
            "cannot find page table memory"
        );
        if kernel_mapping != pt_mapping {
            bail!("kernel memory and page table (0x{:x}) is not in the same physical memory block: 0x{:x}-0x{:x} vs 0x{:x}-0x{:x}", pt_addr, kernel_mapping.phys_addr, kernel_mapping.phys_end(), pt_mapping.phys_addr, pt_mapping.phys_end());
        }
        let host_offset = pt_mapping.phys_to_host_offset();
        let host_addr = add_offset(pt_addr, host_offset);

        info!("pml4: 0x{:x} -> 0x{:x}\n", pt_addr, host_addr);

        let pt = try_with!(
            PageTable::new(hv, host_addr as u64, 0, 0),
            "cannot load page table"
        );
        let mut iter = pt.iter(
            hv,
            host_offset,
            LINUX_KERNEL_KASLR_RANGE_START,
            LINUX_KERNEL_KASLR_RANGE_END,
        );
        let mut first: Option<_> = None;
        for e in &mut iter {
            let (virt_addr, entry) = try_with!(e, "cannot read page table");
            if virt_addr as usize > LINUX_KERNEL_KASLR_RANGE_START {
                first = Some((virt_addr, entry));
                break;
            }
        }
        let first = require_with!(first, "no linux kernel found in page table");
        let mut last = first;
        for e in &mut iter {
            let (virt_addr, entry) = try_with!(e, "cannot read page table");
            if virt_addr as usize > LINUX_KERNEL_KASLR_RANGE_END {
                break;
            }
            last = (virt_addr, entry);
        }
        Ok(MappedMemory {
            phys_start: first.1.addr() as usize,
            virt_start: first.0 as usize,
            len: (last.0 - first.0) as usize,
        })
    }
}
