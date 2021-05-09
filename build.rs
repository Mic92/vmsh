use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

// Example custom build script.
fn main() {
    // Tell Cargo that if the given file changes, to rerun this build script.
    let srcs = ["build.rs", "module.c", "Makefile", "src/lib.rs"];
    for src in &srcs {
        // In theory this breaks paths on windows, but so does the linux build system.
        println!("cargo:rerun-if-changed=src/stage1/{}", src);
    }

    // Re-run build if kernel dir changes
    println!("rerun-if-env-changed=KERNELDIR");

    let kernel_dir = env::var("KERNELDIR").expect("KERNELDIR environment variable not set");

    let mut stage1_dir = env::current_dir().expect("cannot get current working directory");
    stage1_dir.push("src");
    stage1_dir.push("stage1");
    println!(
        "make -C {} M={} RUST_DIR={}",
        kernel_dir,
        stage1_dir.display(),
        stage1_dir.display()
    );
    Command::new("make")
        .arg("-C")
        .arg(kernel_dir)
        .arg(format!("M={}", stage1_dir.display()))
        .arg(format!("RUST_DIR={}", stage1_dir.display()))
        .status()
        .expect("make command failed");

    let kernel_obj = stage1_dir.join("stage1.ko");
    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR is not set");
    let stage1_exe = Path::new(&out_dir).join("stage1.ko");
    println!("cp {} {}", kernel_obj.display(), stage1_exe.display());
    fs::copy(kernel_obj, stage1_exe).expect("failed to copy stage1");
}
