///! Bindings for ioregionfd.
use num_derive::*;
use num_traits as num;
use std::mem::size_of;
use std::os::unix::prelude::RawFd;

#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct kvm_ioregion {
    pub guest_paddr: u64, // guest physical address
    pub memory_size: u64, // bytes
    pub user_data: u64,
    pub rfd: i32,
    pub wfd: i32,
    pub flags: u32,
    pub pad: [u8; 28],
}

impl kvm_ioregion {
    pub fn new(guest_paddr: u64, len: usize, rfd: RawFd, wfd: RawFd) -> Self {
        kvm_ioregion {
            guest_paddr,
            memory_size: len as u64,
            user_data: 0,
            rfd,
            wfd,
            flags: 0,
            pad: [0; 28],
        }
    }
}

#[allow(non_camel_case_types)]
enum kvm_ioregion_flag_nr {
    pio,
    posted_writes,
    max,
}

// TODO bitflags!?
//#define KVM_IOREGION_PIO (1 << kvm_ioregion_flag_nr_pio)
pub const KVM_IOREGION_PIO: u32 = 1 << kvm_ioregion_flag_nr::pio as u32;
//#define KVM_IOREGION_POSTED_WRITES (1 << kvm_ioregion_flag_nr_posted_writes)
pub const KVM_IOREGION_POSTED_WRITES: u32 = 1 << kvm_ioregion_flag_nr::posted_writes as u32;
//#define KVM_IOREGION_VALID_FLAG_MASK ((1 << kvm_ioregion_flag_nr_max) - 1)
pub const KVM_IOREGION_VALID_FLAG_MASK: u32 = (1 << kvm_ioregion_flag_nr::max as u32) - 1;

pub const KVM_CAP_IOREGIONFD: u32 = 195;

/// wire protocol guest->host
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct ioregionfd_cmd {
    pub info: Info,
    pub pad: u32,
    pub user_data: u64,
    pub offset: u64,
    pub data: u64,
}

impl ioregionfd_cmd {
    pub fn data(&self) -> &[u8] {
        let data = unsafe {
            std::slice::from_raw_parts((&self.data as *const u64) as *const u8, size_of::<u64>())
        };
        match self.info.size() {
            Size::b8 => &data[0..1],
            Size::b16 => &data[0..2],
            Size::b32 => &data[0..4],
            Size::b64 => &data[0..8],
        }
    }
    pub fn data_mut(&mut self) -> &mut [u8] {
        let data = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut self.data as *mut u64) as *mut u8,
                size_of::<u64>(),
            )
        };
        match self.info.size() {
            Size::b8 => &mut data[0..1],
            Size::b16 => &mut data[0..2],
            Size::b32 => &mut data[0..4],
            Size::b64 => &mut data[0..8],
        }
    }
}

/// wire protocol host->guest
#[allow(non_camel_case_types)]
pub struct ioregionfd_resp {
    pub data: u64,
    pub pad: [u8; 24],
}

impl ioregionfd_resp {
    pub fn new(data: u64) -> Self {
        ioregionfd_resp { data, pad: [0; 24] }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct Info(u32);

const IOREGIONFD_CMD_OFFSET: usize = 0;
const IOREGIONFD_CMD_LEN: usize = 1;
const IOREGIONFD_SIZE_OFFSET: usize = 4;
const IOREGIONFD_SIZE_LEN: usize = 2;
const IOREGIONFD_RESP_OFFSET: usize = 6;
const IOREGIONFD_RESP_LEN: usize = 1;
//#define IOREGIONFD_SIZE(x) ((x) << IOREGIONFD_SIZE_OFFSET)
//#define IOREGIONFD_RESP(x) ((x) << IOREGIONFD_RESP_OFFSET)
impl Info {
    pub fn new(cmd: Cmd, size: Size, response: bool) -> Self {
        let mut ret = 0;
        ret |= (cmd as u32) << IOREGIONFD_CMD_OFFSET;
        ret |= (size as u32) << IOREGIONFD_SIZE_OFFSET;
        ret |= (response as u32) << IOREGIONFD_RESP_OFFSET;
        Info(ret)
    }

    pub fn cmd(&self) -> Cmd {
        let mut i: u32 = self.0 >> IOREGIONFD_CMD_OFFSET;
        let valid_bits = !(!0 << IOREGIONFD_CMD_LEN);
        i &= valid_bits;
        num::FromPrimitive::from_u32(i).unwrap_or(Cmd::Write)
    }

    pub fn size(&self) -> Size {
        let mut i: u32 = self.0 >> IOREGIONFD_SIZE_OFFSET;
        let valid_bits = !(!0 << IOREGIONFD_SIZE_LEN);
        i &= valid_bits;
        num::FromPrimitive::from_u32(i).unwrap_or(Size::b8)
    }

    pub fn is_response(&self) -> bool {
        let mut i: u32 = self.0 >> IOREGIONFD_RESP_OFFSET;
        let valid_bits = !(!0 << IOREGIONFD_RESP_LEN);
        i &= valid_bits;
        i == 0
    }
}

// pub const IOREGIONFD_CMD_READ: usize = 0;
// pub const IOREGIONFD_CMD_WRITE: usize = 1;
#[derive(Debug, FromPrimitive)]
pub enum Cmd {
    Read,
    Write,
}

//pub const IOREGIONFD_SIZE_8BIT: usize = 0;
//pub const IOREGIONFD_SIZE_16BIT: usize = 1;
//pub const IOREGIONFD_SIZE_32BIT: usize = 2;
//pub const IOREGIONFD_SIZE_64BIT: usize = 3;
#[allow(non_camel_case_types)]
#[derive(Debug, FromPrimitive)]
pub enum Size {
    b8,
    b16,
    b32,
    b64,
}
