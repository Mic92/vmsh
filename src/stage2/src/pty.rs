use std::collections::BinaryHeap;
use std::fs::{self, File, OpenOptions};
use std::os::unix::prelude::IntoRawFd;
use std::os::unix::prelude::{AsRawFd, FromRawFd};
use std::path::PathBuf;
use std::thread::Builder;

use crate::result::Result;

use ioutils::shovel::{shovel, FilePair};

use nix::fcntl::OFlag;
use nix::pty::{grantpt, posix_openpt, ptsname_r, unlockpt, PtyMaster};
use nix::sys::socket::{self, AddressFamily, SockAddr, SockFlag, SockType, VsockAddr};
use nix::sys::stat;
use nix::{fcntl, unistd};
use simple_error::{require_with, try_with};

pub fn open_ptm() -> Result<PtyMaster> {
    let pty_master = try_with!(posix_openpt(OFlag::O_RDWR), "posix_openpt()");

    try_with!(grantpt(&pty_master), "grantpt()");
    try_with!(unlockpt(&pty_master), "unlockpt()");

    Ok(pty_master)
}

pub struct VsockPty {
    vsock: std::fs::File,
    pty: std::fs::File,
}

#[derive(Clone)]
pub struct Pts {
    name: String,
}

impl Pts {
    pub fn attach(&self) -> nix::Result<()> {
        unistd::setsid()?;

        let pty_slave = fcntl::open(self.name.as_str(), OFlag::O_RDWR, stat::Mode::empty())?;

        unistd::dup2(pty_slave, libc::STDIN_FILENO)?;
        unistd::dup2(pty_slave, libc::STDOUT_FILENO)?;
        unistd::dup2(pty_slave, libc::STDERR_FILENO)?;

        unistd::close(pty_slave)?;

        Ok(())
    }
}

impl VsockPty {
    fn forward(&self) {
        shovel(
            &mut [
                FilePair::new(&self.vsock, &self.pty),
                FilePair::new(&self.pty, &self.vsock),
            ],
            None,
        );
    }
}

fn connect_vsock(port: u32) -> Result<File> {
    let raw_sock = try_with!(
        socket::socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC,
            None
        ),
        "cannot create socket"
    );
    let sock = unsafe { File::from_raw_fd(raw_sock) };

    let addr = VsockAddr::new(2, port);

    try_with!(
        socket::connect(sock.as_raw_fd(), &SockAddr::Vsock(addr)),
        "cannot connect vsock({})",
        addr
    );

    Ok(sock)
}

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
            if name.starts_with("hvc") {
                if let Ok(num) = name[3..].parse::<usize>() {
                    heap.push(num);
                }
            }
        }
    }
    let num = require_with!(heap.pop(), "no virtio console found in /dev");
    let monitor = format!("/dev/hvc{}", num);
    let f = try_with!(OpenOptions::new().write(true).open(monitor), "failed to open {}");
    Ok(f)
}

pub fn setup_pty() -> Result<(VsockPty, Pts)> {
    let monitor_console = find_vmsh_consoles()?;
    try_with!(
        unistd::dup2(monitor_console.as_raw_fd(), libc::STDOUT_FILENO),
        "cannot replace stdout with monitor connection"
    );
    try_with!(
        unistd::dup2(monitor_console.as_raw_fd(), libc::STDERR_FILENO),
        "cannot replace stderr with monitor connection"
    );

    let pty_conn = try_with!(connect_vsock(9999), "failed to setup pty connection");

    let pty_master = try_with!(open_ptm(), "failed to open pty master");

    let pts = Pts {
        name: try_with!(ptsname_r(&pty_master), "failed to get pts name"),
    };

    let pty_file = unsafe { File::from_raw_fd(pty_master.into_raw_fd()) };
    let pty = VsockPty {
        vsock: pty_conn,
        pty: pty_file,
    };

    Ok((pty, pts))
}

pub fn forward_thread(pty: VsockPty) -> Result<()> {
    let builder = Builder::new().name(String::from("pty-thread"));
    try_with!(
        builder.spawn(move || pty.forward()),
        "failed to spawn thread"
    );
    Ok(())
}
