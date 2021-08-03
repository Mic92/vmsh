use build_utils::{copy_out, rebuild_if_dir_changed, run, stage_dir};

fn main() {
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
