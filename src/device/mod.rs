
pub mod virtio;

use crate::kvm::Hypervisor;
use vm_virtio::device::status::RESET;
use vm_device::resources::ResourceConstraint;

// Where BIOS/VGA magic would live on a real PC.
const EBDA_START: u64 = 0x9fc00;
const FIRST_ADDR_PAST_32BITS: u64 = 1 << 32;
const MEM_32BIT_GAP_SIZE: u64 = 768 << 20;
/// The start of the memory area reserved for MMIO devices.
pub const MMIO_MEM_START: u64 = FIRST_ADDR_PAST_32BITS - MEM_32BIT_GAP_SIZE;

pub struct Device { 
    vmm: Hypervisor,
}

impl Device {
    pub fn new(vmm: Hypervisor) -> Device {
        Device {
            vmm: vmm
        }
    }

    pub fn create(self) {
        let a = RESET;
        let b = ResourceConstraint::new_pio(1);
        println!("create device");


    }
}
