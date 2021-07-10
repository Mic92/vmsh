use std::sync::Arc;

use log::debug;
use simple_error::{bail, require_with, try_with};
use vm_device::bus::{MmioAddress, MmioRange};

use crate::tracer::proc::Mapping;
use crate::{page_math, result::Result};

use super::hypervisor::{Hypervisor, VmMem};

pub struct PhysMemAllocator {
    hv: Arc<Hypervisor>,
    /// the last memory allocation of the VM
    last_mapping: Mapping,
    /// Physical address where we last allocated memory from.
    /// After an allocating we substract the allocation size from this value.
    next_allocation: usize,
}

const ADDRESS_SIZE_FUNCTION: u32 = 0x80000008;
fn get_first_allocation(hv: &Arc<Hypervisor>) -> Result<usize> {
    let host_cpuid = unsafe { core::arch::x86_64::__cpuid(ADDRESS_SIZE_FUNCTION) };
    let vm_cpuid = try_with!(hv.get_cpuid2(&hv.vcpus[0]), "cannot get cpuid2");
    // Get Virtual and Physical address Sizes
    let entry = vm_cpuid
        .entries
        .iter()
        .find(|c| c.function == ADDRESS_SIZE_FUNCTION);
    let entry = require_with!(
        entry,
        "could not get the vm's cpuid entry for virtual and physical address size"
    );
    // Supported guest physical address size in bits
    let vm_phys_bits = entry.eax as u8;
    // Supported guest virtual address size in bits
    let vm_virt_bits = (entry.eax >> 8) as u8;
    debug!(
        "vm/cpu: phys_bits: {}, virt_bits: {}",
        vm_phys_bits, vm_virt_bits
    );
    // Supported host physical address size in bits
    let host_phys_bits = host_cpuid.eax as u8;
    // Supported host virtual address size in bits
    let host_virt_bits = (host_cpuid.eax >> 8) as u8;
    debug!(
        "host/cpu: phys_bits: {}, virt_bits: {}",
        host_phys_bits, host_virt_bits
    );
    Ok(std::cmp::min(1 << vm_phys_bits, 1 << host_phys_bits))
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

        let last_mapping = require_with!(
            maps.iter().max_by_key(|m| m.phys_addr + m.size()),
            "vm has no memory assigned"
        )
        .clone();
        let next_allocation = get_first_allocation(&hv)?;
        Ok(Self {
            hv,
            last_mapping,
            next_allocation,
            //next_allocation: 0xd0000000 + 0x1000 * 2,
        })
    }

    fn reserve_range(&mut self, size: usize) -> Result<usize> {
        let start = require_with!(self.next_allocation.checked_sub(size), "out of memory");
        let last_alloc = self.last_mapping.phys_addr + self.last_mapping.size();
        if start < last_alloc {
            bail!(
                "cannot allocate memory at {:x}, our allocator conflicts with mapping at {:x} ({:x}B)\
                   This might happen if the last vmsh run did not clean up memory\
                   correctly or the hypervisor has allocated memory at the very end of\
                   the physical address space",
                start, self.last_mapping.start, self.last_mapping.size()
            );
        }
        self.next_allocation = start;

        Ok(start)
    }

    pub fn alloc(&mut self, size: usize, readonly: bool) -> Result<VmMem<u8>> {
        let old_start = self.next_allocation;
        let padded_size = page_math::page_align(size);
        let start = self.reserve_range(padded_size)?;
        let res = self.hv.vm_add_mem(start as u64, padded_size, readonly);
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
