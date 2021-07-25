use log::{debug, warn};
use log::{info, log_enabled, Level};
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
use crate::kernel::{find_kernel, Kernel};
use crate::kvm;
use crate::kvm::allocator::VirtAlloc;
use crate::loader::Loader;
use crate::page_table::VirtMem;
use crate::result::Result;

const STAGE1_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stage1.ko"));
const STAGE1_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libstage1_freestanding.so"));

pub struct Stage1 {
    virt_mem: VirtMem,
    kernel: Kernel,
}

impl Stage1 {
    pub fn new(mut allocator: kvm::PhysMemAllocator) -> Result<Stage1> {
        let kernel = find_kernel(&allocator.guest_mem, &allocator.hv)?;

        let alloc = [
            VirtAlloc {
                len: 256 * 0x1000,
                prot: ProtFlags::PROT_WRITE,
            },
            VirtAlloc {
                len: 512 * 0x1000,
                prot: ProtFlags::PROT_WRITE,
            },
        ];
        let virt_mem = try_with!(
            allocator.virt_alloc(kernel.range.end, &alloc),
            "cannot map virtual memory"
        );
        let base = kernel.range.end;
        let mut loader = try_with!(
            Loader::new(STAGE1_LIB, base as u64, &mut allocator),
            "cannot load stage1"
        );

        try_with!(loader.load_binary(), "cannot load stage1");

        debug!("load stage1 ({} kB) into vm", STAGE1_LIB.len() / 1024);

        Ok(Stage1 { virt_mem, kernel })
    }

    pub fn spawn(
        &self,
        ssh_args: &str,
        command: &[String],
        mmio_ranges: Vec<u64>,
        result_sender: &SyncSender<()>,
    ) -> Result<InterrutableThread<Kmod>> {
        let ssh_args = ssh_args.to_string();
        let command = command.to_vec();

        let printk_addr = *require_with!(
            self.kernel.symbols.get("printk"),
            "no printk function found in kernel symbols"
        );

        let virt_addr = self.virt_mem.mappings[1].virt_start;

        let res = InterrutableThread::spawn(
            "stage1",
            result_sender,
            move |should_stop: Arc<AtomicBool>| {
                // wait until vmsh can process block device requests
                stage1_thread(
                    ssh_args,
                    &command,
                    mmio_ranges,
                    virt_addr,
                    printk_addr,
                    should_stop,
                )
            },
        );
        Ok(try_with!(res, "failed to create stage1 thread"))
    }
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

pub struct Kmod {
    ssh_args: String,
}

impl Drop for Kmod {
    fn drop(&mut self) {
        debug!("stage1 cleanup started");
        if let Err(e) = cleanup_stage1(&self.ssh_args) {
            warn!("could not cleanup stage1: {}", e);
        }
        debug!("stage1 cleanup finished");
    }
}

fn stage1_thread(
    ssh_args: String,
    command: &[String],
    mmio_addrs: Vec<u64>,
    virt_addr: usize,
    printk_addr: usize,
    should_stop: Arc<AtomicBool>,
) -> Result<Kmod> {
    let debug_stage1 = if log_enabled!(Level::Debug) { "x" } else { "" };
    let stage1_size = padded_size(STAGE1_EXE.len());

    let mmio_addrs = mmio_addrs
        .iter()
        .cloned()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(",");

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
insmod "$tmpdir/stage1.ko" devices="{}" stage2_argv="{}" virt_mem="{}" printk_addr="{}"
"#,
            debug_stage1,
            stage1_size / 512,
            mmio_addrs,
            command.join(","),
            virt_addr,
            printk_addr
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
    Ok(Kmod { ssh_args })
}

//#[cfg(test)]
//mod tests {
//    use crate::{loader::Loader, stage1::STAGE1_LIB};
//
//    #[test]
//    fn test_load_binary() {
//        let mut loader = Loader::new(STAGE1_LIB).expect("cannot load stage1");
//        loader.load_binary().expect("cannot load stage1");
//    }
//}
