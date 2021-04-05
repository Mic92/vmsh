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
