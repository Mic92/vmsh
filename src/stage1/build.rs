use std::env;

use build_utils::{copy_out, rebuild_if_dir_changed, run, stage_dir};

fn main() {
    let stage2_dir = stage_dir("../../stage2");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH not set");
    let target = format!("{}-unknown-linux-musl", target_arch);
    rebuild_if_dir_changed(&stage2_dir.join("src"));

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
