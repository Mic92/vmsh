use std::process::Command;

use libc::gettid;

#[allow(dead_code)]
pub fn gdb_break() {
    let tid = unsafe { gettid() }.to_string();

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
