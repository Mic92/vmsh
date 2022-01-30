use chlorine::c_char;

extern "C" {
    pub fn _printk(fmt: *const c_char, ...);
}

#[macro_export]
macro_rules! c_str {
    ($arg:expr) => {
        concat!($arg, '\x00')
    };
}

#[macro_export]
macro_rules! printk {
    // Static (zero-allocation) implementation that uses compile-time `concat!()` only
    ($fmt:expr) => ({
        // KERN_SOH + KERN_INFO + fmt + null byte
        let msg = concat!("\x016", c_str!($fmt));
        let ptr = msg.as_ptr() as *const ::chlorine::c_char;
        #[allow(unused_unsafe)]
        unsafe { crate::printk::_printk(ptr) };
    });

    // Dynamic implementation that processes format arguments
    ($fmt:expr, $($arg:tt)*) => ({
        // KERN_SOH + KERN_INFO + fmt + null byte
        let msg = concat!("\x016", c_str!($fmt));
        let ptr = msg.as_ptr() as *const ::chlorine::c_char;
        #[allow(unused_unsafe)]
        unsafe { crate::printk::_printk(ptr, $($arg)*) };
    });
}

#[macro_export]
macro_rules! printkln {
    ($fmt:expr)              => (printk!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)+) => (printk!(concat!($fmt, "\n"), $($arg)*));
}
