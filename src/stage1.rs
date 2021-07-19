use log::debug;
use log::{info, log_enabled, warn, Level};
use nix::sys::mman::ProtFlags;
use nix::sys::signal::{kill, SIGTERM};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
/// This module loads kernel code into the VM that we want to attach to.
use simple_error::bail;
use simple_error::{require_with, try_with};
use std::io::Write;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Duration;

use crate::interrutable_thread::InterrutableThread;
use crate::kvm;
use crate::kvm::allocator::VirtAlloc;
use crate::page_table::VirtMem;
use crate::result::Result;

const STAGE1_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stage1.ko"));
const STAGE1_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libstage1_freestanding.so"));

pub struct Stage1 {
    ssh_args: String,
    pub virt_mem: Option<VirtMem>,
}

fn cleanup_stage1(ssh_args: &str) -> Result<()> {
    let mut proc = ssh_command(ssh_args, |cmd| cmd.arg(r#"rmmod stage1.ko"#))?;
    let status = try_with!(proc.wait(), "failed to wait for ssh");
    if !status.success() {
        match status.code() {
            Some(code) => bail!("ssh exited with status code: {}", code),
            None => bail!("ssh terminated by signal"),
        }
    }
    Ok(())
}

impl Drop for Stage1 {
    fn drop(&mut self) {
        debug!("stage1 cleanup started");
        if let Err(e) = cleanup_stage1(&self.ssh_args) {
            warn!("could not cleanup stage1: {}", e);
        }
        debug!("stage1 cleanup finished");
    }
}

fn ssh_command<F>(ssh_args: &str, mut configure: F) -> Result<std::process::Child>
where
    F: FnMut(&mut Command) -> &mut Command,
{
    let mut cmd = Command::new("sh");
    // shell split first argument into multiple
    let cmd_ref = cmd
        .arg("-c")
        .arg(r#"set -f; set -- $1 "$2"; exec ssh -oStrictHostKeyChecking=no -oUserKnownHostsFile=/dev/null "$@""#)
        .arg("--")
        .arg(ssh_args);
    let configured = configure(cmd_ref);
    Ok(try_with!(configured.spawn(), "ssh command failed"))
}

fn padded_size(size: usize) -> usize {
    ((size + 512 - 1) / 512) * 512
}

fn write_padded(f: &mut dyn Write, bytes: &[u8], padded_size: usize) -> Result<()> {
    try_with!(f.write_all(bytes), "Failed to write");
    let mut padding = padded_size - bytes.len();
    while padding != 0 {
        padding -= try_with!(f.write(&[0]), "Failed to write");
    }
    Ok(())
}

fn stage1_thread(
    ssh_args: String,
    command: &[String],
    mmio_addrs: Vec<u64>,
    virt_mem: VirtMem,
    should_stop: Arc<AtomicBool>,
) -> Result<Stage1> {
    std::thread::sleep(Duration::from_millis(3000));

    let debug_stage1 = if log_enabled!(Level::Debug) { "x" } else { "" };

    debug!("load stage1 ({} kB) into vm", STAGE1_LIB.len() / 1024);

    let stage1_size = padded_size(STAGE1_EXE.len());
    let mmio_addrs = mmio_addrs
        .iter()
        .cloned()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let virt_addr = virt_mem.mappings[0].virt_start;
    info!("virt: 0x{:x}", virt_addr);
    let mut child = ssh_command(&ssh_args, move |cmd| -> &mut Command {
        let script = format!(
            r#"
set -eu{} -o pipefail
tmpdir=$(mktemp -d)
trap "rm -rf '$tmpdir'" EXIT
dd if=/proc/self/fd/0 of="$tmpdir/stage1.ko" count={} bs=512
# cleanup old driver if still loaded
rmmod stage1 2>/dev/null || true
insmod "$tmpdir/stage1.ko" devices="{}" stage2_argv="{}" virt_mem="{}"
"#,
            debug_stage1,
            stage1_size / 512,
            mmio_addrs,
            command.join(","),
            virt_addr
        );
        cmd.stdin(Stdio::piped()).arg(script)
    })?;

    info!("wait for payload to be written");
    let mut stdin = require_with!(child.stdin.take(), "Failed to open stdin");
    try_with!(
        write_padded(&mut stdin, STAGE1_EXE, stage1_size),
        "failed to write stage1"
    );

    let pid = nix::unistd::Pid::from_raw(child.id() as i32);

    info!("wait for ssh to complete");
    // In theory interrupting waitpid could be implemented faster with signals...,
    // however since we will replace this eventually. just use a simple sleep...
    let mut wait_flag = Some(WaitPidFlag::WNOHANG);
    loop {
        if should_stop.load(Ordering::Relaxed) {
            try_with!(kill(pid, SIGTERM), "cannot terminate stage1 ssh command");
            wait_flag = None;
        }
        let status = try_with!(waitpid(Some(pid), wait_flag), "waitpid failed");
        match status {
            WaitStatus::StillAlive => {
                std::thread::sleep(Duration::from_millis(100));
            }
            WaitStatus::Exited(_, status) => {
                if status != 0 {
                    bail!("ssh command failed: {}", status);
                }
                break;
            }
            WaitStatus::Signaled(_, SIGTERM, _) => {
                if should_stop.load(Ordering::Relaxed) {
                    break;
                }
                bail!("ssh command was stopped by term signal");
            }
            status => {
                bail!("unexpected wait result: {:?}", status);
            }
        };
    }

    info!("block device driver started");

    Ok(Stage1 {
        ssh_args,
        virt_mem: Some(virt_mem),
    })
}

pub fn spawn_stage1(
    ssh_args: &str,
    command: &[String],
    mmio_ranges: Vec<u64>,
    mut allocator: kvm::PhysMemAllocator,
    result_sender: &SyncSender<()>,
) -> Result<InterrutableThread<Stage1>> {
    let ssh_args = ssh_args.to_string();
    let command = command.to_vec();
    let kernel = try_with!(
        allocator.find_kernel(),
        "could not find Linux kernel in VM memory"
    );
    let virt_start = kernel.virt_start + kernel.len;
    let alloc = [VirtAlloc {
        len: 0x2000,
        prot: ProtFlags::PROT_WRITE,
    }];
    let virt_mem = try_with!(
        allocator.virt_alloc(virt_start, &alloc),
        "cannot map virtual memory"
    );

    let res = InterrutableThread::spawn(
        "stage1",
        result_sender,
        move |should_stop: Arc<AtomicBool>| {
            // wait until vmsh can process block device requests
            stage1_thread(ssh_args, &command, mmio_ranges, virt_mem, should_stop)
        },
    );
    Ok(try_with!(res, "failed to create stage1 thread"))
}
