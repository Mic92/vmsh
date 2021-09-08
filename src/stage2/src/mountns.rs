use ioutils::tmp;
use nix::sched::CloneFlags;
use nix::sys::wait::waitpid;
use nix::sys::wait::WaitStatus;
use nix::unistd::fork;
use nix::{mount, sched, unistd};
use nix::{mount::MsFlags, unistd::getpid};
use simple_error::try_with;
use simple_error::SimpleError;
use std::fs::File;
use std::fs::{create_dir_all, metadata, remove_dir};
use std::fs::{set_permissions, Permissions};
use std::io;
use std::os::unix::prelude::*;
use std::path::{Path, PathBuf};

use crate::block::BlockDevice;
use crate::namespace::{self, MOUNT};
use crate::result::Result;

pub struct MountNamespace {
    new_namespace: namespace::Namespace,
    old_namespace: namespace::Namespace,
    mountpoint: PathBuf,
    temp_mountpoint: PathBuf,
}

const MOUNTS: &[&str] = &[
    "etc/passwd",
    "etc/group",
    "etc/resolv.conf",
    "etc/hosts",
    "etc/hostname",
    "etc/localtime",
    "etc/zoneinfo",
    "dev",
    "sys",
    "proc",
];

const VMSH_MOUNT_POINT: &str = "var/lib/vmsh";

impl MountNamespace {
    fn new(old_namespace: namespace::Namespace) -> Result<MountNamespace> {
        // Find some other writeable mountpoint if / is readonly? /dev/shm fallback?
        let path = PathBuf::from("/tmp");
        // TODO we should not create /tmp
        try_with!(mkdir_p(&path), "failed to create /tmp");

        let mountpoint = try_with!(tmp::tempdir(), "failed to create temporary mountpoint");
        try_with!(
            set_permissions(mountpoint.path(), Permissions::from_mode(0o755)),
            "cannot change permissions of '{}'",
            mountpoint.path().display()
        );

        let temp_mountpoint = try_with!(tmp::tempdir(), "failed to create temporary mountpoint");
        try_with!(
            set_permissions(temp_mountpoint.path(), Permissions::from_mode(0o755)),
            "cannot change permissions of '{}'",
            temp_mountpoint.path().display()
        );

        try_with!(
            sched::unshare(CloneFlags::CLONE_NEWNS),
            "failed to create mount namespace"
        );

        let new_namespace = try_with!(MOUNT.open(getpid()), "cannot open new mount namespace");

        Ok(MountNamespace {
            new_namespace,
            old_namespace,
            mountpoint: mountpoint.into_path(),
            temp_mountpoint: temp_mountpoint.into_path(),
        })
    }

    fn _cleanup(&self) -> Result<()> {
        try_with!(
            self.old_namespace.apply(),
            "failed to switch back to old mount namespace"
        );
        try_with!(
            remove_dir(&self.mountpoint),
            "failed to cleanup mountpoint {}",
            self.mountpoint.display()
        );
        try_with!(
            remove_dir(&self.temp_mountpoint),
            "failed to cleanup mountpoint {}",
            self.mountpoint.display()
        );
        try_with!(
            self.new_namespace.apply(),
            "cannot switch back to new mount namespace"
        );
        Ok(())
    }

    fn cleanup(&self) -> Result<()> {
        match unsafe { fork() } {
            Ok(unistd::ForkResult::Parent { child, .. }) => {
                match try_with!(waitpid(child, None), "could not wait for child") {
                    WaitStatus::Exited(_, 0) => Ok(()),
                    _ => Err(SimpleError::new("cannot cleanup mountpoints")),
                }
            }
            Ok(unistd::ForkResult::Child) => {
                let rc = match self._cleanup() {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("cannot cleanup mount namespace: {}", e);
                        1
                    }
                };
                std::process::exit(rc)
            }
            Err(e) => {
                panic!("fork failed: {}", e);
            }
        }
    }
}

impl Drop for MountNamespace {
    fn drop(&mut self) {
        if let Err(e) = self.cleanup() {
            eprintln!("{}", e);
        }
    }
}

const NONE: Option<&'static [u8]> = None;

fn mkdir_p<P: AsRef<Path>>(path: &P) -> io::Result<()> {
    if let Err(e) = create_dir_all(path) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(e);
        }
    }
    Ok(())
}

pub fn setup_bindmounts(mounts: &[&str]) -> Result<()> {
    for m in mounts {
        let mountpoint_buf = PathBuf::from("/").join(m);
        let mountpoint = mountpoint_buf.as_path();
        let source_buf = PathBuf::from("/var/lib/vmsh").join(m);
        let source = source_buf.as_path();

        let source_stat = match metadata(source) {
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    continue;
                }
                return try_with!(
                    Err(e),
                    "failed to get metadata of path {}",
                    source.display()
                );
            }
            Ok(data) => data,
        };

        if !mountpoint.exists() {
            // degrade gracefully if fs is read-only
            if source_stat.is_dir() {
                if mkdir_p(&mountpoint).is_err() {
                    continue;
                };
            } else {
                if mkdir_p(&mountpoint.parent().unwrap()).is_err() {
                    continue;
                };
                if File::create(mountpoint).is_err() {
                    continue;
                }
            }
        }
        let mountpoint_stat = match metadata(mountpoint) {
            Err(e) => {
                return try_with!(
                    Err(e),
                    "failed to get metadata of path {}",
                    mountpoint.display()
                );
            }
            Ok(data) => data,
        };

        #[allow(clippy::suspicious_operation_groupings)]
        if !((source_stat.is_file() && !mountpoint_stat.is_dir())
            || (source_stat.is_dir() && mountpoint_stat.is_dir()))
        {
            continue;
        }

        let res = mount::mount(
            Some(source),
            mountpoint,
            NONE,
            MsFlags::MS_REC | MsFlags::MS_BIND,
            NONE,
        );

        if res.is_err() {
            eprintln!("could not bind mount {:?}", mountpoint);
        }
    }
    Ok(())
}

pub fn setup(
    device: &BlockDevice,
    container_namespace: namespace::Namespace,
    mount_label: &Option<String>,
) -> Result<MountNamespace> {
    let ns = MountNamespace::new(container_namespace)?;

    try_with!(
        mount::mount(
            Some("none"),
            "/",
            NONE,
            MsFlags::MS_REC | MsFlags::MS_PRIVATE,
            NONE,
        ),
        "unable to mark mounts as private"
    );

    // prepare bind mounts
    try_with!(
        mount::mount(
            Some("/"),
            &ns.temp_mountpoint,
            NONE,
            MsFlags::MS_REC | MsFlags::MS_BIND,
            NONE,
        ),
        "unable to move mounts to temporary mountpoint"
    );

    device.mount(ns.mountpoint.as_path(), mount_label)?;

    let vmsh_mount_point = &ns.mountpoint.join(VMSH_MOUNT_POINT);
    try_with!(
        mkdir_p(&vmsh_mount_point),
        "cannot create container mountpoint /{}",
        VMSH_MOUNT_POINT
    );
    let flags = MsFlags::MS_REC | MsFlags::MS_MOVE;
    try_with!(
        mount::mount(
            Some(&ns.temp_mountpoint),
            vmsh_mount_point.as_path(),
            NONE,
            flags,
            NONE,
        ),
        "unable to move mounts to new mountpoint: mount(\"{}\",\"{}\",NULL,MS_REC|MS_MOVE,NULL)",
        ns.temp_mountpoint.display(),
        vmsh_mount_point.display()
    );

    // useful for debugging
    //eprintln!(
    //    "/proc/self/mountinfo: {}",
    //    try_with!(
    //        std::fs::read_to_string("/proc/self/mountinfo"),
    //        "cannot read /proc/self/mountinfo"
    //    )
    //);

    try_with!(
        unistd::chdir(&ns.mountpoint),
        "failed to chdir to new mountpoint"
    );

    try_with!(
        unistd::chroot(&ns.mountpoint),
        "failed to chroot to new mountpoint"
    );

    // ensure we have at least these directories for bind mounts
    for p in &["/dev", "/sys", "/proc"] {
        try_with!(mkdir_p(p), "cannot create directory {}", p);
    }

    try_with!(setup_bindmounts(MOUNTS), "failed to setup bind mounts");

    Ok(ns)
}
