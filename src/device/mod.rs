pub mod mmio;
mod virtio;

use crate::device::mmio::{IoPirate, MmioDeviceSpace};
use crate::device::virtio::block::{self, BlockArgs};
use crate::device::virtio::{CommonArgs, MmioConfig};
use crate::kvm::hypervisor::{Hypervisor, VmMem};
use crate::result::Result;
use crate::tracer::proc::Mapping;
use event_manager::{EventManager, MutEventSubscriber};
use simple_error::{bail, try_with};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use vm_device::bus::{MmioAddress, MmioRange};
use vm_device::device_manager::IoManager;
use vm_device::device_manager::MmioManager;
use vm_device::resources::ResourceConstraint;
use vm_memory::guest_memory::GuestAddress;
use vm_memory::mmap::MmapRegion;
use vm_memory::GuestMemoryRegion;
use vm_memory::{GuestMemoryMmap, GuestRegionMmap};
use vm_virtio::device::status::RESET;

// Where BIOS/VGA magic would live on a real PC.
#[allow(dead_code)] // FIXME
const EBDA_START: u64 = 0x9fc00;
const FIRST_ADDR_PAST_32BITS: u64 = 1 << 32;
const MEM_32BIT_GAP_SIZE: u64 = 768 << 20;
/// The start of the memory area reserved for MMIO devices.
pub const MMIO_MEM_START: u64 = FIRST_ADDR_PAST_32BITS - MEM_32BIT_GAP_SIZE;

pub type Block = block::Block<Arc<GuestMemoryMmap>>;

fn convert(mappings: &[Mapping]) -> Result<GuestMemoryMmap> {
    let mut regions: Vec<Arc<GuestRegionMmap>> = vec![];

    println!("{}", mappings.len());
    for mapping in mappings {
        // TODO need reason for why this is safe. ("a smart human wrote it")
        let mmap_region = try_with!(
            unsafe {
                MmapRegion::build_raw(
                    mapping.start as *mut u8,
                    (mapping.end - mapping.start) as usize,
                    mapping.prot_flags.bits(),
                    mapping.map_flags.bits(),
                )
            },
            "cannot instanciate MmapRegion"
        );

        let guest_region_mmap = try_with!(
            GuestRegionMmap::new(mmap_region, GuestAddress(mapping.phys_addr as u64)),
            "cannot allocate guest region"
        );

        regions.push(Arc::new(guest_region_mmap));
    }

    // sort after guest address
    regions.sort_unstable_by_key(|r| r.start_addr());

    // trows regions overlap error because start_addr (guest) is 0 for all regions.
    Ok(try_with!(
        GuestMemoryMmap::from_arc_regions(regions),
        "GuestMemoryMmap error"
    ))
}

#[allow(dead_code)] // FIXME
pub struct Device {
    vmm: Arc<Hypervisor>,
    pub blkdev: Arc<Mutex<Block>>, // FIXME this is an Arc Mutex Arc Mutex Block
    /// None if not attached to the Hv
    pub mmio_device_mem: Option<VmMem<MmioDeviceSpace>>,
    pub mmio_device_space: MmioDeviceSpace,
    pub mmio_mgr: Arc<Mutex<IoPirate>>,
}

impl Device {
    pub fn new(vmm: &Arc<Hypervisor>) -> Result<Device> {
        let guest_memory = try_with!(vmm.get_maps(), "cannot get guests memory");
        let mem: Arc<GuestMemoryMmap> = Arc::new(try_with!(
            convert(&guest_memory),
            "cannot convert Mapping to GuestMemoryMmap"
        ));

        println!("mmio range start {:x}", MMIO_MEM_START);
        let range = MmioRange::new(MmioAddress(MMIO_MEM_START), 0x1000).unwrap();
        let mmio_cfg = MmioConfig { range, gsi: 5 };

        // TODO is there more we have to do with this mgr?
        let device_manager = Arc::new(Mutex::new(IoManager::new()));
        let _guard = device_manager.lock().unwrap();
        // IoManager replacement:
        let device_manager = Arc::new(Mutex::new(IoPirate::new()));
        let guard = device_manager.lock().unwrap();
        guard.mmio_device(MmioAddress(MMIO_MEM_START));

        let mut event_manager = try_with!(
            EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new(),
            "cannot create event manager"
        );
        // TODO add subscriber (wrapped_exit_handler) and stuff?

        let common = CommonArgs {
            mem,
            vmm: vmm.clone(),
            event_mgr: &mut event_manager,
            mmio_mgr: guard,
            mmio_cfg,
        };

        let args = BlockArgs {
            common,
            file_path: PathBuf::from("/dev/null"),
            read_only: false,
            root_device: true,
            advertise_flush: true,
        };

        let blkdev: Arc<Mutex<Block>> = match Block::new(args) {
            Ok(v) => v,
            Err(e) => bail!("cannot create block device: {:?}", e),
        };

        // create device space
        let mmio_dev_space;
        {
            let blkdev = blkdev.clone();
            let blkdev = blkdev.lock().unwrap();
            mmio_dev_space = MmioDeviceSpace::new(&blkdev);
        }

        let mut device = Device {
            vmm: vmm.clone(),
            blkdev,
            mmio_device_mem: None,
            mmio_device_space: mmio_dev_space,
            mmio_mgr: device_manager,
        };

        device.attach_device_space()?;
        Ok(device)
    }

    // vmm.stopped
    pub fn attach_device_space(&mut self) -> Result<()> {
        let mmio_device_mem = self.vmm.vm_add_mem(MMIO_MEM_START, false)?;
        self.mmio_device_mem = Some(mmio_device_mem);
        self.mmio_device_mem
            .as_ref()
            .unwrap()
            .mem
            .write(&self.mmio_device_space)?;
        Ok(())
    }

    pub fn update_device_mem(&self) -> Result<()> {
        let mmio_device_mem = self
            .mmio_device_mem
            .as_ref()
            .expect("don't call this function when there is not device space attached");
        mmio_device_mem.mem.write(&self.mmio_device_space)?;
        Ok(())
    }

    pub fn create(&self) {
        let _a = RESET;
        let _b = ResourceConstraint::new_pio(1);
        println!("create device");
    }
}
