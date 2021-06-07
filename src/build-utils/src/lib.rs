use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn stage_dir(name: &str) -> PathBuf {
    let mut dir = env::current_dir().expect("cannot get current working directory");
    dir.push("src");
    dir.push(name);
    dir
}

#[macro_export]
macro_rules! ok(($expression:expr) => ($expression.unwrap()));

#[macro_export]
macro_rules! log {
    ($fmt:expr) => (println!(concat!("vmsh/build.rs:{}: ", $fmt), line!()));
    ($fmt:expr, $($arg:tt)*) => (println!(concat!("vmsh/build.rs:{}: ", $fmt),
    line!(), $($arg)*));
}

pub fn run<F>(name: &str, mut configure: F)
where
    F: FnMut(&mut Command) -> &mut Command,
{
    let mut command = Command::new(name);
    let configured = configure(&mut command);
    log!("Executing {:?}", configured);
    if !ok!(configured.status()).success() {
        panic!("failed to execute {:?}", configured);
    }
    log!("Command {:?} finished successfully", configured);
}

pub fn copy_out(source: &Path) {
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR is not set");
    let target = Path::new(&out_dir).join(source.file_name().expect("source has no filename"));
    println!("cp {} {}", source.display(), target.display());
    fs::copy(source, target)
        .unwrap_or_else(|e| panic!("failed to copy {}: {}", source.display(), e));
}
