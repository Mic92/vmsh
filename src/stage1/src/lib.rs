#![no_std]
#![allow(non_camel_case_types)]

mod printk;

use core::panic::PanicInfo;
use core::ptr;

// used by our driver
const MMIO_BASE: usize = 0xd0000000;
const MMIO_SIZE: usize = 0x1000;
const MMIO_IRQ: usize = 5;

// kernel constants and definition
const IORESOURCE_MEM: libc::c_ulong = 0x00000200;
const IORESOURCE_IRQ: libc::c_ulong = 0x00000400;
const MAX_ERRNO: libc::c_ulong = 4095;

// kernel structures
type phys_addr_t = usize;
type resource_size_t = phys_addr_t;

// We omit some kernel structs here, that we don't need
type device = libc::c_void;
type fwnode_handle = libc::c_void;
type platform_device = libc::c_void;
type property_entry = libc::c_void;

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

// from the linux kernel, see `struct resource`
#[repr(C)]
pub struct resource {
    start: resource_size_t,
    end: resource_size_t,
    name: *const libc::c_char,
    flags: libc::c_ulong,
    desc: libc::c_ulong,
    parent: *mut resource,
    sibling: *mut resource,
    child: *mut resource,
}

// from the linux kernel, see `struct platform_device_info`
#[repr(C)]
pub struct platform_device_info {
    parent: *mut device,
    fwnode: *mut fwnode_handle,
    of_node_reused: bool,

    name: *const libc::c_char,
    id: libc::c_int,

    res: *const resource,
    num_res: libc::c_uint,

    data: *const libc::c_void,
    size_data: libc::size_t,
    dma_mask: u64,

    properties: *const property_entry,
}

extern "C" {
    pub fn platform_device_register_full(
        pdevinfo: *const platform_device_info,
    ) -> *mut platform_device;
    pub fn platform_device_unregister(pdev: *mut platform_device);

}

unsafe fn register_virtio_mmio(base: usize, size: usize, irq: usize) -> *mut platform_device {
    let resources: [resource; 2] = [
        resource {
            name: ptr::null(),
            flags: IORESOURCE_MEM,
            start: base,
            end: base + size - 1,
            desc: 0,
            parent: ptr::null_mut(),
            sibling: ptr::null_mut(),
            child: ptr::null_mut(),
        },
        resource {
            name: ptr::null(),
            flags: IORESOURCE_IRQ,
            start: irq,
            end: irq,
            desc: 0,
            parent: ptr::null_mut(),
            sibling: ptr::null_mut(),
            child: ptr::null_mut(),
        },
    ];

    let info = platform_device_info {
        parent: ptr::null_mut(),
        fwnode: ptr::null_mut(),
        of_node_reused: false,
        name: b"virtio-mmio\0".as_ptr() as *const i8,
        id: 0,
        res: resources.as_ptr(),
        // not stable yet
        //num_res: resources.as_slice.size(),
        num_res: 2,
        data: ptr::null(),
        size_data: 0,
        dma_mask: 0,
        properties: ptr::null(),
    };

    return platform_device_register_full(&info);
}

/// Holds the device we create by this code, so we can unregister it later
static mut BLK_DEV: *mut platform_device = ptr::null_mut();

/// re-implementation of IS_ERR_VALUE
fn is_err_value(x: *const libc::c_void) -> bool {
    return x as libc::c_long >= -(MAX_ERRNO as libc::c_long);
}

/// Retrieves error value from pointer
fn err_value(ptr: *const libc::c_void) -> libc::c_long {
    ptr as libc::c_long
}

/// Safety: this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn init_vmsh_stage1() -> libc::c_int {
    printkln!("stage1: init");
    BLK_DEV = register_virtio_mmio(MMIO_BASE, MMIO_SIZE, MMIO_IRQ);
    if is_err_value(BLK_DEV) {
        printkln!(
            "stage1: initializing virt-blk driver failed: {}",
            err_value(BLK_DEV)
        );
        return err_value(BLK_DEV) as libc::c_int;
    }
    printkln!("stage1: virt-blk driver set up");
    return 0;
}

/// Safety: this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn cleanup_vmsh_stage1() {
    platform_device_unregister(BLK_DEV);
}
