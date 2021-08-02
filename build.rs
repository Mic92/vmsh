use std::env;
use std::process::Command;
use std::thread;

use build_utils::{copy_out, log, rebuild_if_dir_changed, run, stage_dir};

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

fn build_stage1_kmod() {
    // Tell Cargo that if the given file changes, to rerun this build script.
    let srcs = ["stage1.ko", "build.rs", "module.c", "Makefile"];

    for src in &srcs {
        println!("cargo:rerun-if-changed=src/stage1/{}", src);
    }

    // Re-run build if kernel dir changes
    println!("cargo:rerun-if-env-changed=KERNELDIR");

    let kernel_dir = env::var("KERNELDIR").unwrap_or_else(|_| fallback_kernel_dir());
    let stage1_dir = stage_dir("stage1");

    log!("cd {} && cargo build --release", stage1_dir.display(),);

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

    copy_out(&stage1_dir.join("stage1.ko"));
}

fn build_stage1_freestanding() {
    let srcs = ["build.rs"];

    for src in &srcs {
        println!("cargo:rerun-if-changed=src/stage1/{}", src);
    }

    let stage1_dir = stage_dir("stage1");
    rebuild_if_dir_changed(&stage1_dir.join("src"));

    let stage2_dir = stage_dir("stage2");
    rebuild_if_dir_changed(&stage2_dir.join("src"));

    run("cargo", |command| {
        command
            .arg("build")
            .arg("--release")
            .current_dir(&stage1_dir)
    });
    copy_out(
        &stage1_dir
            .join("target")
            .join("release")
            .join("libstage1.so"),
    );
}

fn main() {
    let stage1_kmod = thread::spawn(build_stage1_kmod);
    let stage1_freestanding = thread::spawn(build_stage1_freestanding);
    stage1_kmod.join().expect("stage2 failed to build");
    stage1_freestanding.join().expect("stage1 failed to build");
}
