use libc::c_int;
use nix::fcntl::{self, OFlag};
use nix::sys::mman::{MapFlags, ProtFlags};
use nix::sys::stat;
use nix::unistd::{getpid, Pid};
use simple_error::try_with;
use std::fs::{read_dir, read_link, File};
use std::io::{BufRead, BufReader};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::prelude::RawFd;
use std::path::PathBuf;

use crate::result::Result;

#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    pub start: usize,
    pub end: usize,
    pub prot_flags: ProtFlags,
    pub map_flags: MapFlags,
    pub offset: u64,
    pub major_dev: u64,
    pub minor_dev: u64,
    pub inode: u64,
    pub pathname: String,

    // only for VM mappings, 0 otherwise
    pub phys_addr: usize,
}

impl Mapping {
    pub fn size(&self) -> usize {
        self.end - self.start
    }
    pub fn phys_end(&self) -> usize {
        self.phys_addr + self.size()
    }
    pub fn phys_to_host_offset(&self) -> isize {
        if self.start > self.phys_addr {
            (self.start - self.phys_addr) as isize
        } else {
            -((self.phys_addr - self.start) as isize)
        }
    }
    //pub fn phys_to_host_addr(&self, phys_addr: usize) -> Result<usize> {
    //    let offset = phys_addr - self.phys_addr;
    //    if phys_addr < self.phys_addr || offset > self.size() {
    //        bail!("address is not included in this mapping")
    //    }
    //    Ok(self.start + offset)
    //}
}

pub fn find_mapping(mappings: &[Mapping], ip: usize) -> Option<Mapping> {
    mappings
        .iter()
        .find(|m| m.start <= ip && ip < m.end)
        .cloned()
}

pub struct PidHandle {
    pub pid: Pid,
    file: File,
}

pub fn pid_path(pid: Pid) -> PathBuf {
    PathBuf::from("/proc").join(pid.as_raw().to_string())
}

pub fn openpid(pid: Pid) -> Result<PidHandle> {
    let path = pid_path(pid);
    let fd = try_with!(
        fcntl::open(
            &path,
            OFlag::O_PATH | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            stat::Mode::empty(),
        ),
        "failed to open: {}",
        path.display()
    );
    let file = unsafe { File::from_raw_fd(fd) };

    Ok(PidHandle { pid, file })
}

fn parse_flags(fields: &[u8]) -> (ProtFlags, MapFlags) {
    assert!(fields.len() == 4);
    (
        (if fields[0] == b'r' {
            ProtFlags::PROT_READ
        } else {
            ProtFlags::empty()
        }) | (if fields[1] == b'w' {
            ProtFlags::PROT_WRITE
        } else {
            ProtFlags::empty()
        }) | (if fields[2] == b'x' {
            ProtFlags::PROT_EXEC
        } else {
            ProtFlags::empty()
        }),
        if fields[3] == b'p' {
            MapFlags::MAP_PRIVATE
        } else {
            MapFlags::MAP_SHARED
        },
    )
}

fn parse_line(line: &str) -> Result<Mapping> {
    let fields = line.splitn(6, ' ').collect::<Vec<_>>();
    let range = fields[0].splitn(2, '-').collect::<Vec<_>>();

    let start = try_with!(
        usize::from_str_radix(range[0], 16),
        "start address is not a number: {}",
        range[0]
    );
    let end = try_with!(
        usize::from_str_radix(range[1], 16),
        "end address is not a number: {}",
        range[1]
    );
    let (prot_flags, map_flags) = parse_flags(fields[1].as_bytes());
    let offset = try_with!(
        u64::from_str_radix(fields[2], 16),
        "offset is not a number: {}",
        range[2]
    );
    let dev = fields[3].splitn(2, ':').collect::<Vec<_>>();
    let major_dev = try_with!(
        u64::from_str_radix(dev[0], 16),
        "major dev is not a number: {}",
        dev[0]
    );
    let minor_dev = try_with!(
        u64::from_str_radix(dev[1], 16),
        "minor dev is not a number: {}",
        dev[1]
    );
    let inode = try_with!(
        fields[4].parse::<u64>(),
        "inode is not a number: {}",
        fields[4]
    );
    let stripped = fields[5].trim_start();
    let pathname = stripped.strip_suffix('\n').unwrap_or(stripped).to_string();

    Ok(Mapping {
        start,
        end,
        prot_flags,
        map_flags,
        offset,
        major_dev,
        minor_dev,
        inode,
        pathname,
        phys_addr: 0,
    })
}

pub struct ProcFd {
    pub fd_num: RawFd,
    pub path: PathBuf,
}

impl PidHandle {
    pub fn entry(&self, name: &str) -> PathBuf {
        pid_path(getpid())
            .join("fd")
            .join(self.file.as_raw_fd().to_string())
            .join(name)
    }

    pub fn fds(&self) -> Result<Vec<ProcFd>> {
        let path = self.entry("fd");
        let mut fds = vec![];
        let entries = try_with!(read_dir(&path), "failed to read {}", path.display());
        for maybe_entry in entries {
            let entry = try_with!(maybe_entry, "failed to read {}", path.display());
            let file_name = entry.file_name();
            let target = if let Ok(res) = read_link(entry.path()) {
                res
            } else {
                // file might be closed again
                continue;
            };
            let fd_num = try_with!(
                file_name.to_str().unwrap().parse::<c_int>(),
                "not a valid number: {}",
                PathBuf::from(file_name).display()
            );
            fds.push(ProcFd {
                fd_num,
                path: target,
            });
        }
        Ok(fds)
    }

    pub fn maps(&self) -> Result<Vec<Mapping>> {
        let path = self.entry("maps");
        let f = try_with!(File::open(&path), "cannot open {}", path.display());
        let buf = BufReader::new(f);
        let mut maps = vec![];
        for line in buf.lines() {
            let line = try_with!(line, "cannot read from {}", path.display());
            maps.push(try_with!(parse_line(&line), "cannot parse line {}", line));
        }
        Ok(maps)
    }
}
