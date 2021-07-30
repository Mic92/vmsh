#![no_std]
#![allow(non_camel_case_types)]

mod ffi;
mod printk;

use core::include_bytes;
use core::panic::PanicInfo;
use core::ptr;

use chlorine::{c_char, c_int, c_long, c_ulonglong, c_void, size_t};
use ffi::loff_t;

// used by our driver
const MMIO_SIZE: usize = 0x1000;
const MMIO_IRQ: usize = 5;
// chosen randomly, hopefully unused
const MMIO_DEVICE_ID: i32 = 1863406883;

const STAGE2_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stage2"));

/// Holds the device we create by this code, so we can unregister it later

const MAX_DEVICES: usize = 3;
static mut DEVICES: [Option<PlatformDevice>; MAX_DEVICES] = [None, None, None];

static STAGE2_PATH: &str = c_str!("/dev/.vmsh");

const MAX_ARGV: usize = 256;
#[repr(C)]
struct Stage1Args {
    /// physical mmio addresses
    device_addrs: [c_ulonglong; MAX_DEVICES],
    /// null terminated array
    /// the first argument is always STAGE2_PATH, the actual arguments come after
    argv: [*mut c_char; MAX_ARGV],
}

#[no_mangle]
static mut VMSH_STAGE1_ARGS: Stage1Args = Stage1Args {
    device_addrs: [0; MAX_DEVICES],
    argv: [ptr::null_mut(); MAX_ARGV],
};

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // static assertion to make sure our code never uses a panic
    extern "C" {
        #[cfg_attr(
            target_family = "unix",
            link_name = "\n\n\x1b[s\x1b[1000D\x1b[0;31m\x1b[1merror\x1b[0m\x1b[1m: the static assertion that no panics are present has failed\x1b[0m\x1b[u\n\n"
        )]
        #[cfg_attr(
            not(target_family = "unix"),
            link_name = "\n\nerror: the static assertion that no panics are present has failed\n\n"
        )]
        fn never_panic() -> !;
    }

    unsafe { never_panic() }
}

struct PlatformDevice {
    dev: *mut ffi::platform_device,
}

impl Drop for PlatformDevice {
    fn drop(&mut self) {
        unsafe { ffi::platform_device_unregister(self.dev) }
    }
}

// we put this in stack to avoid stack overflows
static mut RESOURCES: [ffi::resource; 2] = [
    ffi::resource {
        name: ptr::null(),
        flags: ffi::IORESOURCE_MEM,
        start: 0,
        end: 0,
        desc: 0,
        parent: ptr::null_mut(),
        sibling: ptr::null_mut(),
        child: ptr::null_mut(),
    },
    ffi::resource {
        name: ptr::null(),
        flags: ffi::IORESOURCE_IRQ,
        start: 0,
        end: 0,
        desc: 0,
        parent: ptr::null_mut(),
        sibling: ptr::null_mut(),
        child: ptr::null_mut(),
    },
];

// we put this in stack to avoid stack overflows
static mut INFO: ffi::platform_device_info = ffi::platform_device_info {
    parent: ptr::null_mut(),
    fwnode: ptr::null_mut(),
    of_node_reused: false,
    name: b"virtio-mmio\0".as_ptr() as *const i8,
    id: 0,
    res: unsafe { RESOURCES.as_ptr() },
    // not stable yet
    //num_res: resources.as_slice.size(),
    num_res: 2,
    data: ptr::null(),
    size_data: 0,
    dma_mask: 0,
    properties: ptr::null(),
};

unsafe fn register_virtio_mmio(
    id: c_int,
    base: usize,
    size: usize,
    irq: usize,
) -> Result<PlatformDevice, c_int> {
    // we need to use static here to no got out of stack memory
    RESOURCES[0].start = base;
    RESOURCES[0].end = base + size - 1;
    RESOURCES[1].start = irq;
    RESOURCES[1].end = irq;
    INFO.id = id;

    let dev = ffi::platform_device_register_full(&INFO);
    if is_err_value(dev) {
        return Err(err_value(dev) as c_int);
    }
    Ok(PlatformDevice { dev })
}

/// re-implementation of IS_ERR_VALUE
fn is_err_value(x: *const c_void) -> bool {
    x as c_long >= -(ffi::MAX_ERRNO as c_long)
}

/// Retrieves error value from pointer
fn err_value(ptr: *const c_void) -> c_long {
    ptr as c_long
}

struct KFile {
    file: *mut ffi::file,
}

impl KFile {
    fn open(name: &str, flags: c_int, mode: ffi::umode_t) -> core::result::Result<KFile, c_int> {
        let file = unsafe { ffi::filp_open(name.as_ptr() as *const c_char, flags, mode) };
        if is_err_value(file) {
            return Err(err_value(file) as c_int);
        }
        Ok(KFile { file })
    }

    fn write_all(&mut self, data: &[u8], pos: loff_t) -> core::result::Result<size_t, c_int> {
        let mut out: size_t = 0;
        let mut count = data.len();
        let mut p = data.as_ptr();

        /* sys_write only can write MAX_RW_COUNT aka 2G-4K bytes at most */
        while count != 0 {
            let rv = unsafe { ffi::kernel_write(self.file, p as *const c_void, count, pos) };

            match -rv as c_int {
                0 => break,
                ffi::EINTR | ffi::EAGAIN => continue,
                1..=c_int::MAX => return if out == 0 { Err(-rv as c_int) } else { Ok(out) },
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
        let res = unsafe { ffi::filp_close(self.file, ptr::null_mut()) };
        if res != 0 {
            printkln!("stage1: error closing file: %d", res)
        }
    }
}

unsafe extern "C" fn spawn_stage2(_arg: *mut c_void) -> c_int {
    for (i, addr) in VMSH_STAGE1_ARGS.device_addrs.iter().enumerate() {
        if *addr == 0 {
            continue;
        }
        printkln!("stage1: init dev at 0x%llx", *addr);
        match register_virtio_mmio(
            MMIO_DEVICE_ID + (i as i32),
            *addr as usize,
            MMIO_SIZE,
            MMIO_IRQ,
        ) {
            Ok(v) => {
                if let Some(elem) = DEVICES.get_mut(i) {
                    *elem = Some(v);
                } else {
                    printkln!("stage1: out-of-bound write to devs");
                    return -ffi::EFAULT;
                }
            }
            Err(res) => {
                printkln!("stage1: failed to register block mmio device: %d", res);
                return res;
            }
        };
    }

    // we never delete this file, however deleting files is complex and requires accessing
    // internal structs that might change.
    let mut file = match KFile::open(STAGE2_PATH, ffi::O_WRONLY | ffi::O_CREAT, 0o755) {
        Ok(f) => f,
        Err(e) => {
            printkln!("stage1: cannot open /dev/.vmsh: %d", e);
            return e;
        }
    };
    match file.write_all(STAGE2_EXE, 0) {
        Ok(n) => {
            if n != STAGE2_EXE.len() {
                printkln!(
                    "/dev/.vmsh: incomplete write (%zu != %zu)",
                    n,
                    STAGE2_EXE.len()
                );
                return -ffi::EIO;
            }
        }
        Err(res) => {
            printkln!("stage1: cannot write /dev/.vmsh: %d", res);
            return res;
        }
    }
    drop(file);
    ffi::flush_delayed_fput();

    let mut envp: [*mut c_char; 1] = [ptr::null_mut()];

    let res = ffi::call_usermodehelper(
        STAGE2_PATH.as_ptr() as *mut c_char,
        VMSH_STAGE1_ARGS.argv.as_mut_ptr(),
        envp.as_mut_ptr(),
        ffi::UMH_WAIT_EXEC,
    );
    if res != 0 {
        printkln!("stage1: failed to spawn stage2: %d", res);
    }
    res
}

/// # Safety
///
/// this code is not thread-safe as it uses static globals
#[no_mangle]
unsafe fn init_vmsh_stage1() -> c_int {
    printkln!("stage1: init");
    VMSH_STAGE1_ARGS.argv[0] = STAGE2_PATH.as_ptr() as *mut c_char;

    // We cannot close a file synchronusly outside of a kthread
    // Within a kthread we can use `flush_delayed_fput`
    let thread = ffi::kthread_create_on_node(
        spawn_stage2,
        ptr::null_mut(),
        0,
        c_str!("vmsh-stage1").as_ptr() as *const c_char,
    );
    if is_err_value(thread) {
        printkln!(
            "stage1: failed to spawn kernel thread: %d",
            err_value(thread)
        );
        return err_value(thread) as c_int;
    }
    ffi::wake_up_process(thread);

    0
}

/// # Safety
///
/// this code is not thread-safe as it uses static globals
#[no_mangle]
pub unsafe fn cleanup_vmsh_stage1() {
    printkln!("stage1: cleanup");
    DEVICES.iter_mut().for_each(|d| {
        d.take();
    });
}
