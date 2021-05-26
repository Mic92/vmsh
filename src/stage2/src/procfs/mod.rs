use libc::pid_t;
use nix::unistd::Pid;
use simple_error::{try_with, SimpleError};
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::PathBuf;

use crate::result::Result;

mod unix;

// TODO: This came for cntr we might even want to mount our own procfs
pub fn get_path() -> PathBuf {
    PathBuf::from(&env::var_os("CNTR_PROC").unwrap_or_else(|| OsString::from("/proc")))
}

pub struct ProcStatus {
    pub global_pid: Pid,
    pub local_pid: Pid,
    pub inherited_capabilities: u64,
    pub effective_capabilities: u64,
}

pub fn status(target_pid: Pid) -> Result<ProcStatus> {
    let path = get_path().join(target_pid.to_string()).join("status");
    let file = try_with!(File::open(&path), "failed to open {}", path.display());

    let mut ns_pid: Option<Pid> = None;
    let mut inherited_caps: Option<u64> = None;
    let mut effective_caps: Option<u64> = None;

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = try_with!(line, "could not read {}", path.display());
        let columns: Vec<&str> = line.split('\t').collect();
        assert!(columns.len() >= 2);
        if columns[0] == "NSpid:" {
            if let Some(pid_string) = columns.last() {
                let pid = try_with!(
                    pid_string.parse::<pid_t>(),
                    "read invalid pid from proc: '{}'",
                    columns[1]
                );
                ns_pid = Some(Pid::from_raw(pid));
            }
        } else if columns[0] == "CapInh:" {
            if let Some(cap_string) = columns.last() {
                let cap = try_with!(
                    u64::from_str_radix(cap_string, 16),
                    "read invalid capability from proc: '{}'",
                    columns[1]
                );
                inherited_caps = Some(cap);
            }
        } else if columns[0] == "CapEff:" {
            if let Some(cap_string) = columns.last() {
                let cap = try_with!(
                    u64::from_str_radix(cap_string, 16),
                    "read invalid capability from proc: '{}'",
                    columns[1]
                );
                effective_caps = Some(cap);
            }
        }
    }

    Ok(ProcStatus {
        global_pid: target_pid,
        local_pid: try_with!(
            ns_pid.ok_or_else(|| SimpleError::new(format!(
                "Could not find namespace pid in {}",
                path.display()
            ))),
            ""
        ),
        inherited_capabilities: try_with!(
            inherited_caps.ok_or_else(|| {
                SimpleError::new(format!(
                    "Could not find inherited capabilities in {}",
                    path.display()
                ))
            }),
            ""
        ),
        effective_capabilities: try_with!(
            effective_caps.ok_or_else(|| {
                SimpleError::new(format!(
                    "Could not find effective capabilities in {}",
                    path.display()
                ))
            }),
            ""
        ),
    })
}
