use std::{fs::OpenOptions, io::Write};

pub fn kmsg_log(msg: &str) {
    let mut v = match OpenOptions::new().write(true).open("/dev/kmsg") {
        Ok(v) => v,
        Err(_) => return,
    };
    let _ = v.write_all(msg.as_bytes());
}
