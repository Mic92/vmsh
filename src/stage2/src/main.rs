use nix::unistd;
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::process::exit;
use user_namespace::IdMap;

use crate::block::find_vmsh_blockdev;
use crate::result::Result;

mod block;
mod capabilities;
mod lsm;
mod mount_context;
mod mountns;
mod namespace;
mod procfs;
mod pty;
mod result;
mod sys_ext;
mod tmp;
mod user_namespace;

fn run_stage2(target_pid: Pid) -> Result<()> {
    // get a console to report errors as quick as possible
    try_with!(pty::forward_thread(), "failed to spawn forward thread");

    let dev = try_with!(find_vmsh_blockdev(), "cannot find block_device");

    let (uid_map, gid_map) = try_with!(
        IdMap::new_from_pid(target_pid),
        "failed to read usernamespace properties of {}",
        target_pid
    );

    let process_status = try_with!(
        procfs::status(target_pid),
        "failed to get status of target process"
    );

    let metadata = try_with!(
        fs::metadata(procfs::get_path().join(target_pid.to_string())),
        "failed to container uid/gid"
    );

    let container_uid = unistd::Uid::from_raw(uid_map.map_id_up(metadata.uid()));
    let container_gid = unistd::Gid::from_raw(gid_map.map_id_up(metadata.gid()));

    let lsm_profile = try_with!(lsm::read_profile(target_pid), "failed to get lsm profile");

    let mount_label = if let Some(ref p) = lsm_profile {
        try_with!(p.mount_label(target_pid), "failed to read mount options")
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
        namespace::MOUNT.open(target_pid),
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
        if kind.is_same(target_pid) {
            continue;
        }

        other_namespaces.push(try_with!(
            kind.open(target_pid),
            "failed to open {} namespace",
            kind.name
        ));
    }

    try_with!(mount_namespace.apply(), "failed to apply mount namespace");

    mountns::setup(&dev, mount_namespace, &mount_label)?;
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

    Ok(())
}

fn log_to_kmsg(msg: &str) {
    let mut v = match OpenOptions::new().write(true).open("/dev/kmsg") {
        Ok(v) => v,
        Err(_) => return,
    };
    let _ = v.write_all(msg.as_bytes());
}

fn main() {
    log_to_kmsg("[stage2] start\n");
    if let Err(e) = run_stage2(Pid::from_raw(1)) {
        // print to both allocated pty and kmsg
        log_to_kmsg(&format!("[stage2] {}\n", e));
        eprintln!("{}", &e);
        exit(1);
    }
}
