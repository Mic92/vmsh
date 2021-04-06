mod mmio;
mod virtio;

use crate::device::mmio::MmioDeviceSpace;
use crate::device::virtio::block::{self, BlockArgs};
use crate::device::virtio::{CommonArgs, MmioConfig};
use crate::kvm::hypervisor::{Hypervisor, VmMem};
use crate::proc::Mapping;
use crate::result::Result;
use event_manager::{EventManager, MutEventSubscriber};
use simple_error::{bail, try_with};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use vm_device::bus::{MmioAddress, MmioRange};
use vm_device::device_manager::IoManager;
use vm_device::resources::ResourceConstraint;
use vm_memory::guest_memory::GuestAddress;
use vm_memory::mmap::MmapRegion;
use vm_memory::GuestMemoryRegion;
use vm_memory::{GuestMemoryMmap, GuestRegionMmap};
use vm_virtio::device::status::RESET;

// Where BIOS/VGA magic would live on a real PC.
const EBDA_START: u64 = 0x9fc00;
const FIRST_ADDR_PAST_32BITS: u64 = 1 << 32;
const MEM_32BIT_GAP_SIZE: u64 = 768 << 20;
/// The start of the memory area reserved for MMIO devices.
pub const MMIO_MEM_START: u64 = FIRST_ADDR_PAST_32BITS - MEM_32BIT_GAP_SIZE;

type Block = block::Block<Arc<GuestMemoryMmap>>;

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

pub struct Device {
    vmm: Arc<Hypervisor>,
    blkdev: Arc<Mutex<Block>>,
    mmio_device_mem: VmMem<MmioDeviceSpace>,
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
        let guard = device_manager.lock().unwrap();

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

        let mmio_device_mem = vmm.vm_add_mem(MMIO_MEM_START, false)?;
        {
            let blkdev = blkdev.clone();
            let blkdev = blkdev.lock().unwrap();
            let mmio_dev_space = MmioDeviceSpace::new(&blkdev);
            mmio_device_mem.mem.write(&mmio_dev_space)?;
        }

        let device = Device {
            vmm: vmm.clone(),
            blkdev,
            mmio_device_mem,
        };
        Ok(device)
    }

    pub fn create(&self) {
        let _a = RESET;
        let _b = ResourceConstraint::new_pio(1);
        println!("create device");
    }
}
