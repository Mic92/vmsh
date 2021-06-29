use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::prelude::{AsRawFd, RawFd};

use nix::errno::Errno;
use nix::fcntl;
use nix::sys::select;
use nix::sys::time::TimeVal;
use nix::sys::time::TimeValLike;

enum FilePairState {
    Write,
    Read,
}

pub struct FilePair<'a> {
    from: &'a File,
    to: &'a File,
    buf: [u8; libc::BUFSIZ as usize],
    read_offset: usize,
    write_offset: usize,
    state: FilePairState,
}

impl<'a> FilePair<'a> {
    pub fn new(from: &'a File, to: &'a File) -> FilePair<'a> {
        FilePair {
            from,
            to,
            buf: [8; libc::BUFSIZ as usize],
            write_offset: 0,
            read_offset: 0,
            state: FilePairState::Read,
        }
    }
    fn read(&mut self) -> bool {
        match self.from.read(&mut self.buf) {
            Ok(read) => {
                self.read_offset = read;
                self.write()
            }
            Err(_) => false,
        }
    }
    fn write(&mut self) -> bool {
        match self
            .to
            .write(&self.buf[self.write_offset..self.read_offset])
        {
            Ok(written) => {
                self.write_offset += written;
                if self.write_offset >= self.read_offset {
                    self.read_offset = 0;
                    self.write_offset = 0;
                    self.state = FilePairState::Read;
                } else {
                    self.state = FilePairState::Write;
                };
                true
            }
            Err(_) => false,
        }
    }
}

fn set_nonblock(fd: RawFd) -> nix::Result<()> {
    let flags = fcntl::fcntl(fd, fcntl::F_GETFL)?;
    let new_flags = fcntl::OFlag::from_bits_truncate(flags) | fcntl::OFlag::O_NONBLOCK;
    fcntl::fcntl(fd, fcntl::F_SETFL(new_flags))?;
    Ok(())
}

pub fn shovel(pairs: &mut [FilePair], timeout: Option<i64>) -> bool {
    let mut read_set = select::FdSet::new();
    let mut write_set = select::FdSet::new();

    for pair in pairs.iter() {
        if set_nonblock(pair.from.as_raw_fd()).is_err()
            || set_nonblock(pair.to.as_raw_fd()).is_err()
        {
            return false;
        }
    }

    loop {
        read_set.clear();
        write_set.clear();
        let mut highest = 0;

        for pair in pairs.iter_mut() {
            let fd = match pair.state {
                FilePairState::Read => {
                    let raw_fd = pair.from.as_raw_fd();
                    read_set.insert(raw_fd);
                    raw_fd
                }
                FilePairState::Write => {
                    let raw_fd = pair.to.as_raw_fd();
                    write_set.insert(raw_fd);
                    raw_fd
                }
            };
            if highest < fd {
                highest = fd;
            }
        }

        let res = match timeout {
            Some(v) => select::select(
                highest + 1,
                Some(&mut read_set),
                Some(&mut write_set),
                None,
                &mut TimeVal::milliseconds(v),
            ),
            None => select::select(
                highest + 1,
                Some(&mut read_set),
                Some(&mut write_set),
                None,
                None,
            ),
        };

        match res {
            Err(nix::Error::Sys(Errno::EINTR)) => {
                continue;
            }
            Err(_) => {
                return false;
            }
            _ => {}
        }

        for pair in pairs.iter_mut() {
            match pair.state {
                FilePairState::Read => {
                    if read_set.contains(pair.from.as_raw_fd()) && !pair.read() {
                        return false;
                    }
                }
                FilePairState::Write => {
                    if write_set.contains(pair.to.as_raw_fd()) && !pair.write() {
                        return false;
                    }
                }
            }
        }
    }
}
