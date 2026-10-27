//! Plain-text notes on disk (the Scratchpad today; a Notes list later).
//!
//! [`NoteStore`] loads and saves a note by id as `<dir>/<id>.txt`
//! (`%APPDATA%\FlowClone\scratchpad.txt` for [`SCRATCHPAD`]). [`AutoSave`]
//! sits in front of it: the editor window sends every change, the latest
//! text per note is written once the edits pause for the debounce time, and
//! [`AutoSave::flush_all`] writes whatever is left (window closed, app quit).
//! Time is injected (`now_ms`), so the debounce is tested without sleeping.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The Scratchpad's note id.
pub const SCRATCHPAD: &str = "scratchpad";
/// Write a note this long after its last edit.
pub const SAVE_DEBOUNCE_MS: u64 = 750;

/// Ids become file names: lowercase ASCII letters, digits, `-` and `_`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `%APPDATA%\FlowClone` (next to `prefs.json`).
pub fn default_dir(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
}

#[derive(Debug, Clone)]
pub struct NoteStore {
    dir: PathBuf,
}

impl NoteStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        NoteStore { dir: dir.into() }
    }

    pub fn path(&self, id: &str) -> Result<PathBuf, String> {
        if valid_id(id) {
            Ok(self.dir.join(format!("{id}.txt")))
        } else {
            Err(format!("\"{id}\" is not a note id"))
        }
    }

    /// The note's text; a note never saved is empty.
    pub fn load(&self, id: &str) -> Result<String, String> {
        let path = self.path(id)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Atomic write (tmp + rename): a crash never leaves half a note.
    pub fn save(&self, id: &str, text: &str) -> Result<(), String> {
        let path = self.path(id)?;
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        std::fs::create_dir_all(&self.dir).map_err(err)?;
        let tmp = path.with_extension("txt.tmp");
        std::fs::write(&tmp, text).map_err(err)?;
        std::fs::rename(&tmp, &path).map_err(err)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Debounced writes in front of a [`NoteStore`].
#[derive(Debug)]
pub struct AutoSave {
    store: NoteStore,
    debounce_ms: u64,
    /// id → (latest text, time of the last edit).
    pending: BTreeMap<String, (String, u64)>,
}

impl AutoSave {
    pub fn new(store: NoteStore, debounce_ms: u64) -> Self {
        AutoSave {
            store,
            debounce_ms,
            pending: BTreeMap::new(),
        }
    }

    /// The editor changed the note; written by a later [`Self::tick`].
    pub fn edit(&mut self, id: &str, text: &str, now_ms: u64) -> Result<(), String> {
        self.store.path(id)?;
        self.pending
            .insert(id.to_string(), (text.to_string(), now_ms));
        Ok(())
    }

    /// What the editor should show: an unsaved edit wins over the file.
    pub fn load(&self, id: &str) -> Result<String, String> {
        match self.pending.get(id) {
            Some((text, _)) => Ok(text.clone()),
            None => self.store.load(id),
        }
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Write the notes whose last edit is at least the debounce time old.
    /// A failed write stays pending (retried by a later tick or flush).
    pub fn tick(&mut self, now_ms: u64) -> Vec<(String, Result<(), String>)> {
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, (_, at))| now_ms.saturating_sub(*at) >= self.debounce_ms)
            .map(|(id, _)| id.clone())
            .collect();
        due.into_iter().map(|id| self.write(id)).collect()
    }

    /// Write one note now (its window closed).
    pub fn flush(&mut self, id: &str) -> Option<Result<(), String>> {
        self.pending
            .contains_key(id)
            .then(|| self.write(id.to_string()).1)
    }

    /// Write every pending note now (app exit).
    pub fn flush_all(&mut self) -> Vec<(String, Result<(), String>)> {
        let ids: Vec<String> = self.pending.keys().cloned().collect();
        ids.into_iter().map(|id| self.write(id)).collect()
    }

    fn write(&mut self, id: String) -> (String, Result<(), String>) {
        let Some((text, at)) = self.pending.remove(&id) else {
            return (id, Ok(()));
        };
        let r = self.store.save(&id, &text);
        if r.is_err() {
            self.pending.insert(id.clone(), (text, at));
        }
        (id, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "flow-notes-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn scratchpad_lives_next_to_prefs() {
        let s = NoteStore::new(default_dir(Some(r"C:\Users\u\AppData\Roaming")));
        let p = s.path(SCRATCHPAD).unwrap();
        assert!(
            p.ends_with("FlowClone/scratchpad.txt") || p.ends_with(r"FlowClone\scratchpad.txt")
        );
    }

    #[test]
    fn ids_are_file_name_safe() {
        for ok in ["scratchpad", "note-1", "a_b", "x"] {
            assert!(valid_id(ok), "{ok}");
        }
        let long = "x".repeat(65);
        for bad in [
            "", "../prefs", "a/b", r"a\b", "Prefs", "a.txt", "a b", &long,
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert!(NoteStore::new("d").path("../x").is_err());
    }

    #[test]
    fn missing_note_is_empty_and_save_round_trips() {
        let dir = tmp("rt");
        let s = NoteStore::new(&dir);
        assert_eq!(s.load(SCRATCHPAD).unwrap(), "");
        let text = "Grüße\r\nline two\n\nemoji 🎙";
        s.save(SCRATCHPAD, text).unwrap();
        assert_eq!(s.load(SCRATCHPAD).unwrap(), text);
        // A new store over the same folder (the app restarted) sees it.
        assert_eq!(NoteStore::new(&dir).load(SCRATCHPAD).unwrap(), text);
        assert!(!dir.join("scratchpad.txt.tmp").exists());
        s.save(SCRATCHPAD, "").unwrap();
        assert_eq!(s.load(SCRATCHPAD).unwrap(), "", "cleared stays cleared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_are_written_once_they_pause() {
        let dir = tmp("debounce");
        let mut a = AutoSave::new(NoteStore::new(&dir), 750);
        a.edit(SCRATCHPAD, "h", 0).unwrap();
        a.edit(SCRATCHPAD, "he", 500).unwrap();
        assert!(a.tick(1000).is_empty(), "only 500 ms since the last edit");
        assert!(!dir.join("scratchpad.txt").exists());
        assert_eq!(a.load(SCRATCHPAD).unwrap(), "he", "unsaved edit shown");
        a.edit(SCRATCHPAD, "hello", 1100).unwrap();
        assert!(a.tick(1849).is_empty());
        let done = a.tick(1850);
        assert_eq!(done.len(), 1);
        assert!(done[0].1.is_ok());
        assert_eq!(NoteStore::new(&dir).load(SCRATCHPAD).unwrap(), "hello");
        assert!(!a.has_pending());
        assert!(a.tick(5000).is_empty(), "nothing new to write");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_and_exit_write_pending_edits_at_once() {
        let dir = tmp("flush");
        let mut a = AutoSave::new(NoteStore::new(&dir), 750);
        assert!(a.flush(SCRATCHPAD).is_none(), "nothing pending");
        a.edit(SCRATCHPAD, "typed then closed", 10).unwrap();
        a.edit("note-2", "other", 10).unwrap();
        assert!(a.flush(SCRATCHPAD).unwrap().is_ok());
        assert_eq!(
            NoteStore::new(&dir).load(SCRATCHPAD).unwrap(),
            "typed then closed"
        );
        assert!(a.has_pending(), "note-2 still waiting");
        assert_eq!(a.flush_all().len(), 1);
        assert_eq!(NoteStore::new(&dir).load("note-2").unwrap(), "other");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_write_stays_pending() {
        let dir = tmp("fail");
        // A file where the folder should be: every save fails.
        std::fs::write(&dir, "not a folder").unwrap();
        let mut a = AutoSave::new(NoteStore::new(&dir), 0);
        a.edit(SCRATCHPAD, "keep me", 0).unwrap();
        let r = a.tick(1);
        assert!(r[0].1.is_err());
        assert_eq!(a.load(SCRATCHPAD).unwrap(), "keep me");
        assert!(a.flush_all()[0].1.is_err());
        std::fs::remove_file(&dir).unwrap();
        assert!(a.flush_all()[0].1.is_ok(), "retried once the disk is back");
        assert_eq!(NoteStore::new(&dir).load(SCRATCHPAD).unwrap(), "keep me");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_id_is_rejected_on_edit() {
        let mut a = AutoSave::new(NoteStore::new(tmp("bad")), 750);
        assert!(a.edit("../prefs", "x", 0).is_err());
        assert!(!a.has_pending());
    }
}
