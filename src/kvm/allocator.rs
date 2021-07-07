use std::sync::Arc;

use log::debug;
use simple_error::{bail, require_with, try_with};
use vm_device::bus::{MmioAddress, MmioRange};

use crate::result::Result;
use crate::tracer::proc::Mapping;

use super::hypervisor::{Hypervisor, VmMem};

pub struct PhysMemAllocator {
    hv: Arc<Hypervisor>,
    /// the last memory allocation of the VM
    last_mapping: Mapping,
    /// Physical address where we last allocated memory from.
    /// After an allocating we substract the allocation size from this value.
    next_allocation: usize,
}

impl PhysMemAllocator {
    pub fn new(hv: Arc<Hypervisor>) -> Result<Self> {
        // We only get maps ones. This information could get all if the
        // hypervisor dynamically allocates physical memory. However this is
        // problematic anyway since it could override allocations made by us.
        // To make the design sound we try to allocate memory near the 4 Peta
        // byte limit in the hope that VMs are not getting close to this limit
        // any time soon.
        let maps = try_with!(hv.get_maps(), "cannot vm memory allocations");
        let cpuid2 = try_with!(hv.get_cpuid2(&hv.vcpus[0]), "cannot get cpuid2");
        // Get Virtual and Physical address Sizes
        let entry = cpuid2.entries.iter().find(|c| c.function == 0x80000008);
        let entry = require_with!(
            entry,
            "could not get the vm's cpuid entry for virtual and physical address size"
        );
        // Supported physical address size in bits
        let phys_bits = entry.eax as u8;
        // Supported virtual address size in bits
        let virt_bits = (entry.eax >> 8) as u8;
        debug!("vm/cpu: phys_bits: {}, virt_bits: {}", phys_bits, virt_bits);

        let last_mapping = require_with!(
            maps.iter().max_by_key(|m| m.phys_addr + m.size()),
            "vm has no memory assigned"
        )
        .clone();
        Ok(Self {
            hv,
            last_mapping,
            next_allocation: 1 << phys_bits,
        })
    }

    fn reserve_range(&mut self, size: usize) -> Result<usize> {
        let start = require_with!(self.next_allocation.checked_sub(size), "out of memory");
        let last_alloc = self.last_mapping.phys_addr + self.last_mapping.size();
        if start < last_alloc {
            bail!(
                "cannot allocate memory, our allocator conflicts with {:?}.\
                   This might happen if the last vmsh run did not clean up memory\
                   correctly or the hypervisor has allocated memory at the very end of\
                   the physical address space",
                self.last_mapping
            );
        }
        self.next_allocation = start;

        Ok(start)
    }

    pub fn alloc(&mut self, size: usize, readonly: bool) -> Result<VmMem<u8>> {
        let old_start = self.next_allocation;
        let start = self.reserve_range(size)?;
        let res = self.hv.vm_add_mem(start as u64, size, readonly);
        if res.is_err() {
            self.next_allocation = old_start;
        }
        res
    }

    pub fn alloc_mmio_range(&mut self, size: usize) -> Result<MmioRange> {
        let start = self.reserve_range(size)?;
        Ok(try_with!(
            MmioRange::new(MmioAddress(start as u64), size as u64),
            "failed to allocate mmio range"
        ))
    }
}
