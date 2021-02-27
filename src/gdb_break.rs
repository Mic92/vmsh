use std::process::Command;

use libc::{pid_t, syscall, SYS_gettid};

#[allow(dead_code)]
pub fn gdb_break() {
    let tid = unsafe { syscall(SYS_gettid) as pid_t }.to_string();

    println!("GDB PROBE HIT, WAITING");
    // 1. finish: wait4
    // 2. finish: Process::wait
    // 3. finish: gdb::break
    // 4. finish: caller
    let args = vec![
        "-c", "tmux new-window sudo -E gdb --pid \"$0\" -ex \"shell kill -9 $$\" -ex finish -ex finish -ex finish -ex finish; kill -STOP $$", &tid
    ];
    let _ = Command::new("sh").args(args.as_slice()).status();
}
