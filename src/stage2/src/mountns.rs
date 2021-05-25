use nix::sched::CloneFlags;
use nix::{mount, sched, unistd};
use nix::{mount::MsFlags, unistd::getpid};
use simple_error::try_with;
use std::fs::{create_dir_all, metadata, remove_dir};
use std::fs::{set_permissions, Permissions};
use std::io;
use std::os::unix::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::block::BlockDevice;
use crate::namespace::{self, MOUNT};
use crate::result::Result;
use crate::tmp;

pub struct MountNamespace {
    new_namespace: namespace::Namespace,
    old_namespace: namespace::Namespace,
    mountpoint: PathBuf,
    temp_mountpoint: PathBuf,
    in_cleanup: bool,
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

const CNTR_MOUNT_POINT: &str = "var/lib/vmsh";

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
            in_cleanup: false,
        })
    }

    pub fn cleanup(&mut self) -> Result<()> {
        self.in_cleanup = true;
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
}

impl Drop for MountNamespace {
    fn drop(&mut self) {
        if self.in_cleanup {
            return;
        }
        if let Err(e) = self.cleanup() {
            eprintln!("cannot cleanup mountpoints: {}", e);
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

        let mountpoint_stat = match metadata(mountpoint) {
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    continue;
                }
                return try_with!(
                    Err(e),
                    "failed to get metadata of path {}",
                    mountpoint.display()
                );
            }
            Ok(data) => data,
        };

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
) -> Result<()> {
    let mut ns = MountNamespace::new(container_namespace)?;

    try_with!(
        mount::mount(
            Some("none"),
            "/",
            NONE,
            MsFlags::MS_REC | MsFlags::MS_PRIVATE,
            NONE,
        ),
        "unable to bind mount /"
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
    let cntr_mount_point = &ns.mountpoint.join(CNTR_MOUNT_POINT);
    try_with!(
        mkdir_p(&cntr_mount_point),
        "cannot create container mountpoint /{}",
        CNTR_MOUNT_POINT
    );

    try_with!(
        mount::mount(
            Some(&ns.temp_mountpoint),
            cntr_mount_point.as_path(),
            NONE,
            MsFlags::MS_REC | MsFlags::MS_MOVE,
            NONE,
        ),
        "unable to move mounts to new mountpoint"
    );

    try_with!(
        unistd::chdir(&ns.mountpoint),
        "failed to chdir to new mountpoint"
    );

    try_with!(
        unistd::chroot(&ns.mountpoint),
        "failed to chroot to new mountpoint"
    );

    try_with!(setup_bindmounts(MOUNTS), "failed to setup bind mounts");

    try_with!(
        Command::new("/bin/sh")
            .arg("-c")
            .arg("echo stage2 works")
            .status(),
        "failed to run /bin/sh"
    );

    try_with!(ns.cleanup(), "cannot cleanup mountns");

    Ok(())
}
