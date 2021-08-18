use std::{env, path::Path};

use build_utils::{copy_out, rebuild_if_dir_changed, run, stage_dir};
use cc::Build;

fn main() {
    let stage2_dir = stage_dir("../../stage2");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH not set");
    let target = format!("{}-unknown-linux-musl", target_arch);
    rebuild_if_dir_changed(&stage2_dir.join("src"));

    let out_var = env::var_os("OUT_DIR").expect("OUT_DIR is not set");
    let out_dir = Path::new(&out_var);
    println!("cargo:rustc-link-search={}", out_dir.display());

    Build::new().file("trampoline.S").compile("trampoline");
    println!("cargo:rerun-if-changed=trampoline.S");

    run("cargo", |command| {
        command
            .arg("build")
            .arg("--release")
            .arg(format!("--target={}", target))
            .current_dir(&stage2_dir)
    });
    let bin = stage2_dir
        .join("target")
        .join(target)
        .join("release")
        .join("stage2");
    copy_out(&bin);
}
