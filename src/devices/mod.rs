pub mod mmio;
mod threads;
mod virtio;

use crate::devices::mmio::IoPirate;
use crate::devices::threads::SubscriberEventManager;
use crate::devices::virtio::block::{self, BlockArgs};
use crate::devices::virtio::console::{self, ConsoleArgs};
use crate::devices::virtio::{CommonArgs, MmioConfig};
use crate::kvm::hypervisor::Hypervisor;
use crate::result::Result;
use crate::tracer::proc::Mapping;
use libc::pid_t;
use simple_error::{bail, try_with};
use std::path::Path;
use std::sync::{Arc, Mutex};
use vm_device::bus::{MmioAddress, MmioRange};
use vm_device::device_manager::MmioManager;
use vm_memory::guest_memory::GuestAddress;
use vm_memory::mmap::MmapRegion;
use vm_memory::GuestMemoryRegion;
use vm_memory::{GuestMemoryMmap, GuestRegionMmap};

pub use crate::devices::threads::create_devices;

// Where BIOS/VGA magic would live on a real PC.
#[allow(dead_code)] // FIXME
const EBDA_START: u64 = 0x9fc00;
const FIRST_ADDR_PAST_32BITS: u64 = 1 << 32;
const MEM_32BIT_GAP_SIZE: u64 = 768 << 20;
/// The start of the memory area reserved for MMIO devices.
pub const MMIO_MEM_START: u64 = FIRST_ADDR_PAST_32BITS - MEM_32BIT_GAP_SIZE;
/// max mem space per device
pub const DEVICE_MAX_MEM: u64 = 0x1000;
pub const BLOCK_MEM_START : u64 = MMIO_MEM_START;
pub const CONSOLE_MEM_START : u64 = BLOCK_MEM_START + DEVICE_MAX_MEM;
pub const MMIO_MEM_STOP: u64 = CONSOLE_MEM_START + DEVICE_MAX_MEM;


pub type Block = block::Block<Arc<GuestMemoryMmap>>;
pub type Console = console::Console<Arc<GuestMemoryMmap>>;

fn convert(pid: pid_t, mappings: &[Mapping]) -> Result<GuestMemoryMmap> {
    let mut regions: Vec<Arc<GuestRegionMmap>> = vec![];

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
            GuestRegionMmap::new(pid, mmap_region, GuestAddress(mapping.phys_addr as u64)),
            "cannot allocate guest region"
        );

        regions.push(Arc::new(guest_region_mmap));
    }

    // sort after guest address
    regions.sort_unstable_by_key(|r| r.start_addr());

    // trows regions overlap error because start_addr (guest) is 0 for all regions.
    Ok(try_with!(
        GuestMemoryMmap::from_arc_regions(pid, regions),
        "GuestMemoryMmap error"
    ))
}

pub struct DeviceSpace {
    pub blkdev: Arc<Mutex<Block>>,
    pub console: Arc<Mutex<Console>>,
    pub mmio_mgr: Arc<Mutex<IoPirate>>,
}

impl DeviceSpace {
    pub fn new(
        vmm: &Arc<Hypervisor>,
        event_mgr: &mut SubscriberEventManager,
        backing: &Path,
    ) -> Result<DeviceSpace> {
        let guest_memory = try_with!(vmm.get_maps(), "cannot get guests memory");
        let mem = Arc::new(try_with!(
            convert(vmm.pid.as_raw(), &guest_memory),
            "cannot convert Mapping to GuestMemoryMmap"
        ));

        let block_range = MmioRange::new(MmioAddress(BLOCK_MEM_START), 0x1000).unwrap();
        let block_mmio_cfg = MmioConfig { range: block_range, gsi: 5 };

        let console_range = MmioRange::new(MmioAddress(CONSOLE_MEM_START), 0x1000).unwrap();
        let console_mmio_cfg = MmioConfig { range: console_range, gsi: 5 };

        // IoManager replacement:
        let device_manager = Arc::new(Mutex::new(IoPirate::default()));
        let blkdev = {
          let guard = device_manager.lock().unwrap();
          guard.mmio_device(MmioAddress(BLOCK_MEM_START));

          let common = CommonArgs {
              mem: Arc::clone(&mem),
              vmm: vmm.clone(),
              event_mgr,
              mmio_mgr: guard,
              mmio_cfg: block_mmio_cfg,
          };
          let args = BlockArgs {
              common,
              file_path: backing.to_path_buf(),
              read_only: false,
              root_device: true,
              advertise_flush: true,
          };
          match Block::new(args) {
              Ok(v) => v,
              Err(e) => bail!("cannot create block device: {:?}", e),
          }
        };
        let console = {
            let guard = device_manager.lock().unwrap();
            guard.mmio_device(MmioAddress(CONSOLE_MEM_START));

            let common = CommonArgs {
                mem,
                vmm: vmm.clone(),
                event_mgr,
                mmio_mgr: guard,
                mmio_cfg: console_mmio_cfg,
            };
            let args = ConsoleArgs {
                common,
            };

            match Console::new(args) {
                Ok(v) => v,
                Err(e) => bail!("cannot create console device: {:?}", e),
            }
        };

        let device = DeviceSpace {
            blkdev,
            console,
            mmio_mgr: device_manager,
        };

        Ok(device)
    }
}
