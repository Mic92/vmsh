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

#[macro_export]
macro_rules! dbg_hex {
    // NOTE: We cannot use `concat!` to make a static string as a format argument
    // of `eprintln!` because `file!` could contain a `{` or
    // `$val` expression could be a block (`{ .. }`), in which case the `eprintln!`
    // will be malformed.
    () => {
        ::std::eprintln!("[{}:{}]", ::std::file!(), ::std::line!());
    };
    ($val:expr $(,)?) => {
        // Use of `match` here is intentional because it affects the lifetimes
        // of temporaries - https://stackoverflow.com/a/48732525/1063961
        match $val {
            tmp => {
                ::std::eprintln!("[{}:{}] {} = {:#x}",
                    ::std::file!(), ::std::line!(), ::std::stringify!($val), &tmp);
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($(::std::dbg!($val)),+,)
    };
}
