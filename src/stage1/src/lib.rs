#![no_std]
#![allow(non_camel_case_types)]

mod printk;

use core::include_bytes;
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
const UMH_WAIT_EXEC: libc::c_int = 1;

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

const STAGE2_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stage2"));

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

struct PlatformDevice {
    dev: *mut platform_device,
}

impl Drop for PlatformDevice {
    fn drop(&mut self) {
        unsafe { platform_device_unregister(self.dev) }
    }
}

extern "C" {
    pub fn platform_device_register_full(
        pdevinfo: *const platform_device_info,
    ) -> *mut platform_device;
    pub fn platform_device_unregister(pdev: *mut platform_device);
    pub fn memcpy(dest: *mut libc::c_void, src: *const libc::c_void, count: libc::size_t);

    pub fn filp_open(name: *const libc::c_char, flags: libc::c_int, mode: umode_t) -> *mut file;
    pub fn fput(file: *mut file);
    pub fn kernel_write(
        file: *mut file,
        buf: *const libc::c_void,
        count: libc::size_t,
        pos: libc::loff_t,
    ) -> libc::ssize_t;

    pub fn call_usermodehelper(
        path: *const libc::c_char,
        argv: *mut *mut libc::c_char,
        envp: *mut *mut libc::c_char,
        wait: libc::c_int,
    ) -> libc::c_int;
}

unsafe fn register_virtio_mmio(
    base: usize,
    size: usize,
    irq: usize,
) -> Result<PlatformDevice, libc::c_int> {
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

    let dev = platform_device_register_full(&info);
    if is_err_value(dev) {
        return Err(err_value(dev) as libc::c_int);
    }
    Ok(PlatformDevice { dev })
}

/// Holds the device we create by this code, so we can unregister it later
static mut BLK_DEV: Option<PlatformDevice> = None;

/// re-implementation of IS_ERR_VALUE
fn is_err_value(x: *const libc::c_void) -> bool {
    x as libc::c_long >= -(MAX_ERRNO as libc::c_long)
}

/// Retrieves error value from pointer
fn err_value(ptr: *const libc::c_void) -> libc::c_long {
    ptr as libc::c_long
}

struct KFile {
    file: *mut file,
}

impl KFile {
    fn open(
        name: &str,
        flags: libc::c_int,
        mode: umode_t,
    ) -> core::result::Result<KFile, libc::c_int> {
        let file = unsafe { filp_open(name.as_ptr() as *const libc::c_char, flags, mode) };
        if is_err_value(file) {
            return Err(err_value(file) as libc::c_int);
        }
        Ok(KFile { file })
    }

    fn write_all(
        &mut self,
        data: &[u8],
        pos: libc::loff_t,
    ) -> core::result::Result<libc::size_t, libc::c_int> {
        let mut out: libc::size_t = 0;
        let mut count = data.len();
        let mut p = data.as_ptr();

        /* sys_write only can write MAX_RW_COUNT aka 2G-4K bytes at most */
        while count != 0 {
            let rv = unsafe { kernel_write(self.file, p as *const libc::c_void, count, pos) };

            match -rv as libc::c_int {
                0 => break,
                libc::EINTR | libc::EAGAIN => continue,
                1..=libc::c_int::MAX => {
                    return if out == 0 {
                        Err(-rv as libc::c_int)
                    } else {
                        Ok(out)
                    }
                }
                _ => {}
            }

            p = unsafe { p.add(rv as usize) };
            out += rv as usize;
            count -= rv as usize;
        }

        Ok(out)
    }
}

impl Drop for KFile {
    fn drop(&mut self) {
        unsafe { fput(self.file) };
    }
}

const STAGE2_PATH: &str = c_str!("/dev/.vmsh");

/// # Safety
///
/// this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn init_vmsh_stage1() -> libc::c_int {
    printkln!("stage1: init");
    // we never delete this file, however deleting files is complex and requires accessing
    // internal structs that might change.
    let mut file = match KFile::open(STAGE2_PATH, libc::O_WRONLY | libc::O_CREAT, 0o755) {
        Ok(f) => f,
        Err(e) => {
            printkln!("stage1: cannot open /dev/.vmsh: {}", e);
            return e;
        }
    };
    if let Err(res) = file.write_all(STAGE2_EXE, 0) {
        printkln!("stage1: cannot write /dev/.vmsh: {}", res);
        return res;
    }

    let dev = match register_virtio_mmio(MMIO_BASE, MMIO_SIZE, MMIO_IRQ) {
        Ok(v) => Some(v),
        Err(res) => {
            printkln!("stage1: failed to register mmio device: {}", res);
            return res;
        }
    };
    printkln!("stage1: virt-blk driver set up");

    let envp: *mut *mut libc::c_char = { ptr::null_mut() };
    let argv: *mut *mut libc::c_char = {
        STAGE2_EXE.as_ptr();
        ptr::null_mut()
    };

    let err = call_usermodehelper(
        STAGE2_EXE.as_ptr() as *const libc::c_char,
        argv,
        envp,
        UMH_WAIT_EXEC,
    );

    if err != 0 {
        printkln!("stage1: failed to spawn stage2: {}", err);
    }

    printkln!("stage1: spawned stage2");
    BLK_DEV = dev;
    0
}

/// # Safety
///
/// this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn cleanup_vmsh_stage1() {
    printkln!("stage1: cleanup");
    BLK_DEV.take();
}
