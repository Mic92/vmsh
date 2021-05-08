// int pidfd_open(pid_t pid, unsigned int flags);

use nix::errno::Errno;
use nix::unistd::Pid;
use nix::Result;
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::os::unix::prelude::RawFd;

/// Obtain a file descriptor that refers to a process as specified by its PID.
/// A PID file descriptor can be monitored using poll(2), select(2),  and
/// epoll(7).   When  the process that it refers to terminates, these interfaces
/// indicate the file descriptor as  readable.   Note,  however, that in the
/// current implementation, nothing can be read from the file descriptor
/// (read(2) on the file descriptor fails with the error EINVAL).
pub fn pidfd_open(pid: Pid) -> Result<File> {
    let res = unsafe { libc::syscall(libc::SYS_pidfd_open, pid.as_raw(), 0) };

    Errno::result(res).map(|r| unsafe { File::from_raw_fd(r as RawFd) })
}

#[test]
pub fn test_pidfd_open() -> Result<()> {
    use nix::unistd::getpid;
    use std::os::unix::io::AsRawFd;
    let fd = pidfd_open(getpid())?;
    assert!(fd.as_raw_fd() >= 0);

    assert!(pidfd_open(Pid::from_raw(0)).is_err());
    Ok(())
}
