//! D83: the one-time pick-up of the data folder the app used before the
//! Murmur rename.
//!
//! The Windows app has no export/backup of its own, so on the first start
//! of the renamed app `%APPDATA%\FlowClone` (prefs, history, dictionary,
//! notes, insights, sounds, models) is **copied** to `%APPDATA%\Murmur`. The
//! old folder is never changed or deleted, so the old app keeps working
//! until it is uninstalled. Model files are hard-linked where the volume
//! allows it (same bytes, no second copy of several GB; they are only ever
//! read), everything else is a real copy so the two apps never write into
//! each other's files. A marker file in the new folder stops it repeating.

use std::fs;
use std::io;
use std::path::Path;

/// The data folder's name under `%APPDATA%` before D83.
pub const OLD_APP_DIR: &str = "FlowClone";
/// Written into the new folder once the pick-up has run (or was not needed
/// because the new folder already had data).
pub const MARKER: &str = "old-data-copied.txt";
/// The new folder already holds this when the user has run the renamed app.
const PREFS: &str = crate::prefs::FILE_NAME;

/// What [`copy_once`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Copied: `files` copied, `linked` hard-linked.
    Copied { files: usize, linked: usize },
    /// The marker was there: done on an earlier start.
    AlreadyDone,
    /// The new folder already had settings: nothing copied (marked).
    NewFolderInUse,
    /// No old folder: a fresh install.
    NoOldData,
}

/// Copy `<appdata>\FlowClone` to `<appdata>\Murmur` once (see the module
/// docs). Errors leave no marker, so the next start tries again; files
/// already copied are overwritten then.
pub fn copy_once(appdata: &Path) -> io::Result<Outcome> {
    let old = appdata.join(OLD_APP_DIR);
    let new = appdata.join(crate::prefs::APP_DIR);
    if new.join(MARKER).exists() {
        return Ok(Outcome::AlreadyDone);
    }
    if !old.is_dir() {
        return Ok(Outcome::NoOldData);
    }
    if new.join(PREFS).exists() {
        mark(&new, "not copied: this folder already had settings")?;
        return Ok(Outcome::NewFolderInUse);
    }
    let mut n = (0, 0);
    copy_tree(&old, &new, false, &mut n)?;
    mark(&new, &format!("copied from {}", old.display()))?;
    Ok(Outcome::Copied {
        files: n.0,
        linked: n.1,
    })
}

fn mark(new: &Path, what: &str) -> io::Result<()> {
    fs::create_dir_all(new)?;
    fs::write(
        new.join(MARKER),
        format!("The data folder of the app before its rename to Murmur (D83): {what}.\n"),
    )
}

/// `link`: inside `models` (hard links allowed). `n`: (copied, linked).
fn copy_tree(from: &Path, to: &Path, link: bool, n: &mut (usize, usize)) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_tree(&src, &dst, link || e.file_name() == "models", n)?;
        } else if ty.is_file() {
            if dst.exists() {
                fs::remove_file(&dst)?;
            }
            if link && fs::hard_link(&src, &dst).is_ok() {
                n.1 += 1;
            } else {
                fs::copy(&src, &dst)?;
                n.0 += 1;
            }
        }
        // Symlinks and other entries: the app never makes them; skipped.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "murmur-old-data-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn old_install(appdata: &Path) {
        let old = appdata.join(OLD_APP_DIR);
        fs::create_dir_all(old.join("models/parakeet")).unwrap();
        fs::create_dir_all(old.join("notes")).unwrap();
        fs::write(old.join(PREFS), r#"{"prefs_rev":5}"#).unwrap();
        fs::write(old.join("history.jsonl"), "{\"text\":\"a made-up line\"}\n").unwrap();
        fs::write(old.join("notes/scratchpad.txt"), "synthetic note").unwrap();
        fs::write(old.join("models/parakeet/encoder.onnx"), b"weights").unwrap();
    }

    #[test]
    fn copies_the_old_folder_once_and_leaves_it_untouched() {
        let a = tmp("copy");
        old_install(&a);
        let got = copy_once(&a).unwrap();
        assert_eq!(
            got,
            Outcome::Copied {
                files: 3,
                linked: 1
            }
        );
        let new = a.join(crate::prefs::APP_DIR);
        assert_eq!(
            fs::read_to_string(new.join(PREFS)).unwrap(),
            r#"{"prefs_rev":5}"#
        );
        assert_eq!(
            fs::read_to_string(new.join("notes/scratchpad.txt")).unwrap(),
            "synthetic note"
        );
        assert_eq!(
            fs::read(new.join("models/parakeet/encoder.onnx")).unwrap(),
            b"weights"
        );
        assert!(new.join(MARKER).exists());
        // The old folder is still all there.
        let old = a.join(OLD_APP_DIR);
        assert!(old.join(PREFS).exists() && old.join("models/parakeet/encoder.onnx").exists());

        // A setting changed in the new app is not overwritten on the next start.
        fs::write(new.join(PREFS), r#"{"prefs_rev":6}"#).unwrap();
        assert_eq!(copy_once(&a).unwrap(), Outcome::AlreadyDone);
        assert_eq!(
            fs::read_to_string(new.join(PREFS)).unwrap(),
            r#"{"prefs_rev":6}"#
        );
        // ...and the copy is independent: the old app's file is unchanged.
        assert_eq!(
            fs::read_to_string(old.join(PREFS)).unwrap(),
            r#"{"prefs_rev":5}"#
        );
        let _ = fs::remove_dir_all(&a);
    }

    #[test]
    fn a_fresh_install_copies_nothing_and_checks_again_next_time() {
        let a = tmp("fresh");
        assert_eq!(copy_once(&a).unwrap(), Outcome::NoOldData);
        assert!(!a.join(crate::prefs::APP_DIR).join(MARKER).exists());
        let _ = fs::remove_dir_all(&a);
    }

    #[test]
    fn a_new_folder_already_in_use_is_never_overwritten() {
        let a = tmp("inuse");
        old_install(&a);
        let new = a.join(crate::prefs::APP_DIR);
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join(PREFS), "{}").unwrap();
        assert_eq!(copy_once(&a).unwrap(), Outcome::NewFolderInUse);
        assert_eq!(fs::read_to_string(new.join(PREFS)).unwrap(), "{}");
        assert!(!new.join("history.jsonl").exists());
        assert_eq!(copy_once(&a).unwrap(), Outcome::AlreadyDone);
        let _ = fs::remove_dir_all(&a);
    }
}
