//! Compile tests/c/smoke.c against include/flow_core.h and the staticlib,
//! proving the header matches the exported ABI. Skips without `cc`.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn c_program_links_and_runs() {
    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("SKIP: no cc");
        return;
    }
    let st = Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "flow-ffi", "--lib"])
        .status()
        .unwrap();
    assert!(st.success());
    let exe = std::env::current_exe().unwrap();
    let lib_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("flow_smoke");
    let out = Command::new("cc")
        .arg(dir.join("tests/c/smoke.c"))
        .arg("-I")
        .arg(dir.join("include"))
        .arg(lib_dir.join("libflow_ffi.a"))
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
