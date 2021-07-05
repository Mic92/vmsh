use crate::result::Result;
use crate::tracer::wrap_syscall::{MmioRw, MMIO_RW_DATA_MAX};
use simple_error::map_err_with;
use std::sync::Arc;
use virtio_device::WithDriverSelect;
use virtio_queue::Queue;
use vm_device::bus::{Bus, BusManager, MmioAddress};
use vm_device::device_manager::MmioManager;
use vm_device::DeviceMmio;
use vm_memory::GuestAddressSpace;

// Required by the Virtio MMIO device register layout at offset 0 from base. Turns out this
// is actually the ASCII sequence for "virt" (in little endian ordering).
const MMIO_MAGIC_VALUE: u32 = 0x7472_6976;

// Current version specified by the Virtio standard (legacy devices used 1 here).
const MMIO_VERSION: u32 = 2;

// TODO: Crosvm was using 0 here a while ago, and Firecracker started doing that as well. Should
// we leave it like that, or should we use the VENDOR_ID value for PCI Virtio devices? It looks
// like the standard doesn't say anything regarding an actual VENDOR_ID value for MMIO devices.
const VENDOR_ID: u32 = 0;

type MmioPirateBus<D> = Bus<MmioAddress, D>;

/// Replacement for vm_device::device_manager::IoManager.
/// Can implement MmioManager via vm_device::device_manager::MmioManager.
pub struct IoPirate {
    /// mmio device spaces typically accessed by VM exit mmio
    mmio_bus: MmioPirateBus<Arc<dyn DeviceMmio + Send + Sync>>,
}

impl Default for IoPirate {
    fn default() -> IoPirate {
        IoPirate {
            mmio_bus: Bus::new(),
        }
    }
}

impl IoPirate {
    //pub fn register_mmio_device(
    //    &mut self,
    //    range: MmioRange,
    //    blkdev: Arc<Mutex<Block>>,
    //) -> Result<()> {
    //    map_err_with!(
    //        self.mmio_bus.register(range, blkdev),
    //        "cannot register mmio device on MmioPirateBus"
    //    )?;
    //    Ok(())
    //}

    pub fn handle_mmio_rw(&mut self, mmio_rw: &mut MmioRw) -> Result<()> {
        if mmio_rw.is_write {
            map_err_with!(
                self.mmio_write(MmioAddress(mmio_rw.addr), mmio_rw.data()),
                "write to mmio device (0x{:x}) failed",
                mmio_rw.addr
            )?;
        } else {
            let mut data = [0u8; MMIO_RW_DATA_MAX];
            let len = mmio_rw.data().len();
            let slice = &mut data[0..len];
            map_err_with!(
                self.mmio_read(MmioAddress(mmio_rw.addr), slice),
                "read from mmio device (0x{:x}) failed",
                mmio_rw.addr
            )?;
            mmio_rw.answer_read(slice)?;
        }
        Ok(())
    }
}

// Enables the automatic implementation of `MmioManager` for `IoManager`.
impl BusManager<MmioAddress> for IoPirate {
    type D = Arc<dyn DeviceMmio + Send + Sync>;

    fn bus(&self) -> &MmioPirateBus<Arc<dyn DeviceMmio + Send + Sync>> {
        &self.mmio_bus
    }

    fn bus_mut(&mut self) -> &mut MmioPirateBus<Arc<dyn DeviceMmio + Send + Sync>> {
        &mut self.mmio_bus
    }
}

/// padX fields are reserved for future use.
#[derive(Copy, Clone, Debug)]
#[repr(C)] // actually we want packed. Because that has undefined behaviour in rust 2018 we hope that C is effectively the same.
pub struct MmioDeviceSpace {
    pub magic_value: u32,
    pub version: u32,
    pub device_id: u32,
    pub vendor_id: u32,
    pub device_features: u32,
    pub device_features_sel: u32,
    pad1: [u32; 2],
    pub driver_features: u32,
    /// beyond 32bit there are further feature bits reserved for future use
    pub driver_features_sel: u32,
    pad2: [u32; 2],
    pub queue_sel: u32,
    pub queue_num_max: u32,
    pub queue_num: u32,
    pad3: [u32; 2],
    pub queue_ready: u32,
    pad4: [u32; 2],
    pub queue_notify: u32,
    pad5: [u32; 3],
    pub interrupt_status: u32,
    pub interrupt_ack: u32,
    pad6: [u32; 2],
    pub status: u32,
    pad7: [u32; 3],
    /// 64bit phys addr
    pub queue_desc_low: u32,
    pub queue_desc_high: u32,
    pad8: [u32; 2],
    pub queue_driver_low: u32,
    pub queue_driver_high: u32,
    pad9: [u32; 2],
    pub queue_device_low: u32,
    pub queue_device_high: u32,
    pad10: [u32; 21],
    pub config_generation: u32,
    // optional additional config space: config: [u8; n]
}

impl MmioDeviceSpace {
    pub fn new<M: GuestAddressSpace, E>(
        device: &dyn WithDriverSelect<M, E = E>,
    ) -> MmioDeviceSpace {
        MmioDeviceSpace {
            magic_value: MMIO_MAGIC_VALUE,
            version: MMIO_VERSION,
            device_id: device.device_type(),
            vendor_id: VENDOR_ID,
            device_features: device.device_features() as u32,
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
        use crate::devices::mmio::MmioDeviceSpace;
        use std::mem::size_of;
        assert_eq!(0x100, size_of::<MmioDeviceSpace>());
    }
}
