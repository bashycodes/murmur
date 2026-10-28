//! Plain-text notes on disk: the Notes list, and the Scratchpad (= the
//! "Quick note", id [`SCRATCHPAD`]).
//!
//! [`NoteStore`] keeps one `<id>.txt` per note in `%APPDATA%\FlowClone\notes\`
//! plus `index.json` (pinned, created/updated). The text files are the truth:
//! a file without an index entry is still listed (times from the file), an
//! index entry without a file is dropped. [`migrate_scratchpad`] moves the
//! pre-Notes `FlowClone\scratchpad.txt` in. [`AutoSave`] sits in front of the
//! store: the editor window sends every change, the latest text per note is
//! written once the edits pause for the debounce time, and
//! [`AutoSave::flush_all`] writes whatever is left (window closed, app quit).
//! Time is injected (`now_ms` for the debounce, [`WallClock`] for the
//! created/updated stamps), so both are tested without sleeping.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The Scratchpad's note id (listed as "Quick note"; never deleted).
pub const SCRATCHPAD: &str = "scratchpad";
/// Write a note this long after its last edit.
pub const SAVE_DEBOUNCE_MS: u64 = 750;
/// Folder of the notes under `%APPDATA%\FlowClone`.
pub const NOTES_DIR: &str = "notes";
/// Pinned + created/updated per note id.
pub const INDEX_FILE: &str = "index.json";
/// Longest title shown in the list (characters).
const TITLE_MAX: usize = 80;

/// Ids become file names: lowercase ASCII letters, digits, `-` and `_`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `%APPDATA%\FlowClone` (next to `prefs.json`).
pub fn app_dir(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
}

/// `%APPDATA%\FlowClone\notes`.
pub fn default_dir(appdata: Option<&str>) -> PathBuf {
    app_dir(appdata).join(NOTES_DIR)
}

/// Move the pre-Notes Scratchpad (`<app_dir>/scratchpad.txt`) into the notes
/// folder. Nothing is ever overwritten: if the notes folder already has a
/// Scratchpad the old file is left where it is. `Ok(true)` = moved.
pub fn migrate_scratchpad(app_dir: &Path, notes_dir: &Path) -> Result<bool, String> {
    let old = app_dir.join(format!("{SCRATCHPAD}.txt"));
    let new = notes_dir.join(format!("{SCRATCHPAD}.txt"));
    if !old.is_file() || new.exists() {
        return Ok(false);
    }
    let err = |e: std::io::Error| format!("{}: {e}", old.display());
    std::fs::create_dir_all(notes_dir).map_err(err)?;
    if std::fs::rename(&old, &new).is_err() {
        // A locked file or another volume: copy, then drop the old one.
        std::fs::copy(&old, &new).map_err(err)?;
        let _ = std::fs::remove_file(&old);
    }
    Ok(true)
}

/// Milliseconds since the Unix epoch (created/updated stamps).
pub type WallClock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> WallClock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct IndexEntry {
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    created_ms: u64,
    #[serde(default)]
    updated_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    notes: BTreeMap<String, IndexEntry>,
}

/// One row of the Notes list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteMeta {
    pub id: String,
    /// First non-blank line ("Untitled" if none); "Quick note" for the
    /// Scratchpad.
    pub title: String,
    /// The Quick note's first line (its title is fixed), else "".
    pub preview: String,
    pub words: usize,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub pinned: bool,
    /// The Scratchpad (tray "Scratchpad…" opens it; cannot be deleted).
    pub quick: bool,
}

/// Whitespace-separated words, as the editor counts them.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > TITLE_MAX {
        let cut: String = line.chars().take(TITLE_MAX - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_string()
    }
}

/// A file name (no extension) from a note title: Windows-reserved
/// characters dropped, at most 60 characters, "note" if nothing is left.
pub fn export_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .filter(|c| !c.is_control() && !r#"<>:"/\|?*"#.contains(*c))
        .take(60)
        .collect();
    let s = s.trim().trim_matches('.').trim();
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "lpt1", "lpt2", "lpt3",
    ];
    if s.is_empty() || reserved.contains(&s.to_ascii_lowercase().as_str()) {
        "note".into()
    } else {
        s.to_string()
    }
}

#[derive(Clone)]
pub struct NoteStore {
    dir: PathBuf,
    clock: WallClock,
}

impl std::fmt::Debug for NoteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoteStore").field("dir", &self.dir).finish()
    }
}

impl NoteStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_clock(dir, system_clock())
    }

    pub fn with_clock(dir: impl Into<PathBuf>, clock: WallClock) -> Self {
        NoteStore {
            dir: dir.into(),
            clock,
        }
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

    /// Atomic write (tmp + rename): a crash never leaves half a note. Stamps
    /// the note's updated (and first time, created) time in the index.
    pub fn save(&self, id: &str, text: &str) -> Result<(), String> {
        let path = self.path(id)?;
        write_atomic(&self.dir, &path, text.as_bytes())?;
        let now = (self.clock)();
        self.update_index(|ix| {
            let e = ix.notes.entry(id.to_string()).or_default();
            if e.created_ms == 0 {
                e.created_ms = now;
            }
            e.updated_ms = now;
        })
    }

    /// A new empty note (its file exists at once, so it is listed); returns
    /// its id, `note-<ms>` (`-2`, `-3`… if that is taken).
    pub fn create(&self) -> Result<String, String> {
        let base = format!("note-{}", (self.clock)());
        let mut id = base.clone();
        let mut n = 1;
        while self.path(&id)?.exists() {
            n += 1;
            id = format!("{base}-{n}");
        }
        self.save(&id, "")?;
        Ok(id)
    }

    /// Remove the note's file and index entry. The Quick note is not deleted
    /// (Clear empties it).
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = self.path(id)?;
        if id == SCRATCHPAD {
            return Err("the Quick note cannot be deleted (use Clear)".into());
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        self.update_index(|ix| {
            ix.notes.remove(id);
        })
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), String> {
        let path = self.existing(id)?;
        let stamp = file_ms(&path).unwrap_or_else(|| (self.clock)());
        self.update_index(|ix| {
            let e = ix.notes.entry(id.to_string()).or_insert(IndexEntry {
                pinned: false,
                created_ms: stamp,
                updated_ms: stamp,
            });
            e.pinned = pinned;
        })
    }

    /// Every note whose text contains `query` (case-insensitive; blank =
    /// all), pinned first, then the most recently updated. The Quick note is
    /// always there (empty until first saved).
    pub fn list(&self, query: &str) -> Result<Vec<NoteMeta>, String> {
        let index = self.read_index();
        let mut ids: Vec<String> = vec![SCRATCHPAD.to_string()];
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for ent in rd.flatten() {
                    let name = ent.file_name();
                    let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".txt")) else {
                        continue;
                    };
                    if valid_id(id) && id != SCRATCHPAD && ent.path().is_file() {
                        ids.push(id.to_string());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", self.dir.display())),
        }
        let q = query.trim().to_lowercase();
        let mut out = Vec::new();
        for id in ids {
            let text = self.load(&id)?;
            if !q.is_empty() && !text.to_lowercase().contains(&q) {
                continue;
            }
            let mtime = file_ms(&self.path(&id)?).unwrap_or(0);
            let e = index.notes.get(&id).cloned().unwrap_or(IndexEntry {
                pinned: false,
                created_ms: mtime,
                updated_ms: mtime,
            });
            let quick = id == SCRATCHPAD;
            let line = first_line(&text);
            let (title, preview) = match (quick, line.is_empty()) {
                (true, _) => ("Quick note".to_string(), line),
                (false, true) => ("Untitled".to_string(), String::new()),
                (false, false) => (line, String::new()),
            };
            out.push(NoteMeta {
                words: word_count(&text),
                id,
                title,
                preview,
                created_ms: e.created_ms,
                updated_ms: e.updated_ms,
                pinned: e.pinned,
                quick,
            });
        }
        out.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_ms.cmp(&a.updated_ms))
                .then(a.id.cmp(&b.id))
        });
        Ok(out)
    }

    /// Write the note to `dest/<title>.<ext>` (`txt` or `md`), never over
    /// an existing file (`<title> (2).<ext>`…); returns the path.
    pub fn export(&self, id: &str, ext: &str, dest: &Path) -> Result<PathBuf, String> {
        if ext != "txt" && ext != "md" {
            return Err(format!("cannot export as .{ext}"));
        }
        self.existing(id)?;
        let text = self.load(id)?;
        let title = first_line(&text);
        let stem = export_stem(if title.is_empty() && id == SCRATCHPAD {
            "Quick note"
        } else {
            &title
        });
        let mut out = dest.join(format!("{stem}.{ext}"));
        let mut n = 1;
        while out.exists() {
            n += 1;
            out = dest.join(format!("{stem} ({n}).{ext}"));
        }
        std::fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(out)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The path of a note that exists (the Quick note always does).
    fn existing(&self, id: &str) -> Result<PathBuf, String> {
        let path = self.path(id)?;
        if id != SCRATCHPAD && !path.is_file() {
            return Err(format!("no note \"{id}\""));
        }
        Ok(path)
    }

    /// A missing or unreadable index is an empty one (the files still list).
    fn read_index(&self) -> Index {
        std::fs::read(self.dir.join(INDEX_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Change the index, dropping entries whose file is gone (stale).
    fn update_index(&self, f: impl FnOnce(&mut Index)) -> Result<(), String> {
        let mut ix = self.read_index();
        f(&mut ix);
        ix.notes
            .retain(|id, _| id == SCRATCHPAD || self.path(id).is_ok_and(|p| p.is_file()));
        let json = serde_json::to_vec_pretty(&ix).map_err(|e| e.to_string())?;
        write_atomic(&self.dir, &self.dir.join(INDEX_FILE), &json)
    }
}

/// tmp + rename in `dir` (created if missing).
fn write_atomic(dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(dir).map_err(err)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

fn file_ms(path: &Path) -> Option<u64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
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

    /// The Notes list, unsaved edits written first so it shows them.
    pub fn list(&mut self, query: &str) -> Result<Vec<NoteMeta>, String> {
        self.flush_all();
        self.store.list(query)
    }

    pub fn create(&mut self) -> Result<String, String> {
        self.store.create()
    }

    /// Delete the note; an edit still waiting for it is dropped, so a
    /// later tick cannot write the note back.
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        self.store.path(id)?;
        if id != SCRATCHPAD {
            self.pending.remove(id);
        }
        self.store.delete(id)
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<(), String> {
        self.store.set_pinned(id, pinned)
    }

    /// Export what the editor shows (its pending edit written first).
    pub fn export(&mut self, id: &str, ext: &str, dest: &Path) -> Result<PathBuf, String> {
        self.store.path(id)?;
        if let Some(Err(e)) = self.flush(id) {
            return Err(e);
        }
        self.store.export(id, ext, dest)
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
    fn notes_live_in_a_folder_next_to_prefs() {
        let s = NoteStore::new(default_dir(Some(r"C:\Users\u\AppData\Roaming")));
        let p = s.path(SCRATCHPAD).unwrap();
        assert!(
            p.ends_with("FlowClone/notes/scratchpad.txt")
                || p.ends_with(r"FlowClone\notes\scratchpad.txt"),
            "{}",
            p.display()
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

    /// A store whose wall clock the test moves.
    fn clocked(dir: &Path) -> (NoteStore, Arc<std::sync::atomic::AtomicU64>) {
        let t = Arc::new(std::sync::atomic::AtomicU64::new(1_000));
        let c = t.clone();
        let clock: WallClock = Arc::new(move || c.load(std::sync::atomic::Ordering::SeqCst));
        (NoteStore::with_clock(dir, clock), t)
    }

    fn set(t: &std::sync::atomic::AtomicU64, ms: u64) {
        t.store(ms, std::sync::atomic::Ordering::SeqCst);
    }

    fn ids(list: &[NoteMeta]) -> Vec<&str> {
        list.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn three_notes_survive_a_restart_next_to_the_quick_note() {
        let dir = tmp("three");
        let (s, t) = clocked(&dir);
        s.save(SCRATCHPAD, "quick thought").unwrap();
        let mut made = Vec::new();
        for (i, text) in ["Groceries\nmilk eggs", "Meeting notes", "Idea: a third"]
            .iter()
            .enumerate()
        {
            set(&t, 2_000 + i as u64 * 1_000);
            let id = s.create().unwrap();
            assert!(valid_id(&id), "{id}");
            s.save(&id, text).unwrap();
            made.push(id);
        }
        // Restarted: a new store over the same folder.
        let (s2, _) = clocked(&dir);
        let list = s2.list("").unwrap();
        assert_eq!(list.len(), 4);
        assert_eq!(
            ids(&list),
            [&made[2], &made[1], &made[0], SCRATCHPAD],
            "newest first"
        );
        assert_eq!(list[2].title, "Groceries");
        assert_eq!(list[2].words, 3);
        assert_eq!(list[2].created_ms, 2_000);
        assert_eq!(s2.load(&made[1]).unwrap(), "Meeting notes");
        let quick = &list[3];
        assert!(quick.quick);
        assert_eq!(quick.title, "Quick note");
        assert_eq!(quick.preview, "quick thought");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_note_is_listed_before_it_is_ever_saved_and_never_deleted() {
        let dir = tmp("quick");
        let s = NoteStore::new(&dir);
        let list = s.list("").unwrap();
        assert_eq!(ids(&list), [SCRATCHPAD]);
        assert_eq!(list[0].words, 0);
        assert!(s.delete(SCRATCHPAD).is_err());
        s.set_pinned(SCRATCHPAD, true).unwrap();
        assert!(s.list("").unwrap()[0].pinned);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_never_reuses_an_id() {
        let dir = tmp("create");
        let (s, _) = clocked(&dir); // same clock value every time
        let a = s.create().unwrap();
        let b = s.create().unwrap();
        let c = s.create().unwrap();
        assert_eq!(a, "note-1000");
        assert_eq!(b, "note-1000-2");
        assert_eq!(c, "note-1000-3");
        assert!(dir.join("note-1000-3.txt").is_file(), "listed at once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_filters_by_content_case_insensitively() {
        let dir = tmp("search");
        let s = NoteStore::new(&dir);
        let a = s.create().unwrap();
        s.save(&a, "Call the dentist\nabout Tuesday").unwrap();
        let b = s.create().unwrap();
        s.save(&b, "Shopping list").unwrap();
        s.save(SCRATCHPAD, "remember TUESDAY").unwrap();
        let hits = s.list("tuesday").unwrap();
        let mut got = ids(&hits);
        got.sort();
        let mut want = vec![a.as_str(), SCRATCHPAD];
        want.sort();
        assert_eq!(got, want, "body lines match too, not just the title");
        assert_eq!(ids(&s.list("  shopping ").unwrap()), [b.as_str()]);
        assert!(s.list("nothing like this").unwrap().is_empty());
        assert_eq!(s.list("").unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pinned_notes_come_first() {
        let dir = tmp("pin");
        let (s, t) = clocked(&dir);
        let old = s.create().unwrap();
        set(&t, 5_000);
        let new = s.create().unwrap();
        s.save(SCRATCHPAD, "x").unwrap();
        assert_eq!(ids(&s.list("").unwrap())[2], old.as_str());
        s.set_pinned(&old, true).unwrap();
        assert_eq!(ids(&s.list("").unwrap())[0], old.as_str());
        // Pin survives later edits and a restart.
        set(&t, 9_000);
        s.save(&new, "edited").unwrap();
        let (s2, _) = clocked(&dir);
        let list = s2.list("").unwrap();
        assert_eq!(ids(&list)[0], old.as_str());
        assert!(list[0].pinned);
        s2.set_pinned(&old, false).unwrap();
        assert_eq!(ids(&s2.list("").unwrap())[0], new.as_str());
        assert!(s2.set_pinned("note-missing", true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_file_and_its_index_entry() {
        let dir = tmp("delete");
        let s = NoteStore::new(&dir);
        let id = s.create().unwrap();
        s.save(&id, "bye").unwrap();
        s.set_pinned(&id, true).unwrap();
        s.delete(&id).unwrap();
        assert!(!dir.join(format!("{id}.txt")).exists());
        assert_eq!(ids(&s.list("").unwrap()), [SCRATCHPAD]);
        let index = std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
        assert!(!index.contains(&id), "{index}");
        assert!(s.delete(&id).is_ok(), "already gone is fine");
        assert!(s.delete("../prefs").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_without_index_entries_and_stale_entries_are_tolerated() {
        let dir = tmp("tolerant");
        std::fs::create_dir_all(&dir).unwrap();
        // Copied in by hand: no index entry.
        std::fs::write(dir.join("hand-made.txt"), "From Explorer").unwrap();
        // Not notes: bad ids, other extensions, tmp files.
        std::fs::write(dir.join("Bad Name.txt"), "x").unwrap();
        std::fs::write(dir.join("readme.md"), "x").unwrap();
        std::fs::write(dir.join("x.txt.tmp"), "x").unwrap();
        std::fs::create_dir_all(dir.join("folder.txt")).unwrap();
        // An index naming a note whose file is gone, plus junk fields.
        std::fs::write(
            dir.join(INDEX_FILE),
            r#"{"notes":{"gone":{"pinned":true,"created_ms":5,"updated_ms":5},
                "hand-made":{"future_field":1}},"version":9}"#,
        )
        .unwrap();
        let s = NoteStore::new(&dir);
        let list = s.list("").unwrap();
        let mut got = ids(&list);
        got.sort();
        assert_eq!(got, ["hand-made", SCRATCHPAD]);
        // The next write drops the stale entry.
        s.save(SCRATCHPAD, "x").unwrap();
        let index = std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
        assert!(!index.contains("gone"), "{index}");
        // A corrupt index: the notes still list.
        std::fs::write(dir.join(INDEX_FILE), "{not json").unwrap();
        assert_eq!(s.list("explorer").unwrap()[0].id, "hand-made");
        let hand = s.list("explorer").unwrap().remove(0);
        assert!(hand.updated_ms > 0, "time from the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_scratchpad_moves_into_the_notes_folder() {
        let app = tmp("migrate");
        let notes = app.join(NOTES_DIR);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("scratchpad.txt"), "kept from 0.6").unwrap();
        assert!(migrate_scratchpad(&app, &notes).unwrap());
        assert!(!app.join("scratchpad.txt").exists());
        let s = NoteStore::new(&notes);
        assert_eq!(s.load(SCRATCHPAD).unwrap(), "kept from 0.6");
        assert_eq!(s.list("").unwrap()[0].preview, "kept from 0.6");
        assert!(!migrate_scratchpad(&app, &notes).unwrap(), "only once");
        // Both exist (old copy restored by hand): nothing is overwritten.
        std::fs::write(app.join("scratchpad.txt"), "older").unwrap();
        assert!(!migrate_scratchpad(&app, &notes).unwrap());
        assert_eq!(s.load(SCRATCHPAD).unwrap(), "kept from 0.6");
        assert!(app.join("scratchpad.txt").exists());
        // Nothing to migrate (fresh install).
        let fresh = tmp("migrate-fresh");
        assert!(!migrate_scratchpad(&fresh, &fresh.join(NOTES_DIR)).unwrap());
        assert!(!fresh.exists());
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn export_writes_txt_or_md_named_after_the_title_without_overwriting() {
        let dir = tmp("export");
        let dest = tmp("export-dest");
        std::fs::create_dir_all(&dest).unwrap();
        let mut a = AutoSave::new(NoteStore::new(&dir), 750);
        let id = a.create().unwrap();
        a.edit(&id, "Plan: Q3/Q4?\nship it", 0).unwrap();
        let p = a.export(&id, "md", &dest).unwrap();
        assert_eq!(p.file_name().unwrap(), "Plan Q3Q4.md");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "Plan: Q3/Q4?\nship it",
            "the unsaved edit is exported"
        );
        let p2 = a.export(&id, "md", &dest).unwrap();
        assert_eq!(p2.file_name().unwrap(), "Plan Q3Q4 (2).md");
        let p3 = a.export(&id, "txt", &dest).unwrap();
        assert_eq!(p3.file_name().unwrap(), "Plan Q3Q4.txt");
        assert!(a.export(&id, "exe", &dest).is_err());
        assert!(a.export("../prefs", "txt", &dest).is_err());
        assert!(a.export("note-missing", "txt", &dest).is_err());
        let q = a.export(SCRATCHPAD, "txt", &dest).unwrap();
        assert_eq!(q.file_name().unwrap(), "Quick note.txt");
        assert_eq!(export_stem("..."), "note");
        assert_eq!(export_stem("CON"), "note");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn deleting_drops_a_pending_edit_so_it_is_not_written_back() {
        let dir = tmp("del-pending");
        let mut a = AutoSave::new(NoteStore::new(&dir), 750);
        let id = a.create().unwrap();
        a.edit(&id, "typed", 0).unwrap();
        a.delete(&id).unwrap();
        assert!(a.flush_all().is_empty());
        assert!(a.tick(10_000).is_empty());
        assert!(!dir.join(format!("{id}.txt")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_shows_unsaved_edits() {
        let dir = tmp("list-pending");
        let mut a = AutoSave::new(NoteStore::new(&dir), 750);
        a.edit(SCRATCHPAD, "not yet written", 0).unwrap();
        let list = a.list("written").unwrap();
        assert_eq!(list[0].preview, "not yet written");
        assert!(!a.has_pending());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_first_lines_are_cut_for_the_title() {
        let long = "word ".repeat(40);
        let t = first_line(&format!("\n\n  {long}\nsecond"));
        assert_eq!(t.chars().count(), TITLE_MAX);
        assert!(t.ends_with('…'));
        assert_eq!(first_line("  \n\t\n"), "");
    }
}
