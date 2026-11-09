//! Compile tests/c/smoke.c against include/murmur_core.h and the staticlib,
//! proving the header matches the exported ABI. Skips without `cc`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Builds the default-feature staticlib (smoke.c asserts the default build's
/// capabilities) for the profile and target triple this test binary was built
/// with, into the same target dir, and returns the directory that holds it.
/// The test binary sits at `<target>/[<triple>/]<profile-dir>/deps/`.
///
/// `cargo test` builds only the rlib, never the staticlib, so the nested build
/// must match the test's profile: building the default (dev) profile and then
/// looking next to a `--release` test binary linked whatever release
/// `libmurmur_ffi.a` an older build had left there (or none at all).
fn build_staticlib() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let lib_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
    let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).parent().unwrap();
    let profile = match lib_dir.file_name().unwrap().to_str().unwrap() {
        "debug" => "dev",
        other => other, // "release" and custom profiles are named after their dir
    };
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args([
        "build",
        "-q",
        "-p",
        "murmur-ffi",
        "--lib",
        "--profile",
        profile,
    ])
    .arg("--target-dir")
    .arg(target_dir);
    let triple_dir = lib_dir.parent().unwrap();
    if triple_dir != target_dir {
        cmd.arg("--target").arg(triple_dir.file_name().unwrap());
    }
    let st = cmd.status().unwrap();
    assert!(st.success(), "{cmd:?} failed");
    lib_dir
}

#[test]
fn c_program_links_and_runs() {
    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("SKIP: no cc");
        return;
    }
    let lib_dir = build_staticlib();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("murmur_smoke");
    let out = Command::new("cc")
        .arg(dir.join("tests/c/smoke.c"))
        .arg("-I")
        .arg(dir.join("include"))
        .arg(lib_dir.join("libmurmur_ffi.a"))
        .args(["-lpthread", "-ldl", "-lm", "-lstdc++", "-o"])
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().unwrap();
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
