mod device;
mod inorder_handler;
mod queue_handler;

use std::io;

use event_manager::Error as EvmgrError;
use vm_device::bus;
use vmm_sys_util::errno;

use crate::devices::virtio::CommonArgs;
use simple_error::SimpleError;

pub use device::Console;

// Console device ID as defined by the standard.
pub const CONSOLE_DEVICE_ID: u32 = 3;

#[derive(Debug)]
pub enum Error {
    AlreadyActivated,
    BadFeatures(u64),
    Bus(bus::Error),
    Endpoint(EvmgrError),
    EventFd(io::Error),
    #[allow(dead_code)] // FIXME
    QueuesNotValid,
    #[allow(dead_code)] // FIXME
    RegisterIoevent(errno::Error),
    #[allow(dead_code)] // FIXME
    RegisterIrqfd(errno::Error),
    Simple(SimpleError),
}

pub type Result<T> = std::result::Result<T, Error>;

#[repr(C,packed)]
struct virtio_console_config {
    /* colums of the screens */
    cols: u16,
    /* rows of the screens */
    rows: u16,
    /* max. number of ports this device can hold */
    max_nr_ports: u32,
    /* emergency write register */
    emerg_wr: u32,
}

unsafe fn any_as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    ::std::slice::from_raw_parts(
        (p as *const T) as *const u8,
        ::std::mem::size_of::<T>(),
    )
}

fn build_config_space() -> Vec<u8> {
    let config = virtio_console_config {
        cols: 80,
        rows: 24,
        max_nr_ports: 2,
        emerg_wr: 0
    };
    unsafe { any_as_u8_slice(&config) }.to_vec()
}

// Arguments required when building a console device.
pub struct ConsoleArgs<'a, M, B> {
    pub common: CommonArgs<'a, M, B>,
}