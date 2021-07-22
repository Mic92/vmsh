use crate::result::Result;
use crate::tracer::wrap_syscall::{MmioRw, MMIO_RW_DATA_MAX};
use simple_error::{map_err_with,try_with};
use std::sync::Arc;
use vm_device::bus::{Bus, BusManager, MmioAddress};
use vm_device::device_manager::MmioManager;
use vm_device::DeviceMmio;
use crate::kvm::ioctls::{ioregionfd_cmd, Cmd};
use crate::kvm::hypervisor::IoRegionFd;

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

    /// Used with MmioExitWrapper.
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

    /// Used with IoRegionFd.
    pub fn handle_ioregion_rw(&mut self, ioregionfd: &IoRegionFd, mut rw: ioregionfd_cmd) -> Result<()> {
        let addr = ioregionfd.ioregion.guest_paddr + rw.offset;
        let res = match rw.info.cmd() {
            Cmd::Write => {
                let data = rw.data();
                map_err_with!(
                    self.mmio_write(MmioAddress(addr), data),
                    "write to mmio device (0x{:x}) failed",
                    addr
                )?;
                // must be acknowledged with an arbitrary response
                ioregionfd.write(0)
            }
            Cmd::Read => {
                let data = rw.data_mut();
                map_err_with!(
                    self.mmio_read(MmioAddress(addr), data),
                    "read from mmio device (0x{:x}) failed",
                    addr
                )?;
                ioregionfd.write_slice(data)
            }
        };
        Ok(try_with!(res, "cannot handle ioregion command"))
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
