// Required by the Virtio MMIO device register layout at offset 0 from base. Turns out this
// is actually the ASCII sequence for "virt" (in little endian ordering).
const MMIO_MAGIC_VALUE: u32 = 0x7472_6976;

// Current version specified by the Virtio standard (legacy devices used 1 here).
const MMIO_VERSION: u32 = 2;

// TODO: Crosvm was using 0 here a while ago, and Firecracker started doing that as well. Should
// we leave it like that, or should we use the VENDOR_ID value for PCI Virtio devices? It looks
// like the standard doesn't say anything regarding an actual VENDOR_ID value for MMIO devices.
const VENDOR_ID: u32 = 0;

/// padX fields are reserved for future use.
#[derive(Copy, Clone, Debug)]
#[repr(C)] // actually we want packed. Because that has undefined behaviour in rust 2018 we hope that C is effectively the same.
pub struct MmioDeviceSpace {
    magic_value: u32,
    version: u32,
    device_id: u32,
    vendor_id: u32,
    device_features: u32,
    device_features_sel: u32,
    pad1: [u32; 2],
    driver_features: u32,
    /// beyond 32bit there are further feature bits reserved for future use
    driver_features_sel: u32,
    pad2: [u32; 2],
    queue_sel: u32,
    queue_num_max: u32,
    queue_num: u32,
    pad3: [u32; 2],
    queue_ready: u32,
    pad4: [u32; 2],
    queue_notify: u32,
    pad5: [u32; 3],
    interrupt_status: u32,
    interrupt_ack: u32,
    pad6: [u32; 2],
    status: u32,
    pad7: [u32; 3],
    /// 64bit phys addr
    queue_desc_low: u32,
    queue_desc_high: u32,
    pad8: [u32; 2],
    queue_driver_low: u32,
    queue_driver_high: u32,
    pad9: [u32; 2],
    queue_device_low: u32,
    queue_device_high: u32,
    pad10: [u32; 21],
    config_generation: u32,
    // optional additional config space: config: [u8; n]
}

use crate::device::virtio::block;
use std::sync::Arc;
use vm_memory::{GuestMemoryMmap, GuestRegionMmap};
use vm_virtio::device::{VirtioDevice, WithDriverSelect};
use vm_virtio::Queue;

type Block = block::Block<Arc<GuestMemoryMmap>>;

impl MmioDeviceSpace {
    pub fn new(device: &Block) -> MmioDeviceSpace {
        MmioDeviceSpace {
            magic_value: MMIO_MAGIC_VALUE,
            version: MMIO_VERSION,
            device_id: device.device_type(),
            vendor_id: VENDOR_ID,
            device_features: device.device_features(device.device_features_select()),
            device_features_sel: 0,
            pad1: [0u32; 2],
            driver_features: 0, // TODO properly init this field and others
            driver_features_sel: 0,
            pad2: [0u32; 2],
            queue_sel: 0,
            queue_num_max: device
                .selected_queue()
                .map(Queue::max_size)
                .unwrap_or(0)
                .into(),
            queue_num: 0,
            pad3: [0u32; 2],
            queue_ready: 0,
            pad4: [0u32; 2],
            queue_notify: 0,
            pad5: [0u32; 3],
            interrupt_status: 0,
            interrupt_ack: 0,
            pad6: [0u32; 2],
            status: device.device_status().into(),
            pad7: [0u32; 3],
            queue_desc_low: 0,
            queue_desc_high: 0,
            pad8: [0u32; 2],
            queue_driver_low: 0,
            queue_driver_high: 0,
            pad9: [0u32; 2],
            queue_device_low: 0,
            queue_device_high: 0,
            pad10: [0u32; 21],
            config_generation: device.config_generation().into(),
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn assert_mmio_device_space_size() {
        use crate::device::mmio::MmioDeviceSpace;
        use std::mem::size_of;
        assert_eq!(0x100, size_of::<MmioDeviceSpace>());
    }
}
