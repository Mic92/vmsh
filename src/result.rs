use simple_error::SimpleError;
use std::result;

pub type Result<T> = result::Result<T, SimpleError>;

#[macro_export]
macro_rules! try_core_res {
    ($expr: expr, $str: expr) => (match $expr {
        Ok(val) => val,
        Err(err) => {
            bail!("{}: {}", $str, err);
        },
    });
    ($expr: expr, $fmt:expr, $($arg:tt)+) => (match $expr {
        Ok(val) => val,
        Err(err) => {
            bail!($fmt, "{}: {}", $($arg)+);
        },
    });
}
