use std::env;
use std::fs;
use std::fs::File;
use std::os::unix::fs::symlink;
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
    let stage1_kmod_dir = stage_dir("stage1-kmod");

    rebuild_if_dir_changed(&stage1_dir.join("src"));
    rebuild_if_dir_changed(&stage1_kmod_dir.join("src"));

    log!("cd {} && cargo build --release", stage1_dir.display(),);

    run("cargo", |command| {
        command
            .arg("build")
            .arg("--release")
            .arg("--target=x86_64-unknown-none-linuxkernel")
            .arg("-Zbuild-std=core")
            .current_dir(&stage1_kmod_dir)
    });

    let libstage1_object = stage1_kmod_dir
        .join("target")
        .join("x86_64-unknown-none-linuxkernel")
        .join("release")
        .join("libstage1_kmod.a");
    let libstage1_symlink = stage1_dir.join("libstage1_kmod.o");
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

    File::create(stage1_dir.join(".libstage1_kmod.o.cmd")).unwrap();

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
    let stage2_dir = stage_dir("stage2");
    let stage1_freestanding_dir = stage_dir("stage1-freestanding");

    rebuild_if_dir_changed(&stage1_freestanding_dir.join("src"));
    rebuild_if_dir_changed(&stage2_dir.join("src"));

    run("cargo", |command| {
        command
            .arg("build")
            .arg("--release")
            .current_dir(&stage1_freestanding_dir)
    });
    copy_out(
        &stage1_freestanding_dir
            .join("target")
            .join("release")
            .join("libstage1_freestanding.so"),
    );
}

fn main() {
    let stage1_kmod = thread::spawn(build_stage1_kmod);
    let stage1_freestanding = thread::spawn(build_stage1_freestanding);
    stage1_kmod.join().expect("stage2 failed to build");
    stage1_freestanding.join().expect("stage1 failed to build");
}
