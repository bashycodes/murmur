//! Bakes the git commit into the crate (FLOW_GIT_SHA / FLOW_GIT_DIRTY /
//! FLOW_BUILD / FLOW_FULL) and fails the build when the workspace version
//! drifts from the repo's VERSION file (D26, docs/versioning.md).
//!
//! The env vars FLOW_GIT_SHA and FLOW_GIT_DIRTY (1/true) override git, e.g.
//! for a build from a source tarball. The dirty flag covers tracked files and
//! is refreshed when HEAD, the index, VERSION or any Rust source under
//! crates/ or apps/flow-windows changes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

include!("src/describe.rs");

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn rerun(p: &Path) {
    if p.exists() {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("..").join("..");
    let version = env::var("CARGO_PKG_VERSION").unwrap();

    let vfile = root.join("VERSION");
    rerun(&vfile);
    if let Ok(v) = fs::read_to_string(&vfile) {
        let v = v.trim();
        if v != version {
            panic!(
                "VERSION says {v} but the workspace version is {version}: run tools/release.sh \
                 (never edit the copies by hand) or tools/check-version.sh to see every copy"
            );
        }
    }

    println!("cargo:rerun-if-env-changed=FLOW_GIT_SHA");
    println!("cargo:rerun-if-env-changed=FLOW_GIT_DIRTY");
    let sha = env::var("FLOW_GIT_SHA")
        .ok()
        .or_else(|| git(&root, &["rev-parse", "--short=7", "HEAD"]))
        .unwrap_or_default();
    let dirty = match env::var("FLOW_GIT_DIRTY") {
        Ok(v) => v == "1" || v == "true",
        Err(_) => {
            !sha.is_empty()
                && git(&root, &["status", "--porcelain", "--untracked-files=no"])
                    .is_some_and(|s| !s.is_empty())
        }
    };

    if let Some(gd) = git(&root, &["rev-parse", "--absolute-git-dir"]) {
        let gd = PathBuf::from(gd);
        rerun(&gd.join("HEAD"));
        rerun(&gd.join("index"));
    }
    if let Some(cd) = git(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) {
        let cd = PathBuf::from(cd);
        rerun(&cd.join("packed-refs"));
        if let Some(r) = git(&root, &["symbolic-ref", "-q", "HEAD"]) {
            rerun(&cd.join(r));
        }
    }
    // Edits to sources flip the dirty flag without touching the index.
    for d in [
        "Cargo.toml",
        "Cargo.lock",
        "crates",
        "apps/flow-windows/Cargo.toml",
        "apps/flow-windows/src",
        "apps/flow-windows/ui/src",
    ] {
        rerun(&root.join(d));
    }

    let (build, full) = describe(&version, &sha, dirty);
    println!("cargo:rustc-env=FLOW_GIT_SHA={sha}");
    println!(
        "cargo:rustc-env=FLOW_GIT_DIRTY={}",
        if dirty { "1" } else { "0" }
    );
    println!("cargo:rustc-env=FLOW_BUILD={build}");
    println!("cargo:rustc-env=FLOW_FULL={full}");
}
