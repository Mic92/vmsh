#![no_std]

use chlorine::{c_char, c_ulonglong};

/// Holds the device we create by this code, so we can unregister it later
pub const MAX_DEVICES: usize = 3;
pub const MAX_ARGV: usize = 256;
/// ideally we could have our own IRQ here... 6 seems so far shareable with other devices

#[derive(PartialEq, Copy, Clone, Debug)]
#[repr(C)]
pub enum DeviceState {
    Undefined = 0,
    Initializing = 1,
    Ready = 2,
    Terminating = 3,
    Error = 4,
}

#[repr(C)]
pub struct Stage1Args {
    /// physical mmio addresses
    pub device_addrs: [c_ulonglong; MAX_DEVICES],
    /// null terminated array
    /// the first argument is always stage2_path, the actual arguments come after
    pub argv: [*mut c_char; MAX_ARGV],
    /// HACK we need to set IRQs depending on the hypervisor
    pub irq_num: usize,
    pub device_status: DeviceState,
    pub driver_status: DeviceState,
}
