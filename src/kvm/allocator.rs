use std::sync::Arc;

use crate::{
    guest_mem::{GuestMem, MappedMemory},
    page_table::{estimate_page_table_size, VirtMem},
};
use log::debug;
use nix::sys::mman::ProtFlags;
use simple_error::{bail, require_with, try_with};
use vm_device::bus::{MmioAddress, MmioRange};

use crate::{page_math, result::Result};

use super::hypervisor::{Hypervisor, PhysMem};

pub struct PhysMemAllocator {
    pub hv: Arc<Hypervisor>,
    /// Physical guest memory
    pub guest_mem: GuestMem,
    /// Physical address where we last allocated memory from.
    /// After an allocating we substract the allocation size from this value.
    next_allocation: usize,
}

const EXTEND_CPU_INFO_FUNCTION: u32 = 0x80000001;
const ENCRYPTED_MEMORY_CAPABILITIES: u32 = 0x8000001f;
const ADDRESS_SIZE_FUNCTION: u32 = 0x80000008;
const LONG_MODE: u32 = 1 << 29;

fn sme_sev_bits() -> u8 {
    // get AMD Encrypted Memory Capabilities information according to AMD doc 24594—Rev. 3.32
    let host_cpuid = unsafe { core::arch::x86_64::__cpuid(ENCRYPTED_MEMORY_CAPABILITIES) };
    //host_cpuid(0x8000001f, 0, &eax, &ebx, NULL, NULL);
    // bits:
    // 0:1 SME
    // 1:2 SEV
    // ...
    if host_cpuid.eax & 1 != 0 || host_cpuid.eax & 2 != 0 {
        // bits:
        // 11:6 PhysAddrReduction
        ((host_cpuid.ebx >> 6) & 0x3f) as u8
    } else {
        0
    }
}

fn get_first_allocation(hv: &Arc<Hypervisor>) -> Result<usize> {
    let host_cpuid = unsafe { core::arch::x86_64::__cpuid(ADDRESS_SIZE_FUNCTION) };
    let vm_cpuid = try_with!(hv.get_cpuid2(&hv.vcpus[0]), "cannot get cpuid2");
    // Get Extended Processor Info and Feature Bits
    let extended_cpu_info_entry = vm_cpuid
        .entries
        .iter()
        .find(|c| c.function == EXTEND_CPU_INFO_FUNCTION);
    let extended_cpu_info_entry = require_with!(
        extended_cpu_info_entry,
        "could not get the vm's cpuid entry for extended processor info and features bits ({})",
        EXTEND_CPU_INFO_FUNCTION
    );
    if extended_cpu_info_entry.edx & LONG_MODE == 0 {
        bail!("VM is not 64-bit");
    }

    // Get Virtual and Physical address sizes
    let address_size_entry = vm_cpuid
        .entries
        .iter()
        .find(|c| c.function == ADDRESS_SIZE_FUNCTION);
    let address_size_entry = require_with!(
        address_size_entry,
        "could not get the vm's cpuid entry for virtual and physical address size ({})",
        ADDRESS_SIZE_FUNCTION
    );
    // Supported guest physical address size in bits
    let vm_phys_bits = address_size_entry.eax as u8;
    // Supported guest virtual address size in bits
    let vm_virt_bits = (address_size_entry.eax >> 8) as u8;
    debug!(
        "vm/cpu: phys_bits: {}, virt_bits: {}",
        vm_phys_bits, vm_virt_bits
    );
    // we only implement 4-Level paging for now
    if vm_virt_bits != 48 {
        bail!(
            "VM cpu uses {} bits for virtual addresses. This is unsupported at the moment",
            vm_virt_bits
        );
    }
    // Supported host physical address size in bits
    let host_phys_bits = host_cpuid.eax as u8 - sme_sev_bits();
    // Supported host virtual address size in bits
    let host_virt_bits = (host_cpuid.eax >> 8) as u8;
    debug!(
        "host/cpu: phys_bits: {}, virt_bits: {}",
        host_phys_bits, host_virt_bits
    );
    Ok(std::cmp::min(1 << vm_phys_bits, 1 << host_phys_bits))
}

pub struct VirtAlloc {
    pub len: usize,
    pub prot: ProtFlags,
}

impl PhysMemAllocator {
    pub fn new(hv: Arc<Hypervisor>) -> Result<Self> {
        let next_allocation = get_first_allocation(&hv)?;
        let guest_mem = GuestMem::new(&hv)?;
        Ok(Self {
            hv,
            guest_mem,
            next_allocation,
            //next_allocation: 0xd0000000 + 0x1000 * 2,
        })
    }

    fn reserve_range(&mut self, size: usize) -> Result<usize> {
        let start = require_with!(self.next_allocation.checked_sub(size), "out of memory");
        let last_mapping =
            require_with!(self.guest_mem.last_mapping(), "vm has no memory assigned");
        let last_alloc = last_mapping.phys_addr + last_mapping.size();
        if start < last_alloc {
            bail!(
                "cannot allocate memory at {:x}, our allocator conflicts with mapping at {:x} ({:x}B). \
                   This might happen if the last vmsh run did not clean up memory \
                   correctly or the hypervisor has allocated memory at the very end of \
                   the physical address space",
                start, last_mapping.start, last_mapping.size()
            );
        }
        self.next_allocation = start;

        Ok(start)
    }

    pub fn phys_alloc(&mut self, size: usize, readonly: bool) -> Result<PhysMem<u8>> {
        let old_start = self.next_allocation;
        let padded_size = page_math::page_align(size);
        let start = self.reserve_range(padded_size)?;
        let res = self.hv.vm_add_mem(start as u64, padded_size, readonly);
        if res.is_err() {
            self.next_allocation = old_start;
        }
        res
    }
    pub fn virt_alloc(&mut self, mut virt_start: usize, alloc: &[VirtAlloc]) -> Result<VirtMem> {
        let len = alloc.iter().map(|a| a.len).sum();
        let phys_mem = self.phys_alloc(estimate_page_table_size(len), false)?;

        let mut next_addr = phys_mem.guest_phys_addr.clone();

        let mapped_mem = alloc
            .iter()
            .map(|a| {
                let m = MappedMemory {
                    phys_start: next_addr.clone(),
                    virt_start,
                    len: a.len,
                    prot: a.prot,
                };
                virt_start += a.len;
                next_addr.value += a.len;
                m
            })
            .collect::<Vec<MappedMemory>>();

        self.guest_mem
            .map_memory(self.hv.clone(), phys_mem, &mapped_mem)
    }

    pub fn alloc_mmio_range(&mut self, size: usize) -> Result<MmioRange> {
        let start = self.reserve_range(size)?;
        Ok(try_with!(
            MmioRange::new(MmioAddress(start as u64), size as u64),
            "failed to allocate mmio range"
        ))
    }
}
