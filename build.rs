use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

fn fallback_kernel_dir() -> String {
    let proc = Command::new("uname")
        .arg("-r")
        .output()
        .expect("uname command failed");
    if !proc.status.success() {
        match proc.status.code() {
            Some(code) => panic!("uname exited with status code: {}", code),
            None => panic!("uname terminated by signal"),
        }
    }
    let kernel_version = String::from_utf8(proc.stdout).expect("cannot decode uname output");
    format!("/lib/modules/{}/build", kernel_version.trim_end())
}

macro_rules! ok(($expression:expr) => ($expression.unwrap()));
macro_rules! log {
    ($fmt:expr) => (println!(concat!("vmsh/build.rs:{}: ", $fmt), line!()));
    ($fmt:expr, $($arg:tt)*) => (println!(concat!("vmsh/build.rs:{}: ", $fmt),
    line!(), $($arg)*));
}

fn run<F>(name: &str, mut configure: F)
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

fn main() {
    if env::var("VMSH_SKIP_KERNEL_BUILD").unwrap_or_else(|_| String::from("0")) == "1" {
        return;
    }

    // Tell Cargo that if the given file changes, to rerun this build script.
    let srcs = [
        "build.rs",
        "module.c",
        "Makefile",
        "src/lib.rs",
        "src/printk.rs",
    ];
    for src in &srcs {
        // In theory this breaks paths on windows, but so does the linux build system.
        println!("cargo:rerun-if-changed=src/stage1/{}", src);
    }

    // Re-run build if kernel dir changes
    println!("rerun-if-env-changed=KERNELDIR");

    let kernel_dir = env::var("KERNELDIR").unwrap_or_else(|_| fallback_kernel_dir());

    let mut stage1_dir = env::current_dir().expect("cannot get current working directory");
    stage1_dir.push("src");
    stage1_dir.push("stage1");

    log!("cd {} && cargo build --release", stage1_dir.display(),);

    run("cargo", |command| {
        command
            .arg("build")
            .arg("--release")
            .current_dir(&stage1_dir)
    });

    let libstage1_object = stage1_dir
        .join("target")
        .join("release")
        .join("libstage1.a");
    let libstage1_symlink = stage1_dir.join("libstage1.o");
    log!(
        "ln -sf {} {}",
        libstage1_object.display(),
        libstage1_symlink.display()
    );
    let _ = fs::remove_file(&libstage1_symlink);
    symlink(&libstage1_object, &libstage1_symlink).unwrap_or_else(|_| {
        panic!(
            "failed to symlink {} to {}",
            libstage1_object.display(),
            libstage1_symlink.display()
        )
    });

    log!(
        "make -C {} M={} RUST_DIR={}",
        kernel_dir,
        stage1_dir.display(),
        stage1_dir.display()
    );
    run("make", |command| {
        command
            .arg("-C")
            .arg(&kernel_dir)
            .arg(format!("M={}", stage1_dir.display()))
            .arg(format!("RUST_DIR={}", stage1_dir.display()))
    });

    let kernel_obj = stage1_dir.join("stage1.ko");
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR is not set");
    let stage1_exe = Path::new(&out_dir).join("stage1.ko");
    println!("cp {} {}", kernel_obj.display(), stage1_exe.display());
    fs::copy(kernel_obj, stage1_exe).expect("failed to copy stage1");
}
