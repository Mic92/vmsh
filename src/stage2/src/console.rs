use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::os::unix::prelude::{AsRawFd, FromRawFd};
use std::path::PathBuf;

use crate::result::Result;

use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat;
use nix::{fcntl, unistd};
use simple_error::{bail, try_with};

// Linux assigns consoles linear so later added devices get a higher number.
// In theory just assuming vmsh is the last console added is racy however
// in practice it seems unlikely to have consoles added at runtime (famous last words).
pub fn find_vmsh_consoles() -> Result<File> {
    let entries = try_with!(
        fs::read_dir(PathBuf::from("/dev/")),
        "failed to open directory /dev"
    );
    let mut heap = BinaryHeap::new();

    for entry in entries {
        let entry = try_with!(entry, "failed to read /dev");
        if let Some(name) = entry.file_name().to_str() {
            if let Some(stripped) = name.strip_prefix("hvc") {
                if let Ok(num) = stripped.parse::<usize>() {
                    heap.push(num);
                }
            }
        }
    }
    for num in heap {
        let name = format!("/dev/hvc{}", num);
        match fcntl::open(name.as_str(), OFlag::O_RDWR, stat::Mode::empty()) {
            Ok(fd) => return Ok(unsafe { File::from_raw_fd(fd) }),
            Err(Errno::ENODEV) => {}
            e => {
                try_with!(e, "failed to open {}", &name);
            }
        };
    }
    bail!("cannot find vmsh console device in /dev");
}

pub fn setup() -> Result<()> {
    let monitor_console = find_vmsh_consoles()?;
    try_with!(
        unistd::dup2(monitor_console.as_raw_fd(), libc::STDOUT_FILENO),
        "cannot replace stdout with monitor connection"
    );
    try_with!(
        unistd::dup2(monitor_console.as_raw_fd(), libc::STDERR_FILENO),
        "cannot replace stderr with monitor connection"
    );

    Ok(())
}
