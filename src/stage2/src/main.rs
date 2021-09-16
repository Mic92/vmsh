use kmsg::kmsg_log;
use nix::sys::statfs::{statfs, FsType};
use nix::unistd;
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::exit;
use std::{env, io};
use user_namespace::IdMap;

use crate::block::find_vmsh_blockdev;
use crate::cmd::Cmd;
use crate::dir::mkdir_p;
use crate::result::Result;

mod block;
mod capabilities;
mod cmd;
mod console;
mod dir;
mod kmsg;
mod lsm;
mod mount_context;
mod mountns;
mod namespace;
mod procfs;
mod result;
mod sys_ext;
mod user_namespace;

struct Options {
    target_pid: Pid,
    command: Option<String>,
    args: Vec<String>,
    home: Option<OsString>,
}

fn cleanup_vmsh_exe() {
    if let Err(e) = fs::remove_file("/proc/self/exe") {
        if e.kind() != io::ErrorKind::NotFound {
            return;
        }
    }
    if let Err(e) = fs::remove_file("/dev/.vmsh") {
        if e.kind() != io::ErrorKind::NotFound {
            return;
        }
    }
    let _ = fs::remove_file("/.vmsh");
}

const NONE: Option<&'static [u8]> = None;

pub const PROC_SUPER_MAGIC: FsType = FsType(0x9fa0);
pub const DEVTMPFS_MAGIC: FsType = FsType(0);
pub const SYSFS_MAGIC: FsType = FsType(0x62656572);

fn ensure_procfs() -> Result<()> {
    let procfs = Path::new("/proc");
    try_with!(mkdir_p(&procfs), "cannot create /proc");

    let fs = try_with!(statfs(procfs), "cannot stat /proc");
    if fs.filesystem_type() != PROC_SUPER_MAGIC {
        try_with!(
            nix::mount::mount(
                Some(procfs),
                procfs,
                Some("proc"),
                nix::mount::MsFlags::empty(),
                NONE,
            ),
            "failed to mount procfs"
        );
    }
    Ok(())
}

fn ensure_sysfs() -> Result<()> {
    let sysfs = Path::new("/sys");
    try_with!(mkdir_p(&sysfs), "cannot create /sys");

    let fs = try_with!(statfs(sysfs), "cannot stat /sys");
    if fs.filesystem_type() != SYSFS_MAGIC {
        try_with!(
            nix::mount::mount(
                Some(sysfs),
                sysfs,
                Some("sysfs"),
                nix::mount::MsFlags::empty(),
                NONE,
            ),
            "failed to mount sysfs"
        );
    }
    Ok(())
}

fn ensure_devtmpfs() -> Result<()> {
    let devtmpfs = Path::new("/dev");
    try_with!(mkdir_p(&devtmpfs), "cannot create /dev");

    // devtmpfs does not have a stable magic number from the looks of it
    if !Path::new("/dev/null").exists() {
        try_with!(
            nix::mount::mount(
                Some(devtmpfs),
                devtmpfs,
                Some("devtmpfs"),
                nix::mount::MsFlags::empty(),
                NONE,
            ),
            "failed to mount devtmpfs"
        );
    }
    Ok(())
}

fn run_stage2(opts: &Options) -> Result<()> {
    // get a console to report errors as quick as possible
    try_with!(console::setup(), "failed to setup console");

    // cleanup ourself
    cleanup_vmsh_exe();

    // make sure /proc, /dev and /sys is set up
    try_with!(ensure_procfs(), "cannot set up /proc");
    try_with!(ensure_sysfs(), "cannot set up /sys");
    try_with!(ensure_devtmpfs(), "cannot set up /dev");

    let dev = try_with!(find_vmsh_blockdev(), "cannot find block_device");

    let (uid_map, gid_map) = try_with!(
        IdMap::new_from_pid(opts.target_pid),
        "failed to read usernamespace properties of {}",
        opts.target_pid
    );

    let process_status = try_with!(
        procfs::status(opts.target_pid),
        "failed to get status of target process"
    );

    let metadata = try_with!(
        fs::metadata(procfs::get_path().join(opts.target_pid.to_string())),
        "failed to container uid/gid"
    );

    let container_uid = unistd::Uid::from_raw(uid_map.map_id_up(metadata.uid()));
    let container_gid = unistd::Gid::from_raw(gid_map.map_id_up(metadata.gid()));

    let lsm_profile = try_with!(
        lsm::read_profile(opts.target_pid),
        "failed to get lsm profile"
    );

    let mount_label = if let Some(ref p) = lsm_profile {
        try_with!(
            p.mount_label(opts.target_pid),
            "failed to read mount options"
        )
    } else {
        None
    };

    let supported_namespaces = try_with!(
        namespace::supported_namespaces(),
        "failed to list namespaces"
    );

    if !supported_namespaces.contains(namespace::MOUNT.name) {
        bail!("the system has no support for mount namespaces");
    };

    let mount_namespace = try_with!(
        namespace::MOUNT.open(opts.target_pid),
        "could not access mount namespace"
    );
    let mut other_namespaces = Vec::new();

    let other_kinds = &[
        namespace::UTS,
        namespace::CGROUP,
        namespace::PID,
        namespace::NET,
        namespace::IPC,
        namespace::USER,
    ];

    for kind in other_kinds {
        if !supported_namespaces.contains(kind.name) {
            continue;
        }
        if kind.is_same(opts.target_pid) {
            continue;
        }

        other_namespaces.push(try_with!(
            kind.open(opts.target_pid),
            "failed to open {} namespace",
            kind.name
        ));
    }

    try_with!(mount_namespace.apply(), "failed to apply mount namespace");

    let mount_ns = mountns::setup(&dev, mount_namespace, &mount_label)?;
    let dropped_groups = if supported_namespaces.contains(namespace::USER.name) {
        unistd::setgroups(&[]).is_ok()
    } else {
        false
    };

    for ns in other_namespaces {
        try_with!(ns.apply(), "failed to apply namespace");
    }

    if supported_namespaces.contains(namespace::USER.name) {
        if let Err(e) = unistd::setgroups(&[]) {
            if !dropped_groups {
                try_with!(Err(e), "could not set groups");
            }
        }
        try_with!(unistd::setgid(container_gid), "could not set group id");
        try_with!(unistd::setuid(container_uid), "could not set user id");
    }

    try_with!(
        capabilities::drop(process_status.effective_capabilities),
        "failed to apply capabilities"
    );

    if let Some(profile) = lsm_profile {
        try_with!(profile.inherit_profile(), "failed to inherit lsm profile");
    }

    let cmd = Cmd::new(
        opts.command.clone(),
        opts.args.clone(),
        opts.target_pid,
        opts.home.clone(),
    )?;

    let mut child = cmd.spawn()?;
    // now that we have our child, we can drop temporary mount points

    drop(mount_ns);
    let status = try_with!(child.wait(), "failed to wait for child process");
    eprintln!("process finished with {}", status);
    Ok(())
}

fn main() {
    kmsg_log("[stage2] start\n");
    let args = env::args().collect::<Vec<_>>();
    let command = if args.len() > 2 {
        Some(args[1].clone())
    } else {
        None
    };
    // TODO
    let opts = Options {
        command,
        target_pid: Pid::from_raw(1),
        args: (&args[2..]).to_vec(),
        home: None,
    };
    if let Err(e) = run_stage2(&opts) {
        // print to both allocated pty and kmsg
        kmsg_log(&format!("[stage2] {}\n", e));
        eprintln!("{}", &e);
        exit(1);
    }
}
