#![no_std]
#![allow(non_camel_case_types)]

mod ffi;
mod printk;

use core::include_bytes;
use core::panic::PanicInfo;
use core::ptr;
use core::str;
use stage1_interface::{DeviceState, Stage1Args, IRQ_NUM, MAX_ARGV, MAX_DEVICES};

use chlorine::{c_char, c_int, c_long, c_uint, c_void, size_t};
use ffi::loff_t;

// used by our driver
const MMIO_SIZE: usize = 0x1000;
// chosen randomly, hopefully unused
const MMIO_DEVICE_ID: i32 = 1863406883;

const STAGE2_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stage2"));

#[no_mangle]
static mut VMSH_STAGE1_ARGS: Stage1Args = Stage1Args {
    device_addrs: [0; MAX_DEVICES],
    argv: [ptr::null_mut(); MAX_ARGV],
    device_status: DeviceState::Undefined,
    driver_status: DeviceState::Undefined,
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

static MMIO_DRIVER_NAME: &[u8; 12] = b"virtio-mmio\0";

// we put this in stack to avoid stack overflows
static mut INFO: ffi::platform_device_info = ffi::platform_device_info {
    parent: ptr::null_mut(),
    fwnode: ptr::null_mut(),
    of_node_reused: false,
    name: MMIO_DRIVER_NAME.as_ptr() as *const i8,
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

static mut INFO_5_0: ffi::platform_device_info_5_0 = ffi::platform_device_info_5_0 {
    parent: ptr::null_mut(),
    fwnode: ptr::null_mut(),
    name: MMIO_DRIVER_NAME.as_ptr() as *const i8,
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
    version: &KernelVersion,
) -> Result<PlatformDevice, c_int> {
    // we need to use static here to no got out of stack memory
    RESOURCES[0].start = base;
    RESOURCES[0].end = base + size - 1;
    RESOURCES[1].start = irq;
    RESOURCES[1].end = irq;

    let dev = if version.major < 5 || version.major == 5 && version.minor == 0 {
        INFO_5_0.id = id;
        let info = &*core::mem::transmute::<_, *const ffi::platform_device_info>(&INFO_5_0);
        ffi::platform_device_register_full(info)
    } else {
        INFO.id = id;
        ffi::platform_device_register_full(&INFO)
    };
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
    fn open(
        name: *const c_char,
        flags: c_int,
        mode: ffi::umode_t,
    ) -> core::result::Result<KFile, c_int> {
        let file = unsafe { ffi::filp_open(name, flags, mode) };
        if is_err_value(file) {
            return Err(err_value(file) as c_int);
        }
        Ok(KFile { file })
    }

    fn read_all(&mut self, data: &mut [u8], pos: loff_t) -> core::result::Result<size_t, c_int> {
        let mut out: size_t = 0;
        let mut count = data.len();
        let mut p = data.as_ptr();
        let mut lpos = pos;
        loop {
            let rv = unsafe { ffi::kernel_read(self.file, p as *mut c_void, count, &mut lpos) };
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

    fn write_all(&mut self, data: &[u8], pos: loff_t) -> core::result::Result<size_t, c_int> {
        let mut out: size_t = 0;
        let mut count = data.len();
        let mut p = data.as_ptr();
        let mut lpos = pos;

        /* sys_write only can write MAX_RW_COUNT aka 2G-4K bytes at most */
        while count != 0 {
            let rv = unsafe { ffi::kernel_write(self.file, p as *const c_void, count, &mut lpos) };

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

struct KernelVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

static mut BUF: [u8; 256] = [0; 256];

fn parse_version_part(split: Option<&[u8]>, full_version: &str) -> Result<u16, ()> {
    if let Some(number) = split {
        if let Ok(num_str) = str::from_utf8(number) {
            if let Ok(num) = num_str.parse::<u16>() {
                return Ok(num);
            }
        }
    }
    printkln!("stage1: cannot parse version %s", full_version.as_ptr());
    Err(())
}

unsafe fn get_kernel_version() -> Result<KernelVersion, ()> {
    let path = c_str!("/proc/sys/kernel/osrelease").as_ptr() as *const i8;
    let mut file = match KFile::open(path, ffi::O_RDONLY, 0) {
        Ok(f) => f,
        Err(e) => {
            printkln!(
                "stage1: warning: cannot open /proc/sys/kernel/osrelease: %d",
                e
            );
            // procfs not mounted? -> assume newer kernel
            return Ok(KernelVersion {
                major: 5,
                minor: 5,
                patch: 0,
            });
        }
    };
    // leave one byte null to make it a valid c string
    let buf = match BUF.get_mut(0..BUF.len() - 1) {
        Some(f) => f,
        None => {
            printkln!("stage1: BUG! cannot take slice of buffer");
            return Err(());
        }
    };
    let count = match file.read_all(buf, 0) {
        Ok(n) => n,
        Err(e) => {
            printkln!("stage1: failed to read /proc/sys/kernel/osrelease: %d", e);
            return Err(());
        }
    };
    let read_buf = match BUF.get(0..count) {
        Some(f) => f,
        None => {
            printkln!("stage1: kernel overflowed read buffer");
            return Err(());
        }
    };
    let version_str = match str::from_utf8(read_buf) {
        Ok(s) => s,
        Err(_) => {
            printkln!("stage1: /proc/sys/kernel/osrelease is not a valid utf-8 string");
            return Err(());
        }
    };

    let pos = version_str.find(|c: char| c != '.' && !c.is_ascii_digit());
    let kernel = if let Some(pos) = pos {
        match version_str.get(..pos) {
            Some(s) => s,
            None => {
                printkln!("stage1: could split off kernel version part");
                return Err(());
            }
        }
    } else {
        version_str
    };
    let mut split = kernel.as_bytes().splitn(3, |c| *c == b'.');
    let major = parse_version_part(split.next(), version_str)?;
    let minor = parse_version_part(split.next(), version_str)?;
    let patch = parse_version_part(split.next(), version_str)?;
    let v = KernelVersion {
        major,
        minor,
        patch,
    };
    printkln!(
        "stage1: detected linux version %u.%u.%u",
        v.major as c_uint,
        v.minor as c_uint,
        v.patch as c_uint
    );

    Ok(v)
}

// cannot put this onto the stack without stackoverflows?
static mut DEVICES: [Option<PlatformDevice>; MAX_DEVICES] = [None, None, None];

unsafe fn run_stage2() -> Result<(), ()> {
    let version = get_kernel_version()?;

    for (i, addr) in VMSH_STAGE1_ARGS.device_addrs.iter().enumerate() {
        if *addr == 0 {
            continue;
        }
        printkln!("stage1: init dev at 0x%llx", *addr);
        match register_virtio_mmio(
            MMIO_DEVICE_ID + (i as i32),
            *addr as usize,
            MMIO_SIZE,
            IRQ_NUM, //irq as usize,
            &version,
        ) {
            Ok(v) => {
                if let Some(elem) = DEVICES.get_mut(i) {
                    *elem = Some(v);
                } else {
                    printkln!("stage1: out-of-bound write to devs");
                    return Err(());
                }
            }
            Err(res) => {
                printkln!(
                    "stage1: failed to register block mmio device: errno=%d",
                    res
                );
                return Err(());
            }
        };
    }

    // we never delete this file, however deleting files is complex and requires accessing
    // internal structs that might change.
    let mut file = match KFile::open(
        VMSH_STAGE1_ARGS.argv[0],
        ffi::O_WRONLY | ffi::O_CREAT,
        0o755,
    ) {
        Ok(f) => f,
        Err(e) => {
            if e == -ffi::ENOENT {
                // /dev/ does not exists, let's try /
                match KFile::open(
                    c_str!("/.vmsh").as_ptr() as *const c_char,
                    ffi::O_WRONLY | ffi::O_CREAT,
                    0o755,
                ) {
                    Ok(f) => {
                        VMSH_STAGE1_ARGS.argv[0] = c_str!("/.vmsh").as_ptr() as *mut c_char;
                        f
                    }
                    Err(e) => {
                        printkln!("stage1: cannot open /.vmsh: errno=%d", e);
                        return Err(());
                    }
                }
            } else {
                printkln!(
                    "stage1: cannot open %s: errno=%d",
                    VMSH_STAGE1_ARGS.argv[0],
                    e
                );
                return Err(());
            }
        }
    };
    match file.write_all(STAGE2_EXE, 0) {
        Ok(n) => {
            if n != STAGE2_EXE.len() {
                printkln!(
                    "%s: incomplete write (%zu != %zu)",
                    VMSH_STAGE1_ARGS.argv[0],
                    n,
                    STAGE2_EXE.len()
                );
                return Err(());
            }
        }
        Err(res) => {
            printkln!(
                "stage1: cannot write %s: errno=%d",
                VMSH_STAGE1_ARGS.argv[0],
                res
            );
            return Err(());
        }
    }
    drop(file);

    let mut envp: [*mut c_char; 1] = [ptr::null_mut()];

    loop {
        let res = ffi::call_usermodehelper(
            VMSH_STAGE1_ARGS.argv[0],
            VMSH_STAGE1_ARGS.argv.as_mut_ptr(),
            envp.as_mut_ptr(),
            ffi::UMH_WAIT_EXEC,
        );
        if res == -ffi::ETXTBSY {
            // Ideally we could use flush_delayed_fput to close the binary but not
            // all kernel versions support this.
            // Hence we just sleep until the file is closed.
            ffi::usleep_range(10 * 1000, 100 * 1000);
            continue;
        }
        if res != 0 {
            printkln!("stage1: failed to spawn stage2: errno=%d", res);
            return Err(());
        }
        return Ok(());
    }
}

unsafe extern "C" fn spawn_stage2() {
    //for (i, a) in VMSH_STAGE1_ARGS.argv.iter().enumerate() {
    //    if *a == ptr::null_mut() {
    //        break;
    //    }
    //    printkln!("stage1: argv[%d] = %s", i, *a)
    //}
    if VMSH_STAGE1_ARGS.device_status == DeviceState::Undefined {
        printkln!("stage1: device is in undefined state, stopping...");
        return;
    }
    let mut retries = 0;
    while VMSH_STAGE1_ARGS.device_status == DeviceState::Initializing {
        printkln!(
            "current value: %d, %llx",
            VMSH_STAGE1_ARGS.device_status,
            &VMSH_STAGE1_ARGS.device_status
        );
        ffi::usleep_range(10 * 1000, 100 * 1000);
        retries += 1;
        if retries == 20 {
            printkln!("stage1: timeout waiting for device to be initialized");
            VMSH_STAGE1_ARGS.driver_status = DeviceState::Error;
            return;
        }
    }
    VMSH_STAGE1_ARGS.driver_status = DeviceState::Initializing;
    if VMSH_STAGE1_ARGS.device_status == DeviceState::Error {
        printkln!("stage1: device error detected, stopping...");
        return;
    }
    printkln!("stage1: initializing drivers");
    let res = run_stage2();
    if res.is_ok() {
        printkln!("stage1: ready");
        VMSH_STAGE1_ARGS.driver_status = DeviceState::Ready;
    } else {
        printkln!("stage1: failed");
        DEVICES.iter_mut().for_each(|d| {
            d.take();
        });
        VMSH_STAGE1_ARGS.driver_status = DeviceState::Error;
        return;
    };

    while VMSH_STAGE1_ARGS.device_status == DeviceState::Ready {
        ffi::usleep_range(50 * 1000, 500 * 1000);
    }

    DEVICES.iter_mut().for_each(|d| {
        d.take();
    });
    VMSH_STAGE1_ARGS.driver_status = DeviceState::Terminating;
}
extern "C" {
    #[link(name = "trampoline", kind = "static")]
    pub fn _init_vmsh();
}

#[no_mangle]
unsafe fn linker_hack() {
    // force linker to include _init_vmsh() symbol
    _init_vmsh();
}

extern "C" fn stage2_worker(_work: *mut ffi::work_struct) {
    printkln!("stage1: spawn stage2");
    unsafe { spawn_stage2() };
    printkln!("stage1: finished");
}

static mut THREAD_SPAWN_WORK: ffi::work_struct = ffi::work_struct {
    data: 0,
    entry: ffi::list_head {
        next: ptr::null_mut(),
        prev: ptr::null_mut(),
    },
    func: stage2_worker,
    padding: [0; 100],
};

#[no_mangle]
fn init_vmsh() {
    printkln!("stage1: init");
    unsafe {
        let wq: *mut *mut ffi::workqueue_struct =
            ffi::__symbol_get(c_str!("system_wq").as_ptr() as *mut c_char)
                as *mut *mut ffi::workqueue_struct;
        if wq.is_null() {
            printkln!("stage1: failed to get reference on system work queue (system_wq)");
            return;
        }
        THREAD_SPAWN_WORK.entry.prev = &mut THREAD_SPAWN_WORK.entry;
        THREAD_SPAWN_WORK.entry.next = &mut THREAD_SPAWN_WORK.entry;
        ffi::queue_work_on(0, *wq as *mut c_void, &mut THREAD_SPAWN_WORK);
    };
}
