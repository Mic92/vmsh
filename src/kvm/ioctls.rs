// borrowed from vmm-sys-util

use kvm_bindings as kvmb;

/// Expression that calculates an ioctl number.
///
/// ```
/// const KVMIO: c_uint = 0xAE;
/// ioctl_expr!(_IOC_NONE, KVMIO, 0x01, 0);
/// ```
macro_rules! ioctl_expr {
    ($dir:expr, $ty:expr, $nr:expr, $size:expr) => {
        (($dir << _IOC_DIRSHIFT)
            | ($ty << _IOC_TYPESHIFT)
            | ($nr << _IOC_NRSHIFT)
            | ($size << _IOC_SIZESHIFT)) as ::std::os::raw::c_ulong
    };
}

/// Declare a function that returns an ioctl number.
///
/// ```
/// # use std::os::raw::c_uint;
///
/// const KVMIO: c_uint = 0xAE;
/// ioctl_ioc_nr!(KVM_CREATE_VM, _IOC_NONE, KVMIO, 0x01, 0);
/// ```
macro_rules! ioctl_ioc_nr {
    ($name:ident, $dir:expr, $ty:expr, $nr:expr, $size:expr) => {
        #[allow(non_snake_case)]
        #[allow(clippy::cast_lossless)]
        pub fn $name() -> ::std::os::raw::c_ulong {
            ioctl_expr!($dir, $ty, $nr, $size)
        }
    };
    ($name:ident, $dir:expr, $ty:expr, $nr:expr, $size:expr, $($v:ident),+) => {
        #[allow(non_snake_case)]
        #[allow(clippy::cast_lossless)]
        pub fn $name($($v: ::std::os::raw::c_uint),+) -> ::std::os::raw::c_ulong {
            ioctl_expr!($dir, $ty, $nr, $size)
        }
    };
}

/// Declare an ioctl that transfers no data.
///
/// ```
/// const KVMIO: c_uint = 0xAE;
/// ioctl_io_nr!(KVM_CREATE_VM, KVMIO, 0x01);
/// ```
macro_rules! ioctl_io_nr {
    ($name:ident, $ty:expr, $nr:expr) => {
        ioctl_ioc_nr!($name, _IOC_NONE, $ty, $nr, 0);
    };
    ($name:ident, $ty:expr, $nr:expr, $($v:ident),+) => {
        ioctl_ioc_nr!($name, _IOC_NONE, $ty, $nr, 0, $($v),+);
    };
}

/// Declare an ioctl that writes data.
///
/// ```
/// # #[macro_use] extern crate vmm_sys_util;
/// const TUNTAP: ::std::os::raw::c_uint = 0x54;
/// ioctl_iow_nr!(TUNSETQUEUE, TUNTAP, 0xd9, ::std::os::raw::c_int);
/// ```
macro_rules! ioctl_iow_nr {
    ($name:ident, $ty:expr, $nr:expr, $size:ty) => {
        ioctl_ioc_nr!(
            $name,
            _IOC_WRITE,
            $ty,
            $nr,
            ::std::mem::size_of::<$size>() as u32
        );
    };
    ($name:ident, $ty:expr, $nr:expr, $size:ty, $($v:ident),+) => {
        ioctl_ioc_nr!(
            $name,
            _IOC_WRITE,
            $ty,
            $nr,
            ::std::mem::size_of::<$size>() as u32,
            $($v),+
        );
    };
}

/// Declare an ioctl that reads data.
///
/// ```
/// # #[macro_use] extern crate vmm_sys_util;
/// const TUNTAP: ::std::os::raw::c_uint = 0x54;
/// ioctl_ior_nr!(TUNGETFEATURES, TUNTAP, 0xcf, ::std::os::raw::c_uint);
/// ```
macro_rules! ioctl_ior_nr {
    ($name:ident, $ty:expr, $nr:expr, $size:ty) => {
        ioctl_ioc_nr!(
            $name,
            _IOC_READ,
            $ty,
            $nr,
            ::std::mem::size_of::<$size>() as u32
        );
    };
    ($name:ident, $ty:expr, $nr:expr, $size:ty, $($v:ident),+) => {
        ioctl_ioc_nr!(
            $name,
            _IOC_READ,
            $ty,
            $nr,
            ::std::mem::size_of::<$size>() as u32,
            $($v),+
        );
    };
}

/// Declare an ioctl that writes data.
///
/// ```
/// # #[macro_use] extern crate vmm_sys_util;
/// const TUNTAP: ::std::os::raw::c_uint = 0x54;
/// ioctl_iow_nr!(TUNSETQUEUE, TUNTAP, 0xd9, ::std::os::raw::c_int);
/// ```
//macro_rules! ioctl_iow_nr {
//    ($name:ident, $ty:expr, $nr:expr, $size:ty) => {
//        ioctl_ioc_nr!(
//            $name,
//            _IOC_WRITE,
//            $ty,
//            $nr,
//            ::std::mem::size_of::<$size>() as u32
//        );
//    };
//    ($name:ident, $ty:expr, $nr:expr, $size:ty, $($v:ident),+) => {
//        ioctl_ioc_nr!(
//            $name,
//            _IOC_WRITE,
//            $ty,
//            $nr,
//            ::std::mem::size_of::<$size>() as u32,
//            $($v),+
//        );
//    };
//}

/// Declare an ioctl that reads and writes data.
#[macro_export]
macro_rules! ioctl_iowr_nr {
    ($name:ident, $ty:expr, $nr:expr, $size:ty) => {
        ioctl_ioc_nr!(
            $name,
            _IOC_READ | _IOC_WRITE,
            $ty,
            $nr,
            ::std::mem::size_of::<$size>() as u32
        );
    };
    ($name:ident, $ty:expr, $nr:expr, $size:ty, $($v:ident),+) => {
        ioctl_ioc_nr!(
            $name,
            _IOC_READ | _IOC_WRITE,
            $ty,
            $nr,
            ::std::mem::size_of::<$size>() as u32,
            $($v),+
        );
    };
}

// Define IOC_* constants in a module so that we can allow missing docs on it.
// There is not much value in documenting these as it is code generated from
// kernel definitions.
use std::os::raw::c_uint;

const _IOC_NRBITS: c_uint = 8;
const _IOC_TYPEBITS: c_uint = 8;
const _IOC_SIZEBITS: c_uint = 14;
const _IOC_DIRBITS: c_uint = 2;
const _IOC_NRMASK: c_uint = 255;
const _IOC_TYPEMASK: c_uint = 255;
const _IOC_SIZEMASK: c_uint = 16383;
const _IOC_DIRMASK: c_uint = 3;
const _IOC_NRSHIFT: c_uint = 0;
const _IOC_TYPESHIFT: c_uint = 8;
const _IOC_SIZESHIFT: c_uint = 16;
const _IOC_DIRSHIFT: c_uint = 30;
const _IOC_NONE: c_uint = 0;
const _IOC_WRITE: c_uint = 1;
const _IOC_READ: c_uint = 2;
//const IOC_IN: c_uint = 1_073_741_824;
//const IOC_OUT: c_uint = 2_147_483_648;
//const IOC_INOUT: c_uint = 3_221_225_472;
//const IOCSIZE_MASK: c_uint = 1_073_676_288;
//const IOCSIZE_SHIFT: c_uint = 16;

const KVMIO: c_uint = 0xAE;

// Ioctls for /dev/kvm.
ioctl_io_nr!(KVM_CHECK_EXTENSION, KVMIO, 0x03);

// Available with KVM_CAP_IOEVENTFD
ioctl_iow_nr!(KVM_IOEVENTFD, KVMIO, 0x79, kvmb::kvm_ioeventfd);

// Available with KVM_CAP_IRQFD
ioctl_iow_nr!(KVM_IRQFD, KVMIO, 0x76, kvmb::kvm_irqfd);

// Avaulable with KVM_CAP_USER_MEMORY
ioctl_iow_nr!(
    KVM_SET_USER_MEMORY_REGION,
    KVMIO,
    0x46,
    kvmb::kvm_userspace_memory_region
);

// Available with KVM_CAP_IOREGIONFD
ioctl_iow_nr!(KVM_SET_IOREGION, KVMIO, 0x49, kvm_ioregion);

ioctl_io_nr!(KVM_RUN, KVMIO, 0x80);

// Ioctls for VM fds.
/* Available with KVM_CAP_USER_MEMORY */
//ioctl_iow_nr!(
//    KVM_SET_USER_MEMORY_REGION,
//    KVMIO,
//    0x46,
//    kvm_userspace_memory_region
//);

// Ioctls for VCPU fds.
#[cfg(not(any(target_arch = "arm", target_arch = "aarch64")))]
ioctl_ior_nr!(KVM_GET_REGS, KVMIO, 0x81, kvmb::kvm_regs);
#[cfg(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "powerpc",
    target_arch = "powerpc64"
))]
ioctl_ior_nr!(KVM_GET_SREGS, KVMIO, 0x83, kvmb::kvm_sregs);

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
ioctl_ior_nr!(KVM_GET_FPU, KVMIO, 0x8c, kvmb::kvm_fpu);
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
ioctl_iow_nr!(KVM_SET_FPU, KVMIO, 0x8d, kvmb::kvm_fpu);
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
ioctl_iowr_nr!(KVM_GET_MSRS, KVMIO, 0x88, kvmb::kvm_msrs);
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]

/// according to arch/x86/include/asm/kvm_host.h
pub const KVM_MAX_CPUID_ENTRIES: usize = 256;

/// for simplicity we use a fixed kvm_cpuid2 instead of the dynamic sized kvmb::kvm_cpuid2
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct kvm_cpuid2 {
    pub nent: u32,
    pub padding: u32,
    pub entries: [kvmb::kvm_cpuid_entry2; KVM_MAX_CPUID_ENTRIES],
}
ioctl_iowr_nr!(KVM_GET_CPUID2, KVMIO, 0x91, kvmb::kvm_cpuid2);

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

#[allow(non_camel_case_types)]
enum kvm_ioregion_flag_nr {
    pio,
    posted_writes,
    max,
}

//#define KVM_IOREGION_PIO (1 << kvm_ioregion_flag_nr_pio)
const KVM_IOREGION_PIO: u32 = (1 << kvm_ioregion_flag_nr::pio as u32);
//#define KVM_IOREGION_POSTED_WRITES (1 << kvm_ioregion_flag_nr_posted_writes)
const KVM_IOREGION_POSTED_WRITES: u32 = (1 << kvm_ioregion_flag_nr::posted_writes as u32);
//#define KVM_IOREGION_VALID_FLAG_MASK ((1 << kvm_ioregion_flag_nr_max) - 1)
const KVM_IOREGION_VALID_FLAG_MASK: u32 = ((1 << kvm_ioregion_flag_nr::max as u32) - 1);

pub const KVM_CAP_IOREGIONFD: u32 = 195;

/// wire protocol
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
struct ioregionfd_cmd {
    pub info: Info,
    pub padding: u32,
    pub user_data: u64,
    pub offset: u64,
    pub data: u64,
}

/// wire protocol
struct ioregionfd_resp {
    pub data: u64,
    pub pad: [u8; 24],
}

//libc_bitflags! {
//pub struct Info

#[repr(C)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
struct Info(u32);

pub const IOREGIONFD_SIZE_OFFSET: usize = 4;
pub const IOREGIONFD_RESP_OFFSET: usize = 6;
//+#define IOREGIONFD_SIZE(x) ((x) << IOREGIONFD_SIZE_OFFSET)
//+#define IOREGIONFD_RESP(x) ((x) << IOREGIONFD_RESP_OFFSET)
impl Info {
    pub fn new(cmd: Cmd, size: Size, response: bool) -> Self {
        let mut ret = 0;
        ret |= cmd as u32;
        ret |= (size as u32) << IOREGIONFD_SIZE_OFFSET;
        ret |= (response as u32) << IOREGIONFD_RESP_OFFSET;
        Info(ret)
    }
}

pub const IOREGIONFD_CMD_READ: usize = 0;
pub const IOREGIONFD_CMD_WRITE: usize = 1;
enum Cmd {
    Read,
    Write,
}

//pub const IOREGIONFD_SIZE_8BIT: usize = 0;
//pub const IOREGIONFD_SIZE_16BIT: usize = 1;
//pub const IOREGIONFD_SIZE_32BIT: usize = 2;
//pub const IOREGIONFD_SIZE_64BIT: usize = 3;
#[allow(non_camel_case_types)]
enum Size {
    b8,
    b16,
    b32,
    b64,
}
