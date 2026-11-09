//! D83: the one-time pick-up of the data folder the app used before the
//! Murmur rename.
//!
//! The Windows app has no export/backup of its own, so on the first start
//! of the renamed app `%APPDATA%\FlowClone` (prefs, the transcript history
//! `history.sqlite` with its WAL, dictionary, notes, insights, sounds,
//! models) is **copied** to `%APPDATA%\Murmur`. Saved API keys are not
//! (Credential Manager names changed with the rename): enter them again. The
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
    let mut n = Counts::default();
    copy_tree(&old, &new, false, &mut n)?;
    mark(&new, &format!("copied from {}", old.display()))?;
    Ok(Outcome::Copied {
        files: n.copied,
        linked: n.linked,
    })
}

fn mark(new: &Path, what: &str) -> io::Result<()> {
    fs::create_dir_all(new)?;
    fs::write(
        new.join(MARKER),
        format!("The data folder of the app before its rename to Murmur (D83): {what}.\n"),
    )
}

#[derive(Default)]
struct Counts {
    copied: usize,
    linked: usize,
}

/// `link`: inside `models`, where hard links are safe: the app never writes
/// a finished model file in place (downloads and imports write `<name>.part`
/// and rename it over the target, `model_download.rs` / `model_import.rs`),
/// so a write in either app replaces its own link and never changes the
/// other app's file. A `.part` left by a cut-off download is appended to on
/// resume, so it is copied, not linked.
fn copy_tree(from: &Path, to: &Path, link: bool, n: &mut Counts) -> io::Result<()> {
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
            let partial = src.extension().is_some_and(|x| x == "part");
            if link && !partial && fs::hard_link(&src, &dst).is_ok() {
                n.linked += 1;
            } else {
                fs::copy(&src, &dst)?;
                n.copied += 1;
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
        fs::write(old.join("models/parakeet/decoder.onnx.part"), b"half").unwrap();
    }

    #[test]
    fn copies_the_old_folder_once_and_leaves_it_untouched() {
        let a = tmp("copy");
        old_install(&a);
        let got = copy_once(&a).unwrap();
        assert_eq!(
            got,
            Outcome::Copied {
                files: 4,
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

    /// The point of the copy: transcripts dictated in the old app are in the
    /// new app's History. The old database is still open (the old app
    /// running, rows only in the WAL) while it is copied.
    #[test]
    fn transcripts_of_the_old_app_show_in_the_new_history() {
        use crate::history::{Entry, History, FILE_NAME};
        use crate::prefs::Retention;
        let a = tmp("history");
        let old = a.join(OLD_APP_DIR);
        fs::create_dir_all(&old).unwrap();
        let h = History::open(&old.join(FILE_NAME)).unwrap();
        for (ts, text) in [(1, "an invented first line"), (2, "a made-up second line")] {
            let e = Entry {
                ts,
                app: "notepad.exe".into(),
                raw: text.into(),
                pasted: Some(text.into()),
                ..Entry::default()
            };
            h.record(&e, Retention::Keep, 10).unwrap();
        }
        assert!(matches!(copy_once(&a).unwrap(), Outcome::Copied { .. }));
        drop(h);
        let new = History::open(&a.join(crate::prefs::APP_DIR).join(FILE_NAME)).unwrap();
        let got: Vec<String> = new
            .list("", 10, Retention::Keep, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.raw)
            .collect();
        assert_eq!(got, ["a made-up second line", "an invented first line"]);
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
