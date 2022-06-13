#![allow(dead_code)]

use chlorine::{c_char, c_int, c_long, c_longlong, c_uint, c_ulong, c_ushort, c_void, size_t};

// kernel constants and definition
pub const IORESOURCE_MEM: c_ulong = 0x00000200;
pub const IORESOURCE_IRQ: c_ulong = 0x00000400;
pub const MAX_ERRNO: c_ulong = 4095;
pub const UMH_WAIT_EXEC: c_int = 1;
pub const TASK_UNINTERRUPTIBLE: c_uint = 0x0002;

// errno.h
pub const EPERM: c_int = 1;
pub const ENOENT: c_int = 2;
pub const ESRCH: c_int = 3;
pub const EINTR: c_int = 4;
pub const EIO: c_int = 5;
pub const ENXIO: c_int = 6;
pub const E2BIG: c_int = 7;
pub const ENOEXEC: c_int = 8;
pub const EBADF: c_int = 9;
pub const ECHILD: c_int = 10;
pub const EAGAIN: c_int = 11;
pub const ENOMEM: c_int = 12;
pub const EACCES: c_int = 13;
pub const EFAULT: c_int = 14;
pub const ENOTBLK: c_int = 15;
pub const EBUSY: c_int = 16;
pub const EEXIST: c_int = 17;
pub const EXDEV: c_int = 18;
pub const ENODEV: c_int = 19;
pub const ENOTDIR: c_int = 20;
pub const EISDIR: c_int = 21;
pub const EINVAL: c_int = 22;
pub const ENFILE: c_int = 23;
pub const EMFILE: c_int = 24;
pub const ENOTTY: c_int = 25;
pub const ETXTBSY: c_int = 26;
pub const EFBIG: c_int = 27;
pub const ENOSPC: c_int = 28;
pub const ESPIPE: c_int = 29;
pub const EROFS: c_int = 30;
pub const EMLINK: c_int = 31;
pub const EPIPE: c_int = 32;
pub const EDOM: c_int = 33;
pub const ERANGE: c_int = 34;
pub const EWOULDBLOCK: c_int = EAGAIN;

// open flags
pub const O_RDONLY: c_int = 0;
pub const O_WRONLY: c_int = 1;
pub const O_RDWR: c_int = 2;
pub const O_CREAT: c_int = 64;

// kernel structures
pub type phys_addr_t = usize;
pub type resource_size_t = phys_addr_t;
pub type umode_t = c_ushort;
pub type loff_t = c_longlong;
pub type ssize_t = isize;

// We omit some kernel structs here, that we don't need
pub type device = c_void;
pub type fwnode_handle = c_void;
pub type platform_device = c_void;
pub type property_entry = c_void;
/// same as struct file
pub type file = c_void;
pub type task_struct = c_void;

// from the linux kernel, see `struct resource`
#[repr(C)]
pub struct resource {
    pub start: resource_size_t,
    pub end: resource_size_t,
    pub name: *const c_char,
    pub flags: c_ulong,
    pub desc: c_ulong,
    pub parent: *mut resource,
    pub sibling: *mut resource,
    pub child: *mut resource,
}

// from the linux kernel, see `struct platform_device_info`
#[repr(C)]
pub struct platform_device_info {
    pub parent: *mut device,
    pub fwnode: *mut fwnode_handle,
    pub of_node_reused: bool,

    pub name: *const c_char,
    pub id: c_int,

    pub res: *const resource,
    pub num_res: c_uint,

    pub data: *const c_void,
    pub size_data: size_t,
    pub dma_mask: u64,

    pub properties: *const property_entry,
}

#[repr(C)]
pub struct platform_device_info_5_0 {
    pub parent: *mut device,
    pub fwnode: *mut fwnode_handle,

    pub name: *const c_char,
    pub id: c_int,

    pub res: *const resource,
    pub num_res: c_uint,

    pub data: *const c_void,
    pub size_data: size_t,
    pub dma_mask: u64,

    pub properties: *const property_entry,
}

type atomic_long_t = c_long;

#[repr(C)]
pub struct list_head {
    pub next: *mut list_head,
    pub prev: *mut list_head,
}

type work_func_t = unsafe extern "C" fn(work: *mut work_struct);

#[repr(C)]
pub struct work_struct {
    pub data: atomic_long_t,
    pub entry: list_head,
    pub func: work_func_t,
    // in case this function grows in future
    pub padding: [u8; 100],
}

#[repr(C)]
pub struct execute_work {
    pub work: work_struct,
}

pub type workqueue_struct = c_void;

pub unsafe fn kernel_read_4_13(
    file: *mut file,
    pos: loff_t,
    buf: *mut c_char,
    count: c_ulong,
) -> c_int {
    kernel_read(
        file,
        pos as *mut c_void,
        buf as size_t,
        count as *mut loff_t,
    ) as c_int
}

pub unsafe fn kernel_write_4_13(
    file: *mut file,
    buf: *const c_char,
    count: size_t,
    pos: loff_t,
) -> ssize_t {
    kernel_write(
        file,
        buf as *const c_void,
        count as size_t,
        pos as *mut loff_t,
    ) as ssize_t
}

extern "C" {
    pub fn platform_device_register_full(
        pdevinfo: *const platform_device_info,
    ) -> *mut platform_device;
    pub fn platform_device_unregister(pdev: *mut platform_device);
    pub fn filp_open(name: *const c_char, flags: c_int, mode: umode_t) -> *mut file;
    pub fn filp_close(filp: *mut file, id: *mut c_void) -> c_int;
    pub fn kernel_write(
        file: *mut file,
        buf: *const c_void,
        count: size_t,
        pos: *mut loff_t,
    ) -> ssize_t;
    pub fn kernel_read(
        file: *mut file,
        buf: *mut c_void,
        count: size_t,
        pos: *mut loff_t,
    ) -> ssize_t;

    pub fn call_usermodehelper(
        path: *const c_char,
        argv: *mut *mut c_char,
        envp: *mut *mut c_char,
        wait: c_int,
    ) -> c_int;
    pub fn queue_work_on(cpu: c_int, wq: *mut workqueue_struct, work: *mut work_struct) -> bool;
    pub fn usleep_range_state(min: c_ulong, max: c_ulong, state: c_uint);

    pub fn kthread_create_on_node(
        threadfn: unsafe extern "C" fn(data: *mut c_void) -> c_int,
        data: *mut c_void,
        node: c_int,
        namefmt: *const c_char,
        ...
    ) -> *mut task_struct;

    pub fn wake_up_process(p: *mut task_struct);
    pub fn __symbol_get(symbol: *const c_char) -> *mut c_void;
}
