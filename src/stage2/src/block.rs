use nix::errno::Errno;
use nix::fcntl::{self, open, OFlag};
use nix::sys::stat::{Mode, SFlag};
use nix::unistd::{unlinkat, UnlinkatFlags};
use simple_error::{bail, try_with};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::os::unix::prelude::{AsRawFd, FromRawFd};
use std::path::PathBuf;
use std::{fs, path::Path};

use crate::procfs;
use crate::result::Result;
use crate::sys_ext::mknodat;

pub struct BlockDevice {
    dev_type: libc::dev_t,
}

pub struct DeviceFile {
    dir: File,
    path: PathBuf,
}

impl DeviceFile {
    fn new(tempdir: &Path, dev: &BlockDevice) -> Result<DeviceFile> {
        let dir_fd = try_with!(
            open(
                tempdir,
                OFlag::O_PATH | OFlag::O_CLOEXEC | OFlag::O_DIRECTORY,
                Mode::empty()
            ),
            "cannot open {} as directory",
            tempdir.display()
        );
        let dir = unsafe { File::from_raw_fd(dir_fd) };
        try_with!(
            mknodat(
                dir.as_raw_fd(),
                "vmsh-blk",
                SFlag::S_IFBLK,
                Mode::S_IWUSR | Mode::S_IRUSR,
                dev.dev_type
            ),
            "cannot create device file"
        );
        Ok(DeviceFile {
            dir,
            path: tempdir.join("vmsh-blk"),
        })
    }
}

impl Drop for DeviceFile {
    fn drop(&mut self) {
        if let Err(e) = unlinkat(
            Some(self.dir.as_raw_fd()),
            "vmsh-blk",
            UnlinkatFlags::NoRemoveDir,
        ) {
            eprintln!("cannot remove temporary block device: {}", e);
        }
    }
}

fn get_filesystems() -> Result<Vec<String>> {
    let path = procfs::get_path().join("filesystems");
    let reader = BufReader::new(try_with!(
        File::open(&path),
        "could not open: {}",
        path.display()
    ));
    let res = reader.lines().filter_map(|line| {
        if let Ok(line) = line {
            if !line.starts_with("nodev") {
                let fs = line.trim();
                return Some(String::from(fs));
            }
        }
        None
    });
    Ok(res.collect::<Vec<_>>())
}

fn set_blocking(file: &File, blocking: bool) -> Result<()> {
    let fd = file.as_raw_fd();
    let flags = try_with!(fcntl::fcntl(fd, fcntl::F_GETFL), "fnctl(F_GETFL) failed");
    let flags = OFlag::from_bits_truncate(flags);

    let flags = if blocking {
        flags & !OFlag::O_NONBLOCK
    } else {
        flags | OFlag::O_NONBLOCK
    };
    try_with!(
        fcntl::fcntl(fd, fcntl::F_SETFL(flags)),
        "fcntl(F_SETFL) failed"
    );

    Ok(())
}

fn dump_dmesg() -> Result<()> {
    let path = procfs::get_path().join("kmsg");
    let file = try_with!(File::open(&path), "could not open: {}", path.display());
    set_blocking(&file, false)?;
    let reader = BufReader::new(file);

    println!("dmesg:");
    for line in reader.lines() {
        match line {
            Ok(line) => println!("{}", &line[3..]),
            // end of file
            Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(e) => bail!("error reading: {}", e),
        };
    }
    Ok(())
}

impl BlockDevice {
    pub fn mount(&self, mountpoint: &Path, selinux_context: &Option<String>) -> Result<()> {
        let dev_file = try_with!(
            DeviceFile::new(mountpoint, &self),
            "cannot create block device file"
        );
        let filesystems = try_with!(get_filesystems(), "could not read supported filesystems");
        for fs in &filesystems {
            let mount_flags = selinux_context
                .as_ref()
                .map(|ctx| PathBuf::from(format!("context=\"{}\"", ctx)));
            let ref_mount_flags = mount_flags.as_deref();

            let res = nix::mount::mount(
                Some(&dev_file.path),
                mountpoint,
                Some(fs.as_str()),
                nix::mount::MsFlags::empty(),
                ref_mount_flags,
            );
            match res {
                Ok(()) => return Ok(()),
                Err(nix::Error::Sys(Errno::EINVAL)) => {}
                Err(e) => {
                    if let Err(e) = dump_dmesg() {
                        eprintln!("dmesg failed {}", e);
                    }

                    bail!(
                        "mount(\"{}\", \"{}\") failed with {}",
                        dev_file.path.display(),
                        mountpoint.display(),
                        e
                    );
                }
            };
        }
        bail!(
            "could not mount image. Tried the following supported filesystems: {}",
            filesystems.join(",")
        );
    }
}

pub fn find_vmsh_blockdev() -> Result<BlockDevice> {
    let dir = try_with!(
        fs::read_dir("/sys/block"),
        "failed to read /sys/block directory"
    );

    for entry in dir {
        let entry = try_with!(entry, "error while reading /proc");
        let serial_path = entry.path().join("serial");
        match fs::read_to_string(&serial_path) {
            // not all block devices implement serial
            Ok(s) if s == "vmsh0" => s,
            _ => continue,
        };
        let dev_path = entry.path().join("dev");
        let major_minor = try_with!(
            fs::read_to_string(&dev_path),
            "cannot read device number from {}",
            dev_path.display()
        );
        let splits = major_minor.trim_end().splitn(2, ':').collect::<Vec<_>>();
        if splits.len() != 2 {
            bail!("could not parse major/minor number: {}", major_minor);
        }

        let major = try_with!(
            splits[0].parse(),
            "could not parse major number: {}",
            splits[0]
        );
        let minor = try_with!(
            splits[1].parse(),
            "could not parse minor number: {}",
            splits[1]
        );
        let dev_type = unsafe { libc::makedev(major, minor) };
        return Ok(BlockDevice { dev_type });
    }

    bail!("no vmsh block device found process found");
}
