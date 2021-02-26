
mod virtio;

use vm_virtio::device::status::RESET;
use vm_device::resources::ResourceConstraint;
use vm_memory::{GuestMemoryMmap, GuestRegionMmap, FileOffset};
use vm_memory::guest_memory::GuestAddress;
use vm_memory::mmap::MmapRegion;
use std::sync::{Arc, Mutex};
use simple_error::try_with;
use std::path::PathBuf;
use vm_device::bus::{MmioAddress, MmioRange};

use crate::kvm::Hypervisor;
use crate::device::virtio::block::{self, BlockArgs};
use crate::device::virtio::{CommonArgs, MmioConfig};
use crate::proc::Mapping;

// Where BIOS/VGA magic would live on a real PC.
const EBDA_START: u64 = 0x9fc00;
const FIRST_ADDR_PAST_32BITS: u64 = 1 << 32;
const MEM_32BIT_GAP_SIZE: u64 = 768 << 20;
/// The start of the memory area reserved for MMIO devices.
pub const MMIO_MEM_START: u64 = FIRST_ADDR_PAST_32BITS - MEM_32BIT_GAP_SIZE;

type Block = block::Block<Arc<GuestMemoryMmap>>;

fn convert(mappings: &Vec<Mapping>) -> GuestMemoryMmap {
    let regions: Vec<Arc<GuestRegionMmap>> = vec!{};

    for mapping in mappings {
        let file = Arc::new(std::fs::File::open(&mapping.pathname).expect("could not open file")); // TODO formatted

        let file_offset = FileOffset {
            file,
            start: mapping.offset,
        };

        let mmap_region = MmapRegion {
            /* TODO check correctness
             * addr_a: *mut c_void = mmap(...);
             * addr = addr_a as *mut u8;
             * mapping.start: u64
             * ???
             */
            addr: mapping.start as *mut u8,
            size: (mapping.end - mapping.start) as usize,
            file_offset: Some(file_offset), // is not actually optional
            prot: mapping.prot_flags.bits(),
            flags: mapping.map_flags.bits(),
            owned: false,
            hugetlbfs: None,
        };

        let guest_region_mmap = GuestRegionMmap {
            mapping: mmap_region,
            guest_base: GuestAddress(mapping.phys_addr),
        };

        regions.push(Arc::new(guest_region_mmap));
    }

    GuestMemoryMmap { regions }
}

pub struct Device { 
    vmm: Arc<Hypervisor>,
    blkdev: Arc<Mutex<Block>>,
}

impl Device {
    pub fn new(vmm: Arc<Hypervisor>) -> Device {

        let mem: Arc<GuestMemoryMmap> = Arc::new(convert(&vmm.mappings));

        let range = MmioRange::new(MmioAddress(MMIO_MEM_START), 0x1000).unwrap();
        let mmio_cfg = MmioConfig { range, gsi: 5 };

        let mut guard = None;

        let common = CommonArgs {
            mem,
            vmm,
            event_mgr: &mut self.event_mgr, // TODO
            mmio_mgr: guard.deref_mut(), // TODO
            mmio_cfg,
        };

        let args = BlockArgs {
            common,
            file_path: PathBuf::from("/tmp/foobar"),
            read_only: false,
            root_device: true,
            advertise_flush: true,
        };

        let blkdev: Arc<Mutex<Block>> = Block::new(args).expect("cannot create block device");
        Device {
            vmm,
            blkdev,
        }

    }

    pub fn create(&self) {
        let a = RESET;
        let b = ResourceConstraint::new_pio(1);
        println!("create device");


    }
}
