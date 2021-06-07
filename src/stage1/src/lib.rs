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
type umode_t = libc::c_ushort;

// We omit some kernel structs here, that we don't need
type device = libc::c_void;
type fwnode_handle = libc::c_void;
type platform_device = libc::c_void;
type property_entry = libc::c_void;
/// same as struct file
type file = libc::c_void;
/// same as struct kobject
type kobject = libc::c_void;
type address_space = libc::c_void;

#[repr(C)]
pub struct lock_class_key {
    a: libc::c_uint,
}

/// from <linux/sysfs.h>
#[repr(C)]
pub struct attribute {
    name: *const libc::c_char,
    mode: umode_t,
    /// XXX we skip initializing some fields here present if DEBUG_LOCK_ALLOC is enabled
    /// We assume that most general purpose kernels won't have it
    ignore_lockdep: bool,
    key: *mut lock_class_key,
    skey: lock_class_key,
}

/// from <linux/sysfs.h>
/// XXX this struct could change (i.e. become larger over time)
#[repr(C)]
pub struct bin_attribute {
    attr: attribute,
    size: libc::size_t,
    private: *mut libc::c_void,
    address_space: *mut address_space,
    read: unsafe extern "C" fn(
        *mut file,
        *mut kobject,
        *mut bin_attribute,
        *mut libc::c_char,
        libc::loff_t,
        libc::size_t,
    ) -> libc::ssize_t,
    // actually a function pointer
    write: *mut libc::c_void,
    // actually a function pointer
    mmap: *mut libc::c_void,
}

use core::cmp;
use core::include_bytes;

const STAGE2_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stage2"));

unsafe extern "C" fn read_stage2_binary(
    _: *mut file,
    _: *mut kobject,
    _: *mut bin_attribute,
    buf: *mut libc::c_char,
    off: libc::loff_t,
    size: libc::size_t,
) -> libc::ssize_t {
    let read = cmp::min(
        size,
        STAGE2_EXE.len() - cmp::min(off as usize, STAGE2_EXE.len()),
    );
    memcpy(
        buf as *mut libc::c_void,
        (STAGE2_EXE.as_ptr() as *mut libc::c_void).add(off as usize),
        read,
    );
    read as libc::ssize_t
}

const VMSH_BINARY_ATTR: bin_attribute = bin_attribute {
    attr: attribute {
        name: c_str!("vmsh").as_ptr() as *const libc::c_char,
        mode: 0o755,
        ignore_lockdep: false,
        key: ptr::null_mut(),
        skey: lock_class_key { a: 0 },
    },
    private: ptr::null_mut(),
    address_space: ptr::null_mut(),
    /// TODO
    size: 0,
    mmap: ptr::null_mut(),
    write: ptr::null_mut(),
    read: read_stage2_binary,
};

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
    pub fn memcpy(dest: *mut libc::c_void, src: *const libc::c_void, count: libc::size_t);

    pub fn sysfs_create_bin_file(kobj: *mut kobject, attr: *const bin_attribute) -> libc::c_int;
    pub fn sysfs_remove_bin_file(kobj: *mut kobject, attr: *const bin_attribute);

    static kernel_kobj: *mut kobject;
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

    platform_device_register_full(&info)
}

/// Holds the device we create by this code, so we can unregister it later
static mut BLK_DEV: *mut platform_device = ptr::null_mut();

/// re-implementation of IS_ERR_VALUE
fn is_err_value(x: *const libc::c_void) -> bool {
    x as libc::c_long >= -(MAX_ERRNO as libc::c_long)
}

/// Retrieves error value from pointer
fn err_value(ptr: *const libc::c_void) -> libc::c_long {
    ptr as libc::c_long
}

/// # Safety
///
/// this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn init_vmsh_stage1() -> libc::c_int {
    printkln!("stage1: init");

    let ret = sysfs_create_bin_file(kernel_kobj, &VMSH_BINARY_ATTR);
    if ret != 0 {
        printkln!("stage1: could not register sysfs entry for vmsh: {}", ret);
        return ret as libc::c_int;
    }

    BLK_DEV = register_virtio_mmio(MMIO_BASE, MMIO_SIZE, MMIO_IRQ);
    if is_err_value(BLK_DEV) {
        printkln!(
            "stage1: initializing virt-blk driver failed: {}",
            err_value(BLK_DEV)
        );
        sysfs_remove_bin_file(kernel_kobj, &VMSH_BINARY_ATTR);
        return err_value(BLK_DEV) as libc::c_int;
    }
    printkln!("stage1: virt-blk driver set up");
    0
}

/// # Safety
///
/// this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn cleanup_vmsh_stage1() {
    printkln!("stage1: cleanup");
    platform_device_unregister(BLK_DEV);
    sysfs_remove_bin_file(kernel_kobj, &VMSH_BINARY_ATTR);
}
