//! Manual model import (air-gapped): install a catalogue model from a
//! folder, a `.zip` of its files, or (single-file models) the file itself,
//! with no network call anywhere on the path.
//!
//! * Only [`crate::model_download::CATALOG`] entries: every file must have the
//!   pinned size and SHA-256 the downloader checks. A source with none of the
//!   model's files is refused ("custom models come from the server library").
//! * Files are found by name anywhere in the folder / zip (a zip often has a
//!   top folder), else by their exact pinned size (a renamed copy); the hash
//!   decides either way. A model's files come from one folder where they can
//!   (the one holding most of them, its own name breaking a tie).
//! * All or nothing: every needed file is copied to `<name>.part` and hashed
//!   on the way (`model_download::copy_verified`, the downloader's code);
//!   only when every one matches are they renamed into place. Any mismatch
//!   deletes all `.part` files, so nothing is installed.
//! * A file already in place and intact is not copied again (so importing
//!   just `ggml-tiny-q5_1.bin` completes a Hebrew model that lacks the
//!   language-ID extra). Extras are optional: missing ones are skipped.
//! * Zip entries are streamed straight into the `.part` path; their names
//!   never become paths (no zip-slip).
//!
//! **A folder of models** ([`scan_folder`], [`import_folder`]; setup's "Use
//! model files on this PC" and Settings' "Import from folder…", one code
//! path): the folder is searched once for every catalogue model, by the same
//! rules, and sorted into the three things setup needs: an English (default)
//! speech model, a Hebrew model and the language-ID model (D34). Any layout
//! works: the models library (`<catalogue id>/<files>`, `whisper-tiny-lid/`),
//! the app's own models folder, renamed folders or all files in one folder.
//! A file serves one model at most; a size several catalogue files share
//! (two decoders, the two turbo models) is hashed during the scan so it is
//! not taken for the wrong one. A same-named file of another size, a model
//! with files missing, and model-like files nothing claims are reported, not
//! skipped. Each picked model then installs as above (all or nothing per
//! model, the others unaffected). Fingerprints worked out during a scan are
//! kept (by path, size and modification time), so the import's own scan
//! doesn't read those files again. A folder bigger than [`LIMITS`] is
//! searched only up to them, and the scan says so.

use crate::model_download::{
    copy_verified, intact, part_path, CatalogEntry, ModelEngine, ModelFile, Progress, State, Stop,
};
use crate::prefs::{AsrEngineChoice, Prefs};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// How deep a picked folder is searched, and how many files at most.
#[derive(Debug, Clone, Copy)]
struct Limits {
    depth: usize,
    files: usize,
}

const LIMITS: Limits = Limits {
    depth: 4,
    files: 10_000,
};

/// A file the source offers.
#[derive(Debug, Clone)]
struct Candidate {
    /// Base name (no folders).
    name: String,
    size: u64,
    /// Where it is under the picked folder (`/`-separated; a zip entry's name).
    rel: String,
    at: Loc,
}

impl Candidate {
    /// The folder it is in, relative to the picked one ("" = at its top).
    fn dir(&self) -> &str {
        self.rel.rsplit_once('/').map_or("", |(d, _)| d)
    }
}

#[derive(Debug, Clone)]
enum Loc {
    Path(PathBuf),
    /// Index into the zip.
    Zip(usize),
}

/// What the user picked.
enum Source {
    Files(Vec<Candidate>),
    Zip(zip::ZipArchive<std::fs::File>, Vec<Candidate>),
}

impl Source {
    fn open(src: &Path) -> Result<Source, String> {
        let meta =
            std::fs::metadata(src).map_err(|e| format!("could not open {}: {e}", src.display()))?;
        if meta.is_dir() {
            return Ok(Source::Files(list_folder(src, LIMITS)?.0));
        }
        if is_zip(src) {
            let f = std::fs::File::open(src)
                .map_err(|e| format!("could not open {}: {e}", src.display()))?;
            let mut z = zip::ZipArchive::new(f)
                .map_err(|e| format!("{} is not a readable zip: {e}", src.display()))?;
            let mut out = vec![];
            for i in 0..z.len().min(LIMITS.files) {
                let e = z
                    .by_index_raw(i)
                    .map_err(|e| format!("{} is not a readable zip: {e}", src.display()))?;
                if e.is_dir() {
                    continue;
                }
                let rel = e.name().replace('\\', "/");
                let name = rel.rsplit('/').next().unwrap_or_default().to_string();
                if !name.is_empty() {
                    out.push(Candidate {
                        name,
                        size: e.size(),
                        rel,
                        at: Loc::Zip(i),
                    });
                }
            }
            return Ok(Source::Zip(z, out));
        }
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Source::Files(vec![Candidate {
            rel: name.clone(),
            name,
            size: meta.len(),
            at: Loc::Path(src.to_path_buf()),
        }]))
    }

    fn candidates(&self) -> &[Candidate] {
        match self {
            Source::Files(c) | Source::Zip(_, c) => c,
        }
    }
}

/// A zip by its magic (`PK\x03\x04`, or `PK\x05\x06` for an empty one).
fn is_zip(p: &Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(p)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && (head == *b"PK\x03\x04" || head == *b"PK\x05\x06")
}

/// The files in folder `src`, up to `lim`; true when there was more.
fn list_folder(src: &Path, lim: Limits) -> Result<(Vec<Candidate>, bool), String> {
    if !src.is_dir() {
        return Err(format!("{} is not a folder", src.display()));
    }
    let mut out = vec![];
    let mut cut = false;
    walk(src, "", 0, lim, &mut out, &mut cut)?;
    Ok((out, cut))
}

/// `rel`: `dir` relative to the picked folder ("" for the folder itself).
/// `cut` is set when something was left out for `lim`.
fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    lim: Limits,
    out: &mut Vec<Candidate>,
    cut: &mut bool,
) -> Result<(), String> {
    let rd =
        std::fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    let mut ents: Vec<_> = rd.flatten().collect();
    // The same folder gives the same scan on every PC.
    ents.sort_by_key(|e| e.file_name());
    for ent in ents {
        if out.len() >= lim.files {
            *cut = true;
            break;
        }
        // file_type() does not follow symlinks: a linked folder is skipped.
        let Ok(ft) = ent.file_type() else { continue };
        let p = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        let sub = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if ft.is_dir() {
            if depth + 1 < lim.depth {
                walk(&p, &sub, depth + 1, lim, out, cut)?;
            } else {
                *cut = true;
            }
        } else if let Ok(m) = std::fs::metadata(&p) {
            // Our own leftovers are never a source.
            if m.is_file() && !name.ends_with(".part") {
                out.push(Candidate {
                    name,
                    size: m.len(),
                    rel: sub,
                    at: Loc::Path(p),
                });
            }
        }
    }
    Ok(())
}

/// How a catalogue file was looked for in the source (index into its files).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Found(usize),
    /// Only a same-named file of another size.
    WrongSize(usize),
    /// A file of the right size whose fingerprint differs (checked only for
    /// sizes several catalogue files share, [`Matcher::shared`]).
    WrongHash(usize),
    None,
}

/// A scan ended early (a newer one started, or the import was cancelled).
#[derive(Debug)]
struct Stopped;

/// A file as its fingerprint was worked out: path, size, modification time.
type HashKey = (PathBuf, u64, Option<SystemTime>);

/// Fingerprints already worked out. The import scans its folder again and
/// finds the scan's ones here, so a big file of a shared size is read once to
/// tell which model it is and once more to copy it (checked on the way).
fn hash_cache() -> &'static Mutex<HashMap<HashKey, String>> {
    static CACHE: OnceLock<Mutex<HashMap<HashKey, String>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// `p`'s SHA-256 (`None`: unreadable), from the cache or read now (`reading`
/// is called first); `stop` is asked between chunks.
fn fingerprint(
    p: &Path,
    reading: &mut dyn FnMut(),
    stop: &dyn Fn() -> bool,
) -> Result<Option<String>, Stopped> {
    let Ok(meta) = std::fs::metadata(p) else {
        return Ok(None);
    };
    let key = (p.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(h) = hash_cache().lock().unwrap().get(&key) {
        return Ok(Some(h.clone()));
    }
    reading();
    let Ok(mut f) = std::fs::File::open(p) else {
        return Ok(None);
    };
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if stop() {
            return Err(Stopped);
        }
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(_) => return Ok(None),
        }
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    hash_cache().lock().unwrap().insert(key, hex.clone());
    Ok(Some(hex))
}

/// Finds the source's file for each catalogue file. A file claimed by one
/// model (`taken`) is not offered to another.
struct Matcher<'a> {
    cands: &'a [Candidate],
    taken: Vec<bool>,
    /// Sizes that catalogue files with different fingerprints share: a file
    /// of such a size is hashed before it counts (folder scans only).
    shared: Vec<u64>,
    hashes: HashMap<usize, Option<String>>,
    /// Told each file before it is read to hash it (the scan's "Checking …").
    checking: Option<&'a mut dyn FnMut(&str)>,
    stop: Option<&'a dyn Fn() -> bool>,
    /// `stop` said stop: nothing more is hashed and the scan is void.
    stopped: bool,
}

impl<'a> Matcher<'a> {
    fn new(cands: &'a [Candidate], shared: Vec<u64>) -> Self {
        Matcher {
            cands,
            taken: vec![false; cands.len()],
            shared,
            hashes: HashMap::new(),
            checking: None,
            stop: None,
            stopped: false,
        }
    }

    fn watched(mut self, checking: &'a mut dyn FnMut(&str), stop: &'a dyn Fn() -> bool) -> Self {
        self.checking = Some(checking);
        self.stop = Some(stop);
        self
    }

    /// `cands[i]` may be `f`: always, unless its size is shared, then by hash.
    fn fits(&mut self, i: usize, f: &ModelFile) -> bool {
        if !self.shared.contains(&f.size) {
            return true;
        }
        let cands = self.cands;
        let Loc::Path(p) = &cands[i].at else {
            return true;
        };
        if let Some(h) = self.hashes.get(&i) {
            return h.as_deref() == Some(f.sha256);
        }
        if self.stopped {
            return false;
        }
        let rel = &cands[i].rel;
        let checking = &mut self.checking;
        let mut reading = || {
            if let Some(c) = checking.as_mut() {
                c(rel)
            }
        };
        let h = match fingerprint(p, &mut reading, self.stop.unwrap_or(&|| false)) {
            Ok(h) => h,
            Err(Stopped) => {
                self.stopped = true;
                return false;
            }
        };
        let fits = h.as_deref() == Some(f.sha256);
        self.hashes.insert(i, h);
        fits
    }

    /// The folder holding most of `e`'s files by name and size; one named
    /// like the model wins a tie, then the first by name.
    fn home(&self, e: &CatalogEntry) -> Option<String> {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for (i, c) in self.cands.iter().enumerate() {
            let named = e
                .files
                .iter()
                .any(|f| f.size == c.size && c.name.eq_ignore_ascii_case(f.name));
            if named && !self.taken[i] {
                *count.entry(c.dir()).or_default() += 1;
            }
        }
        let own = |d: &str| {
            let last = d.rsplit('/').next().unwrap_or(d);
            last.eq_ignore_ascii_case(e.id) || last.eq_ignore_ascii_case(e.dest)
        };
        count
            .into_iter()
            .max_by(|(a, n), (b, m)| n.cmp(m).then(own(a).cmp(&own(b))).then(b.cmp(a)))
            .map(|(d, _)| d.to_string())
    }

    /// The source's file for `f`: same name and size, else the first one
    /// with the pinned size (a renamed copy), files in `home` first.
    fn find(&mut self, f: &ModelFile, home: Option<&str>) -> Hit {
        let mut order: Vec<usize> = (0..self.cands.len()).filter(|&i| !self.taken[i]).collect();
        order.sort_by_key(|&i| home.is_some_and(|h| self.cands[i].dir() != h));
        let named = |c: &Candidate| c.name.eq_ignore_ascii_case(f.name);
        let mut wrong_hash = None;
        for &i in &order {
            if named(&self.cands[i]) && self.cands[i].size == f.size {
                if self.fits(i, f) {
                    return Hit::Found(i);
                }
                wrong_hash.get_or_insert(i);
            }
        }
        for &i in &order {
            if !named(&self.cands[i]) && self.cands[i].size == f.size && self.fits(i, f) {
                return Hit::Found(i);
            }
        }
        if let Some(i) = wrong_hash {
            return Hit::WrongHash(i);
        }
        match order.iter().find(|&&i| named(&self.cands[i])) {
            Some(&i) => Hit::WrongSize(i),
            None => Hit::None,
        }
    }
}

/// How a model is looked for.
#[derive(Debug, Clone, Copy)]
struct Mode {
    /// Bring its extras along (the Hebrew models' language-ID model).
    extras: bool,
    /// A file the source lacks may already be installed and intact.
    installed_ok: bool,
}

/// "Import from file…" for one model.
const ONE_MODEL: Mode = Mode {
    extras: true,
    installed_ok: true,
};
/// A folder of models: the language-ID model is a row of its own, and only
/// what the folder holds counts.
const SCAN: Mode = Mode {
    extras: false,
    installed_ok: false,
};

/// One file to copy.
#[derive(Debug)]
struct Step {
    f: &'static ModelFile,
    target: PathBuf,
    /// Index into the source's files.
    from: usize,
}

/// What a source holds of one model.
#[derive(Debug, Default)]
struct Plan {
    steps: Vec<Step>,
    /// Files found (by name or size).
    found: usize,
    /// Files found or present under the wrong size / fingerprint.
    matched: usize,
    /// Of `matched`, the ones with the model's own file name.
    by_name: usize,
    /// Source files it claims (found or wrong).
    used: Vec<usize>,
    missing: Vec<&'static str>,
    wrong: Vec<String>,
    /// The folder its files are in (relative; "" = the top).
    home: String,
}

impl Plan {
    fn complete(&self) -> bool {
        self.found > 0 && self.missing.is_empty() && self.wrong.is_empty()
    }

    /// Why it can't be used, for a list.
    fn problem(&self) -> String {
        let mut out = vec![];
        if !self.wrong.is_empty() {
            out.push(format!(
                "{}: a different version or an incomplete copy",
                self.wrong.join("; ")
            ));
        }
        if !self.missing.is_empty() {
            out.push(format!("{} not found", self.missing.join(", ")));
        }
        out.join(". ")
    }

    fn bytes(&self) -> u64 {
        self.steps.iter().map(|s| s.f.size).sum()
    }
}

/// Look for `e`'s files. Found files are marked taken in `m`; a caller that
/// drops the plan restores `m.taken`.
fn plan(e: &CatalogEntry, models_dir: &Path, m: &mut Matcher, mode: Mode) -> Plan {
    let home = m.home(e);
    let mut p = Plan {
        home: home.clone().unwrap_or_default(),
        ..Plan::default()
    };
    for (f, extra) in e.all_files() {
        if extra && !mode.extras {
            continue;
        }
        let target = e.file_path(models_dir, f, extra);
        let hit = m.find(f, home.as_deref());
        match hit {
            Hit::Found(i) => {
                m.taken[i] = true;
                p.found += 1;
                p.matched += 1;
                p.used.push(i);
                if m.cands[i].name.eq_ignore_ascii_case(f.name) {
                    p.by_name += 1;
                }
                // The picked file is the installed one itself: nothing to do.
                if let Loc::Path(src) = &m.cands[i].at {
                    if same_file(src, &target) {
                        continue;
                    }
                }
                p.steps.push(Step { f, target, from: i });
            }
            _ if extra => {}
            _ if mode.installed_ok && intact(&target, f) => {}
            Hit::None => p.missing.push(f.name),
            Hit::WrongSize(i) => {
                p.matched += 1;
                p.by_name += 1;
                p.used.push(i);
                p.wrong.push(format!(
                    "{} is {} bytes, {} needs {}",
                    f.name, m.cands[i].size, e.title, f.size
                ));
            }
            Hit::WrongHash(i) => {
                p.matched += 1;
                p.by_name += 1;
                p.used.push(i);
                p.wrong.push(format!(
                    "{} did not match the fingerprint of {}",
                    f.name, e.title
                ));
            }
        }
    }
    p
}

/// Import `e` from `src` (folder, zip or file) into `models_dir`. Emits
/// `importing` / `verifying` progress through `emit`, but not the final
/// state: the caller sends that once prefs are updated (as with downloads).
pub fn run_import(
    e: &CatalogEntry,
    models_dir: &Path,
    src: &Path,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Stop> {
    let mut source = Source::open(src).map_err(Stop::Failed)?;
    let cands = source.candidates().to_vec();
    let p = plan(e, models_dir, &mut Matcher::new(&cands, vec![]), ONE_MODEL);
    if p.matched == 0 {
        return Err(Stop::Failed(format!(
            "{} has none of the files of {} ({}). Only the models listed here can be imported, \
             each checked against its built-in fingerprint; custom models will come from your server's library.",
            src.display(),
            e.title,
            e.files.iter().map(|f| f.name).collect::<Vec<_>>().join(", ")
        )));
    }
    if !p.wrong.is_empty() {
        return Err(Stop::Failed(format!(
            "{}: a different version or an incomplete copy. Nothing was installed.",
            p.wrong.join("; ")
        )));
    }
    if !p.missing.is_empty() {
        return Err(Stop::Failed(format!(
            "{} is missing {} of {}. Nothing was installed.",
            src.display(),
            p.missing.join(", "),
            e.title
        )));
    }
    let dir = if e.is_dir() {
        e.dest_path(models_dir)
    } else {
        models_dir.to_path_buf()
    };
    std::fs::create_dir_all(dir)
        .map_err(|er| Stop::Failed(format!("could not create the models folder: {er}")))?;
    let total = p.bytes();
    let id = e.id.to_string();
    install(
        &mut source,
        &cands,
        &p.steps,
        cancel,
        &mut |received, state| {
            emit(Progress {
                id: id.clone(),
                received,
                total,
                state,
                error: None,
                path: None,
            })
        },
    )?;
    Ok(e.dest_path(models_dir))
}

/// Copy and check `steps`, then rename them all into place: all or nothing.
/// `report` gets the bytes done so far of these steps.
fn install(
    source: &mut Source,
    cands: &[Candidate],
    steps: &[Step],
    cancel: &AtomicBool,
    report: &mut dyn FnMut(u64, State),
) -> Result<(), Stop> {
    for s in steps {
        if let Some(d) = s.target.parent() {
            std::fs::create_dir_all(d)
                .map_err(|er| Stop::Failed(format!("could not create the models folder: {er}")))?;
        }
    }
    report(0, State::Importing);
    let mut parts: Vec<PathBuf> = vec![];
    let cleanup = |parts: &[PathBuf]| {
        for p in parts {
            let _ = std::fs::remove_file(p);
        }
    };
    let mut done = 0u64;
    for s in steps {
        let part = part_path(&s.target, s.f);
        parts.push(part.clone());
        let r = copy_one(source, &cands[s.from], s.f, &part, cancel, &mut |n| {
            report(done + n, State::Importing)
        });
        report(done + s.f.size, State::Verifying);
        match r {
            Ok(digest) if digest == s.f.sha256 => {}
            Ok(_) => {
                cleanup(&parts);
                return Err(Stop::Failed(format!(
                    "{} did not match its fingerprint (a different or damaged file). Nothing was installed.",
                    s.f.name
                )));
            }
            Err(stop) => {
                cleanup(&parts);
                return Err(stop);
            }
        }
        done += s.f.size;
    }
    if cancel.load(Ordering::SeqCst) {
        cleanup(&parts);
        return Err(Stop::Cancelled);
    }
    for (i, s) in steps.iter().enumerate() {
        if let Err(er) = std::fs::rename(&parts[i], &s.target) {
            cleanup(&parts[i..]);
            return Err(Stop::Failed(format!("could not save {}: {er}", s.f.name)));
        }
    }
    Ok(())
}

fn copy_one(
    source: &mut Source,
    from: &Candidate,
    f: &ModelFile,
    part: &Path,
    cancel: &AtomicBool,
    tick: &mut dyn FnMut(u64),
) -> Result<String, Stop> {
    let fail = |er: String| Stop::Failed(format!("could not read {}: {er}", from.name));
    match (&from.at, source) {
        (Loc::Path(p), _) => {
            let mut r = std::fs::File::open(p).map_err(|er| fail(er.to_string()))?;
            copy_verified(&mut r, f, part, "reading", cancel, tick)
        }
        (Loc::Zip(i), Source::Zip(z, _)) => {
            let mut r = z.by_index(*i).map_err(|er| fail(er.to_string()))?;
            copy_verified(&mut r, f, part, "unzipping", cancel, tick)
        }
        (Loc::Zip(_), Source::Files(_)) => Err(fail("not in a zip".into())),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

// ------------------------------------------------------------ a folder of models

/// `model:download` id of a folder import ([`import_folder`]).
pub const FOLDER_JOB_ID: &str = "folder-import";
/// Row id of the language-ID model in [`FolderScan`] (not a catalogue id).
pub const LID_ID: &str = "lid";
const LID_TITLE: &str = "Whisper tiny (language detection)";
/// A scan that a newer one replaced ([`scan_folder`]'s `stop`).
pub const SCAN_STOPPED: &str = "stopped: another folder was picked";

/// One of setup's three rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKey {
    English,
    Hebrew,
    Lid,
}

impl RowKey {
    pub const ALL: [RowKey; 3] = [RowKey::English, RowKey::Hebrew, RowKey::Lid];
}

/// Something for each of setup's rows; in JSON `english`, `hebrew`, `lid`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, bound(deserialize = "T: Deserialize<'de> + Default"))]
pub struct Rows<T> {
    pub english: T,
    pub hebrew: T,
    pub lid: T,
}

impl<T> Rows<T> {
    pub fn get(&self, k: RowKey) -> &T {
        match k {
            RowKey::English => &self.english,
            RowKey::Hebrew => &self.hebrew,
            RowKey::Lid => &self.lid,
        }
    }

    pub fn get_mut(&mut self, k: RowKey) -> &mut T {
        match k {
            RowKey::English => &mut self.english,
            RowKey::Hebrew => &mut self.hebrew,
            RowKey::Lid => &mut self.lid,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (RowKey, &T)> {
        RowKey::ALL.into_iter().map(move |k| (k, self.get(k)))
    }
}

/// One of setup's three rows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SlotView {
    /// Usable: `size` bytes in `folder` (relative to the picked one, "" =
    /// its top). `installed`: already in the models folder.
    Found {
        id: String,
        title: String,
        size: u64,
        folder: String,
        installed: bool,
    },
    Missing,
    /// Files of it are there but it can't be used (`message` says why).
    Problem {
        title: String,
        message: String,
    },
}

/// A catalogue model in the folder that none of the rows takes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OtherModel {
    pub id: String,
    pub title: String,
    pub size: u64,
    /// `speech`, `hebrew` or `cleanup`.
    pub kind: &'static str,
}

/// A problem that is not one of the rows' (a second, broken model).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelProblem {
    pub title: String,
    pub message: String,
}

/// A model-like file (`.onnx`, `.bin`, `.gguf`, a `.zip` …) no model claims.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileNote {
    /// Relative to the picked folder.
    pub path: String,
    pub size: u64,
    pub why: String,
}

/// `model_folder_scan`: what a folder holds for setup's checklist.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderScan {
    pub folder: String,
    #[serde(flatten)]
    pub rows: Rows<SlotView>,
    pub others: Vec<OtherModel>,
    pub problems: Vec<ModelProblem>,
    pub unrecognised: Vec<FileNote>,
    /// The folder is too big or too deep to search whole ([`LIMITS`]): what
    /// is past the limit was not looked at.
    pub large: bool,
}

/// Which found rows to use (`model_folder_import`).
pub type FolderPick = Rows<bool>;

/// What became of one row.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SlotResult {
    #[default]
    Skipped,
    Imported {
        id: String,
        title: String,
    },
    Failed {
        message: String,
    },
}

/// `model_folder_import`'s answer.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FolderImportView {
    #[serde(flatten)]
    pub rows: Rows<SlotResult>,
    /// The language was set to "Detect automatically"
    /// ([`auto_detect_when_ready`]).
    pub auto_detect: bool,
    pub cancelled: bool,
}

/// A finished folder import, before the prefs are updated ([`Self::apply`]).
#[derive(Debug)]
pub struct FolderImport {
    pub view: FolderImportView,
    /// The catalogue model behind each row (none for the language-ID row).
    entries: Rows<Option<&'static CatalogEntry>>,
    /// The rows that were installed.
    done: Rows<bool>,
}

impl FolderImport {
    /// Point the prefs at what was imported, the same as after a download
    /// of each; the language-ID model is the models folder's copy. Then
    /// setup's language default ([`auto_detect_when_ready`]).
    pub fn apply(&mut self, prefs: &mut Prefs, models_dir: &Path, appdata: Option<&str>) {
        for (k, e) in self.entries.iter() {
            if let (true, Some(e)) = (*self.done.get(k), e) {
                e.apply_to(prefs, models_dir);
            }
        }
        if self.done.lid {
            prefs.lid_model_path = None;
        }
        if self.any() && auto_detect_when_ready(prefs, appdata) {
            self.view.auto_detect = true;
        }
    }

    /// Something was imported.
    pub fn any(&self) -> bool {
        self.done.iter().any(|(_, d)| *d)
    }

    /// The rows that failed, for the final `model:download` event.
    pub fn errors(&self) -> Vec<String> {
        self.view
            .rows
            .iter()
            .filter_map(|(_, r)| match r {
                SlotResult::Failed { message } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Setup's language default, after its checklist installed something (a
/// folder import, or a row's Download): with the default English model, a
/// Hebrew model and the language-ID model all on this PC, the language
/// becomes "Detect automatically", so the language-ID warning has nothing
/// left to say. "Keyboard language" goes off too: it would otherwise pick
/// each dictation's language from the keyboard layout and auto-detect would
/// never run. True when the prefs were changed.
pub fn auto_detect_when_ready(prefs: &mut Prefs, appdata: Option<&str>) -> bool {
    if !languages_ready(prefs, appdata) {
        return false;
    }
    prefs.languages.clear();
    prefs.keyboard_language = false;
    true
}

/// The default model, a Hebrew model and the language-ID model are on disk.
fn languages_ready(prefs: &Prefs, appdata: Option<&str>) -> bool {
    let english = match prefs.asr_engine {
        AsrEngineChoice::Parakeet => prefs.resolved_parakeet_dir(appdata).is_dir(),
        AsrEngineChoice::Whisper => prefs.resolved_model_path(appdata).is_file(),
    };
    english
        && prefs
            .resolved_hebrew_dir(appdata)
            .is_some_and(|p| p.exists())
        && prefs.lid_model_present(appdata)
}

/// A row while scanning: what to copy for it.
#[derive(Debug)]
enum Slot {
    Found {
        entry: Option<&'static CatalogEntry>,
        title: &'static str,
        size: u64,
        folder: String,
        installed: bool,
        steps: Vec<Step>,
    },
    Missing,
    Problem {
        title: &'static str,
        message: String,
    },
}

impl Slot {
    fn view(&self) -> SlotView {
        match self {
            Slot::Found {
                entry,
                title,
                size,
                folder,
                installed,
                ..
            } => SlotView::Found {
                id: entry.map_or(LID_ID, |e| e.id).to_string(),
                title: title.to_string(),
                size: *size,
                folder: folder.clone(),
                installed: *installed,
            },
            Slot::Missing => SlotView::Missing,
            Slot::Problem { title, message } => SlotView::Problem {
                title: title.to_string(),
                message: message.clone(),
            },
        }
    }
}

/// The row a catalogue model fills; none for AI cleanup models.
fn row_of(e: &CatalogEntry) -> Option<RowKey> {
    if e.is_llm() {
        None
    } else if e.language.is_some() {
        Some(RowKey::Hebrew)
    } else {
        Some(RowKey::English)
    }
}

#[derive(Debug)]
struct Scan {
    rows: Rows<Slot>,
    others: Vec<OtherModel>,
    problems: Vec<ModelProblem>,
    unrecognised: Vec<FileNote>,
}

impl Scan {
    fn view(&self, folder: &Path, large: bool) -> FolderScan {
        FolderScan {
            folder: folder.display().to_string(),
            rows: Rows {
                english: self.rows.english.view(),
                hebrew: self.rows.hebrew.view(),
                lid: self.rows.lid.view(),
            },
            others: self.others.clone(),
            problems: self.problems.clone(),
            unrecognised: self.unrecognised.clone(),
            large,
        }
    }
}

/// The models a folder is matched against (the catalogue, or a test's).
#[derive(Clone, Copy)]
struct Known<'a> {
    catalog: &'static [CatalogEntry],
    lid: &'static ModelFile,
    /// This build runs the engine.
    built: &'a dyn Fn(ModelEngine) -> bool,
}

fn known() -> Known<'static> {
    Known {
        catalog: crate::model_download::CATALOG,
        lid: &crate::model_download::LID_FILES[0],
        built: &crate::model_download::engine_built,
    }
}

/// Sizes that files with different fingerprints share (the catalogue's
/// files and the language-ID model).
fn shared_sizes(catalog: &[CatalogEntry], lid: &ModelFile) -> Vec<u64> {
    let mut by_size: HashMap<u64, Vec<&str>> = HashMap::new();
    let all = catalog
        .iter()
        .flat_map(|e| e.all_files().map(|(f, _)| f))
        .chain(std::iter::once(lid));
    for f in all {
        let v = by_size.entry(f.size).or_default();
        if !v.contains(&f.sha256) {
            v.push(f.sha256);
        }
    }
    let mut out: Vec<u64> = by_size
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(s, _)| s)
        .collect();
    out.sort_unstable();
    out
}

/// Files that look like model files, so leaving one out is worth saying.
fn model_like(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        ".onnx", ".bin", ".gguf", ".ggml", ".zip", ".tar", ".bz2", ".gz", ".7z",
    ]
    .iter()
    .any(|x| n.ends_with(x))
        || (n.starts_with("tokens") && n.ends_with(".txt"))
}

/// Sort a folder's files into setup's rows (see the module docs).
/// `checking` is told each file read for its fingerprint; `stop` ends the
/// scan early.
fn scan_with(
    cands: &[Candidate],
    k: Known,
    models_dir: &Path,
    checking: &mut dyn FnMut(&str),
    stop: &dyn Fn() -> bool,
) -> Result<Scan, Stopped> {
    let mut m = Matcher::new(cands, shared_sizes(k.catalog, k.lid)).watched(checking, stop);
    // 1. Complete models, catalogue order (the recommended ones come first).
    let mut complete: Vec<(&'static CatalogEntry, Plan)> = vec![];
    for e in k.catalog {
        let before = m.taken.clone();
        let p = plan(e, models_dir, &mut m, SCAN);
        if p.complete() {
            complete.push((e, p));
        } else {
            m.taken = before;
        }
    }
    // 2. The language-ID model, a row of its own.
    let lid = k.lid;
    let lid_target = models_dir.join(lid.name);
    let lid_slot = match m.find(lid, None) {
        Hit::Found(i) => {
            m.taken[i] = true;
            let same = matches!(&cands[i].at, Loc::Path(p) if same_file(p, &lid_target));
            Slot::Found {
                entry: None,
                title: LID_TITLE,
                size: lid.size,
                folder: cands[i].dir().to_string(),
                installed: std::fs::metadata(&lid_target).is_ok_and(|x| x.len() == lid.size),
                steps: if same {
                    vec![]
                } else {
                    vec![Step {
                        f: lid,
                        target: lid_target,
                        from: i,
                    }]
                },
            }
        }
        Hit::WrongSize(i) | Hit::WrongHash(i) => {
            m.taken[i] = true;
            Slot::Problem {
                title: LID_TITLE,
                message: if cands[i].size == lid.size {
                    format!("{} did not match its fingerprint", cands[i].rel)
                } else {
                    format!(
                        "{} is {} bytes, needs {}: a different version or an incomplete copy",
                        cands[i].rel, cands[i].size, lid.size
                    )
                },
            }
        }
        Hit::None => Slot::Missing,
    };
    let mut rows = Rows {
        english: Slot::Missing,
        hebrew: Slot::Missing,
        lid: lid_slot,
    };
    // 3. The first complete model of each row that this build runs; the
    // rest are listed.
    let mut others = vec![];
    for (e, p) in complete {
        let Some(row) = row_of(e) else {
            others.push(other(e));
            continue;
        };
        let slot = rows.get_mut(row);
        let open = !matches!(slot, Slot::Found { .. });
        if open && (k.built)(e.engine) {
            *slot = Slot::Found {
                entry: Some(e),
                title: e.title,
                size: e.files.iter().map(|f| f.size).sum(),
                folder: p.home.clone(),
                installed: e.installed(models_dir),
                steps: p.steps,
            };
        } else if open && matches!(slot, Slot::Missing) {
            *slot = Slot::Problem {
                title: e.title,
                message: "this version of Murmur can't run it".into(),
            };
        } else {
            others.push(other(e));
        }
    }
    // 4. Broken or incomplete models: the one with the most matching files
    // first, so a broken Hebrew folder isn't reported as English.
    let mut broken: Vec<(&'static CatalogEntry, Plan)> = vec![];
    for e in k.catalog {
        let before = m.taken.clone();
        let p = plan(e, models_dir, &mut m, SCAN);
        m.taken = before;
        if p.by_name > 0 {
            broken.push((e, p));
        }
    }
    if m.stopped {
        return Err(Stopped);
    }
    broken.sort_by_key(|(_, p)| std::cmp::Reverse((p.found, p.by_name)));
    let mut problems = vec![];
    for (e, p) in broken {
        if p.used.iter().any(|&i| m.taken[i]) {
            continue;
        }
        for &i in &p.used {
            m.taken[i] = true;
        }
        match row_of(e).map(|r| rows.get_mut(r)) {
            Some(s) if matches!(s, Slot::Missing) => {
                *s = Slot::Problem {
                    title: e.title,
                    message: p.problem(),
                }
            }
            _ => problems.push(ModelProblem {
                title: e.title.to_string(),
                message: p.problem(),
            }),
        }
    }
    // 5. Model-like files nothing took.
    let unrecognised = cands
        .iter()
        .enumerate()
        .filter(|(i, c)| !m.taken[*i] && model_like(&c.name))
        .map(|(_, c)| FileNote {
            path: c.rel.clone(),
            size: c.size,
            why: if c.name.to_ascii_lowercase().ends_with(".zip") {
                "a .zip: unzip it into this folder first".into()
            } else {
                "not a model this version knows".into()
            },
        })
        .collect();
    Ok(Scan {
        rows,
        others,
        problems,
        unrecognised,
    })
}

fn other(e: &CatalogEntry) -> OtherModel {
    OtherModel {
        id: e.id.to_string(),
        title: e.title.to_string(),
        size: e.files.iter().map(|f| f.size).sum(),
        kind: match row_of(e) {
            Some(RowKey::Hebrew) => "hebrew",
            Some(_) => "speech",
            None => "cleanup",
        },
    }
}

/// What `src` (a folder) holds for setup's checklist. Reads the folder's
/// listing, plus the files whose size is ambiguous: `checking` is told each
/// of those before it is read, and `stop` (a newer scan started) ends the
/// scan with [`SCAN_STOPPED`]. Fingerprints are remembered for the import.
pub fn scan_folder(
    src: &Path,
    models_dir: &Path,
    checking: &mut dyn FnMut(&str),
    stop: &dyn Fn() -> bool,
) -> Result<FolderScan, String> {
    scan_dir(src, models_dir, LIMITS, known(), checking, stop)
}

fn scan_dir(
    src: &Path,
    models_dir: &Path,
    lim: Limits,
    k: Known,
    checking: &mut dyn FnMut(&str),
    stop: &dyn Fn() -> bool,
) -> Result<FolderScan, String> {
    let (cands, large) = list_folder(src, lim)?;
    let scan = scan_with(&cands, k, models_dir, checking, stop)
        .map_err(|Stopped| SCAN_STOPPED.to_string())?;
    Ok(scan.view(src, large))
}

/// Install the `pick`ed rows found in `src` (scanned again: the folder may
/// have changed since the checklist; fingerprints the scan worked out are
/// reused, so each file is read once more, to copy and check it). Each
/// model is all or nothing; one that fails leaves the others installed.
/// Progress goes out as [`FOLDER_JOB_ID`]; the caller sends the final state
/// after [`FolderImport::apply`].
pub fn import_folder(
    src: &Path,
    models_dir: &Path,
    pick: FolderPick,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<FolderImport, Stop> {
    import_with(src, models_dir, pick, known(), cancel, emit)
}

fn import_with(
    src: &Path,
    models_dir: &Path,
    pick: FolderPick,
    k: Known,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<FolderImport, Stop> {
    let (cands, _) = list_folder(src, LIMITS).map_err(Stop::Failed)?;
    let mut source = Source::Files(cands.clone());
    let stop = || cancel.load(Ordering::SeqCst);
    let scan =
        scan_with(&cands, k, models_dir, &mut |_| {}, &stop).map_err(|Stopped| Stop::Cancelled)?;
    let steps_of = |s: &Slot| -> u64 {
        match s {
            Slot::Found { steps, .. } => steps.iter().map(|x| x.f.size).sum(),
            _ => 0,
        }
    };
    let total: u64 = scan
        .rows
        .iter()
        .filter(|(row, _)| *pick.get(*row))
        .map(|(_, s)| steps_of(s))
        .sum();
    let mut out = FolderImport {
        view: FolderImportView::default(),
        entries: Rows::default(),
        done: Rows::default(),
    };
    let mut done = 0u64;
    for (row, slot) in scan.rows.iter() {
        if !*pick.get(row) {
            continue;
        }
        let result = match slot {
            _ if out.view.cancelled => SlotResult::Skipped,
            Slot::Found {
                entry,
                title,
                steps,
                ..
            } => {
                let r = install(&mut source, &cands, steps, cancel, &mut |n, state| {
                    emit(Progress {
                        id: FOLDER_JOB_ID.to_string(),
                        received: done + n,
                        total,
                        state,
                        error: None,
                        path: None,
                    })
                });
                done += steps_of(slot);
                match r {
                    Ok(()) => {
                        *out.entries.get_mut(row) = *entry;
                        *out.done.get_mut(row) = true;
                        SlotResult::Imported {
                            id: entry.map_or(LID_ID, |e| e.id).to_string(),
                            title: title.to_string(),
                        }
                    }
                    Err(Stop::Cancelled) => {
                        out.view.cancelled = true;
                        SlotResult::Failed {
                            message: format!("{title}: cancelled. Nothing of it was kept."),
                        }
                    }
                    Err(Stop::Failed(m)) => SlotResult::Failed {
                        message: format!("{title}: {m}"),
                    },
                }
            }
            Slot::Missing | Slot::Problem { .. } => SlotResult::Failed {
                message: "no longer in that folder".into(),
            },
        };
        *out.view.rows.get_mut(row) = result;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_download::ModelEngine;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn sha(b: &[u8]) -> &'static str {
        let d = Sha256::digest(b);
        Box::leak(
            d.iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>()
                .into_boxed_str(),
        )
    }

    fn mf(name: &'static str, b: &[u8]) -> ModelFile {
        ModelFile {
            name,
            url: "https://huggingface.co/r/resolve/x/f",
            size: b.len() as u64,
            sha256: sha(b),
        }
    }

    fn entry(engine: ModelEngine, files: Vec<ModelFile>, extras: Vec<ModelFile>) -> CatalogEntry {
        CatalogEntry {
            id: "t",
            title: "Test model",
            desc: "",
            engine,
            dest: match engine {
                ModelEngine::Parakeet | ModelEngine::SherpaOnline => "pk-dir",
                ModelEngine::Llm => "m.gguf",
                ModelEngine::Whisper => "m.bin",
            },
            page: "https://huggingface.co/r",
            recommended: false,
            recommended_gpu: false,
            language: None,
            license: None,
            files: Box::leak(files.into_boxed_slice()),
            extras: Box::leak(extras.into_boxed_slice()),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("murmur-imp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn files_in(d: &Path) -> Vec<String> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), out);
                } else {
                    out.push(e.path());
                }
            }
        }
        let mut v = vec![];
        walk(d, &mut v);
        let mut s: Vec<String> = v
            .iter()
            .map(|p| {
                p.strip_prefix(d)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        s.sort();
        s
    }

    fn zip_of(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, b) in files {
            z.start_file(*name, opt).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    fn import(e: &CatalogEntry, models: &Path, src: &Path) -> (Result<PathBuf, Stop>, Vec<State>) {
        let mut seen = vec![];
        let r = run_import(e, models, src, &AtomicBool::new(false), &mut |p| {
            seen.push(p.state)
        });
        (r, seen)
    }

    fn failed(r: &Result<PathBuf, Stop>, needle: &str) -> bool {
        matches!(r, Err(Stop::Failed(m)) if m.contains(needle))
    }

    #[test]
    fn a_single_model_file_imports_and_is_installed() {
        let e = entry(ModelEngine::Llm, vec![mf("m.gguf", b"gguf bytes")], vec![]);
        let d = tmp("single");
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        // A renamed copy is found by its pinned size; the hash decides.
        std::fs::write(src.join("my-model.gguf"), b"gguf bytes").unwrap();
        let models = d.join("models");
        let (r, seen) = import(&e, &models, &src.join("my-model.gguf"));
        assert_eq!(r.unwrap(), models.join("m.gguf"));
        assert_eq!(files_in(&models), vec!["m.gguf"]);
        assert!(e.installed(&models));
        assert_eq!(seen.first(), Some(&State::Importing));
        assert!(seen.contains(&State::Verifying));
        // The same state as after a download: prefs point at it.
        let mut prefs = crate::prefs::Prefs::default();
        e.apply_to(&mut prefs, &models);
        assert!(prefs.llm_enabled);
        assert_eq!(
            prefs.llm_model_path.as_deref(),
            Some(models.join("m.gguf").display().to_string().as_str())
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_model_imports_from_a_nested_folder() {
        let e = entry(
            ModelEngine::Parakeet,
            vec![mf("encoder.onnx", b"enc"), mf("tokens.txt", b"tokens")],
            vec![],
        );
        let d = tmp("folder");
        let src = d.join("src/sherpa-onnx-thing");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("encoder.onnx"), b"enc").unwrap();
        std::fs::write(src.join("tokens.txt"), b"tokens").unwrap();
        std::fs::write(src.join("README.md"), b"other stuff").unwrap();
        let models = d.join("models");
        let (r, _) = import(&e, &models, &d.join("src"));
        assert_eq!(r.unwrap(), models.join("pk-dir"));
        assert_eq!(
            files_in(&models),
            vec!["pk-dir/encoder.onnx", "pk-dir/tokens.txt"]
        );
        assert!(e.installed(&models));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_zip_with_a_top_folder_imports_with_its_extra() {
        let e = entry(
            ModelEngine::SherpaOnline,
            vec![
                mf("encoder.int8.onnx", b"encoder bytes"),
                mf("tokens.txt", b"tok"),
            ],
            vec![mf("lid.bin", b"language id")],
        );
        let d = tmp("zip");
        let z = d.join("model.zip");
        zip_of(
            &z,
            &[
                ("he/encoder.int8.onnx", b"encoder bytes"),
                ("he/tokens.txt", b"tok"),
                ("he/lid.bin", b"language id"),
            ],
        );
        let models = d.join("models");
        let (r, _) = import(&e, &models, &z);
        r.unwrap();
        assert_eq!(
            files_in(&models),
            vec!["lid.bin", "pk-dir/encoder.int8.onnx", "pk-dir/tokens.txt"]
        );
        assert!(e.complete(&models));
        // A zip entry's name never becomes a path: "../x" lands as the model file.
        let evil = d.join("evil.zip");
        zip_of(&evil, &[("../../m.bin", b"whisper")]);
        let w = entry(ModelEngine::Whisper, vec![mf("m.bin", b"whisper")], vec![]);
        import(&w, &models, &evil).0.unwrap();
        assert!(models.join("m.bin").is_file());
        assert!(!d.join("m.bin").exists() && !d.parent().unwrap().join("m.bin").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_extras_are_skipped_and_can_be_imported_later() {
        let e = entry(
            ModelEngine::SherpaOnline,
            vec![
                mf("encoder.int8.onnx", b"encoder bytes"),
                mf("tokens.txt", b"tok"),
            ],
            vec![mf("lid.bin", b"language id")],
        );
        let d = tmp("extras");
        let z = d.join("model.zip");
        zip_of(
            &z,
            &[
                ("encoder.int8.onnx", b"encoder bytes"),
                ("tokens.txt", b"tok"),
            ],
        );
        let models = d.join("models");
        import(&e, &models, &z).0.unwrap();
        assert!(e.installed(&models) && !e.complete(&models));
        // Later: just the language-ID file; the model files stay as they are.
        std::fs::write(d.join("lid.bin"), b"language id").unwrap();
        import(&e, &models, &d.join("lid.bin")).0.unwrap();
        assert!(e.complete(&models));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hash_mismatch_installs_nothing() {
        // Two files: the first matches, the second has the right size but
        // other bytes. Neither may land, and no .part is left.
        let e = entry(
            ModelEngine::Parakeet,
            vec![mf("encoder.onnx", b"enc"), mf("tokens.txt", b"tokens")],
            vec![],
        );
        let d = tmp("badhash");
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("encoder.onnx"), b"enc").unwrap();
        std::fs::write(src.join("tokens.txt"), b"TOKENS").unwrap();
        let models = d.join("models");
        let (r, _) = import(&e, &models, &src);
        assert!(
            failed(&r, "tokens.txt did not match its fingerprint"),
            "{r:?}"
        );
        assert!(failed(&r, "Nothing was installed"));
        assert!(files_in(&models).is_empty(), "{:?}", files_in(&models));
        assert!(!e.installed(&models));
        // Same through a zip.
        let z = d.join("m.zip");
        zip_of(&z, &[("encoder.onnx", b"enc"), ("tokens.txt", b"TOKENS")]);
        assert!(failed(&import(&e, &models, &z).0, "fingerprint"));
        assert!(files_in(&models).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unknown_files_are_refused() {
        let e = entry(ModelEngine::Llm, vec![mf("m.gguf", b"gguf bytes")], vec![]);
        let d = tmp("unknown");
        std::fs::write(
            d.join("custom-finetune.gguf"),
            b"some other model, not ours",
        )
        .unwrap();
        let models = d.join("models");
        let (r, seen) = import(&e, &models, &d.join("custom-finetune.gguf"));
        assert!(failed(&r, "none of the files of Test model"), "{r:?}");
        assert!(failed(&r, "server's library"));
        assert!(seen.is_empty());
        assert!(!models.exists());
        // A same-named file of the wrong size: says so.
        std::fs::write(d.join("m.gguf"), b"short").unwrap();
        let src = d.join("m.gguf");
        let (r, _) = import(&e, &models, &src);
        assert!(
            failed(&r, "m.gguf is 5 bytes, Test model needs 10"),
            "{r:?}"
        );
        assert!(!models.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_missing_a_file_or_with_a_wrong_size_installs_nothing() {
        let e = entry(
            ModelEngine::Parakeet,
            vec![mf("encoder.onnx", b"enc"), mf("tokens.txt", b"tokens")],
            vec![],
        );
        let d = tmp("partial");
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("encoder.onnx"), b"enc").unwrap();
        let models = d.join("models");
        let (r, _) = import(&e, &models, &src);
        assert!(failed(&r, "is missing tokens.txt of Test model"), "{r:?}");
        assert!(files_in(&models).is_empty());
        std::fs::write(src.join("tokens.txt"), b"tokens, but a newer version").unwrap();
        let (r, _) = import(&e, &models, &src);
        assert!(
            failed(&r, "tokens.txt is 27 bytes, Test model needs 6"),
            "{r:?}"
        );
        assert!(files_in(&models).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cancel_leaves_no_part_file() {
        let e = entry(
            ModelEngine::Whisper,
            vec![mf("m.bin", b"whisper bytes")],
            vec![],
        );
        let d = tmp("cancel");
        std::fs::write(d.join("m.bin"), b"whisper bytes").unwrap();
        let models = d.join("models");
        let r = run_import(
            &e,
            &models,
            &d.join("m.bin"),
            &AtomicBool::new(true),
            &mut |_| {},
        );
        assert_eq!(r, Err(Stop::Cancelled));
        assert!(files_in(&models).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn importing_the_installed_file_itself_is_a_no_op() {
        let e = entry(
            ModelEngine::Whisper,
            vec![mf("m.bin", b"whisper bytes")],
            vec![],
        );
        let d = tmp("self");
        let models = d.join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("m.bin"), b"whisper bytes").unwrap();
        import(&e, &models, &models.join("m.bin")).0.unwrap();
        assert_eq!(
            std::fs::read(models.join("m.bin")).unwrap(),
            b"whisper bytes"
        );
        assert_eq!(files_in(&models), vec!["m.bin"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn works_for_every_catalogue_kind() {
        // Every downloadable entry resolves its files from a folder named
        // like the download (sizes faked: only the matching is checked).
        for e in crate::model_download::CATALOG {
            let cands: Vec<Candidate> = e
                .all_files()
                .map(|(f, _)| Candidate {
                    name: f.name.into(),
                    size: f.size,
                    rel: format!("{}/{}", e.id, f.name),
                    at: Loc::Path(PathBuf::from(f.name)),
                })
                .collect();
            let mut m = Matcher::new(&cands, vec![]);
            for (f, _) in e.all_files() {
                assert!(
                    matches!(m.find(f, None), Hit::Found(i) if cands[i].name == f.name),
                    "{} {}",
                    e.id,
                    f.name
                );
            }
        }
    }

    // ------------------------------------------------------- a folder of models

    const EN_ENC: &[u8] = b"EN encoder bytes";
    // The two decoders share a size (as Parakeet TDT's and unified's do):
    // the scan tells them apart by hash.
    const EN_DEC: &[u8] = b"decoder-A";
    const EN_JOIN: &[u8] = b"EN joiner bytes";
    const EN_TOK: &[u8] = b"EN tokens list";
    const HE_ENC: &[u8] = b"HE encoder bytes!!";
    const HE_DEC: &[u8] = b"decoder-B";
    const HE_JOIN: &[u8] = b"HE joiner bytes..";
    const HE_TOK: &[u8] = b"HE tokens list!!!!!!";
    const LID: &[u8] = b"language id model bytes";
    const LLM: &[u8] = b"cleanup gguf";

    fn lid_file() -> &'static ModelFile {
        Box::leak(Box::new(mf("ggml-tiny-q5_1.bin", LID)))
    }

    fn sherpa_files(b: [&[u8]; 4]) -> &'static [ModelFile] {
        Box::leak(
            vec![
                mf("encoder.int8.onnx", b[0]),
                mf("decoder.int8.onnx", b[1]),
                mf("joiner.int8.onnx", b[2]),
                mf("tokens.txt", b[3]),
            ]
            .into_boxed_slice(),
        )
    }

    fn fake(
        id: &'static str,
        title: &'static str,
        engine: ModelEngine,
        dest: &'static str,
        language: Option<&'static str>,
        files: &'static [ModelFile],
        extras: &'static [ModelFile],
    ) -> CatalogEntry {
        CatalogEntry {
            id,
            title,
            desc: "",
            engine,
            dest,
            page: "https://huggingface.co/r",
            recommended: false,
            recommended_gpu: false,
            language,
            license: None,
            files,
            extras,
        }
    }

    /// An English model, a Hebrew model (same file names, other bytes, the
    /// language-ID model as its extra) and a cleanup model.
    fn catalog(lid: &'static ModelFile) -> &'static [CatalogEntry] {
        Box::leak(
            vec![
                fake(
                    "en-model",
                    "English model",
                    ModelEngine::Parakeet,
                    "en-model",
                    None,
                    sherpa_files([EN_ENC, EN_DEC, EN_JOIN, EN_TOK]),
                    &[],
                ),
                fake(
                    "he-model",
                    "Hebrew model",
                    ModelEngine::SherpaOnline,
                    crate::core_adapter::ZIPFORMER_HE_DIR,
                    Some("he"),
                    sherpa_files([HE_ENC, HE_DEC, HE_JOIN, HE_TOK]),
                    std::slice::from_ref(lid),
                ),
                fake(
                    "cleanup",
                    "Cleanup model",
                    ModelEngine::Llm,
                    "c.gguf",
                    None,
                    Box::leak(vec![mf("c.gguf", LLM)].into_boxed_slice()),
                    &[],
                ),
            ]
            .into_boxed_slice(),
        )
    }

    fn put(root: &Path, rel: &str, b: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b).unwrap();
    }

    fn english(root: &Path, dir: &str) {
        for (n, b) in [
            ("encoder.int8.onnx", EN_ENC),
            ("decoder.int8.onnx", EN_DEC),
            ("joiner.int8.onnx", EN_JOIN),
            ("tokens.txt", EN_TOK),
        ] {
            put(root, &format!("{dir}/{n}"), b);
        }
    }

    fn hebrew(root: &Path, dir: &str) {
        for (n, b) in [
            ("encoder.int8.onnx", HE_ENC),
            ("decoder.int8.onnx", HE_DEC),
            ("joiner.int8.onnx", HE_JOIN),
            ("tokens.txt", HE_TOK),
        ] {
            put(root, &format!("{dir}/{n}"), b);
        }
    }

    struct Fixture {
        root: PathBuf,
        src: PathBuf,
        appdata: PathBuf,
        models: PathBuf,
        lid: &'static ModelFile,
        catalog: &'static [CatalogEntry],
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = tmp(&format!("folder-{name}"));
            let appdata = root.join("appdata");
            let models = crate::core_adapter::models_dir(appdata.to_str());
            let lid = lid_file();
            let src = root.join("picked");
            std::fs::create_dir_all(&src).unwrap();
            Fixture {
                src,
                catalog: catalog(lid),
                root,
                appdata,
                models,
                lid,
            }
        }

        fn known(&self) -> Known<'static> {
            Known {
                catalog: self.catalog,
                lid: self.lid,
                built: &|_| true,
            }
        }

        fn scan_limited(&self, lim: Limits) -> FolderScan {
            scan_dir(
                &self.src,
                &self.models,
                lim,
                self.known(),
                &mut |_| {},
                &|| false,
            )
            .unwrap()
        }

        fn scan(&self) -> FolderScan {
            self.scan_limited(LIMITS)
        }

        fn import(&self, pick: FolderPick) -> FolderImport {
            import_with(
                &self.src,
                &self.models,
                pick,
                self.known(),
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap()
        }

        /// Import, then apply to prefs that had English as the language.
        fn import_and_apply(&self, pick: FolderPick) -> (FolderImport, Prefs) {
            let mut r = self.import(pick);
            let mut prefs = Prefs {
                languages: vec!["en".into()],
                ..Prefs::default()
            };
            r.apply(&mut prefs, &self.models, self.appdata.to_str());
            (r, prefs)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const ALL: FolderPick = FolderPick {
        english: true,
        hebrew: true,
        lid: true,
    };

    fn found(v: &SlotView) -> (&str, &str) {
        match v {
            SlotView::Found { id, folder, .. } => (id.as_str(), folder.as_str()),
            other => panic!("not found: {other:?}"),
        }
    }

    fn problem(v: &SlotView) -> &str {
        match v {
            SlotView::Problem { message, .. } => message.as_str(),
            other => panic!("not a problem: {other:?}"),
        }
    }

    fn en_only() -> Vec<String> {
        vec!["en".to_string()]
    }

    #[test]
    fn the_library_layout_fills_all_three_rows_and_turns_on_auto_detect() {
        let fx = Fixture::new("canonical");
        english(&fx.src, "library/en-model");
        hebrew(&fx.src, "library/he-model");
        put(&fx.src, "library/whisper-tiny-lid/ggml-tiny-q5_1.bin", LID);
        put(&fx.src, "library/he-model/model.toml", b"kind = 'hebrew'");
        put(&fx.src, "library/cleanup/c.gguf", LLM);
        let s = fx.scan();
        assert_eq!(found(&s.rows.english), ("en-model", "library/en-model"));
        assert_eq!(found(&s.rows.hebrew), ("he-model", "library/he-model"));
        assert_eq!(found(&s.rows.lid), (LID_ID, "library/whisper-tiny-lid"));
        // The cleanup model is listed, not used; model.toml is no model file.
        assert_eq!(s.others.len(), 1);
        assert_eq!(s.others[0].kind, "cleanup");
        assert!(s.problems.is_empty() && s.unrecognised.is_empty(), "{s:?}");

        let (r, prefs) = fx.import_and_apply(ALL);
        assert!(
            matches!(&r.view.rows.english, SlotResult::Imported { id, .. } if id == "en-model")
        );
        assert!(matches!(&r.view.rows.hebrew, SlotResult::Imported { id, .. } if id == "he-model"));
        assert!(matches!(&r.view.rows.lid, SlotResult::Imported { id, .. } if id == LID_ID));
        assert!(r.errors().is_empty());
        let he = crate::core_adapter::ZIPFORMER_HE_DIR;
        let mut want = vec![
            "en-model/decoder.int8.onnx".to_string(),
            "en-model/encoder.int8.onnx".into(),
            "en-model/joiner.int8.onnx".into(),
            "en-model/tokens.txt".into(),
            "ggml-tiny-q5_1.bin".into(),
            format!("{he}/decoder.int8.onnx"),
            format!("{he}/encoder.int8.onnx"),
            format!("{he}/joiner.int8.onnx"),
            format!("{he}/tokens.txt"),
        ];
        want.sort();
        assert_eq!(files_in(&fx.models), want);
        // The prefs a download of each would leave, and auto-detect on.
        assert_eq!(prefs.asr_engine, AsrEngineChoice::Parakeet);
        assert_eq!(
            prefs.parakeet_model_dir.as_deref(),
            Some(fx.models.join("en-model").display().to_string().as_str())
        );
        assert_eq!(prefs.hebrew_model, crate::prefs::HebrewModel::Zipformer);
        assert_eq!(prefs.lid_model_path, None);
        assert!(prefs.lid_model_present(fx.appdata.to_str()));
        assert!(prefs.languages.is_empty() && !prefs.keyboard_language);
        assert!(r.view.auto_detect);
    }

    #[test]
    fn renamed_folders_and_files_are_recognised_by_their_files() {
        let fx = Fixture::new("renamed");
        english(&fx.src, "My English");
        hebrew(&fx.src, "copied from usb/hebrew");
        // A renamed language-ID file: found by its pinned size, the hash decides.
        put(&fx.src, "detect.bin", LID);
        let s = fx.scan();
        assert_eq!(found(&s.rows.english), ("en-model", "My English"));
        assert_eq!(
            found(&s.rows.hebrew),
            ("he-model", "copied from usb/hebrew")
        );
        assert_eq!(found(&s.rows.lid), (LID_ID, ""));
        let (r, prefs) = fx.import_and_apply(ALL);
        assert!(r.errors().is_empty(), "{:?}", r.errors());
        assert!(fx.catalog[0].installed(&fx.models) && fx.catalog[1].complete(&fx.models));
        assert!(prefs.languages.is_empty());
    }

    #[test]
    fn flat_files_are_told_apart_even_when_sizes_are_shared() {
        // One folder, no subfolders: the English files renamed so the names
        // don't clash. Both decoders are 9 bytes; only the hash says which.
        let fx = Fixture::new("flat");
        put(&fx.src, "encoder.int8.onnx", HE_ENC);
        put(&fx.src, "decoder.int8.onnx", HE_DEC);
        put(&fx.src, "joiner.int8.onnx", HE_JOIN);
        put(&fx.src, "tokens.txt", HE_TOK);
        put(&fx.src, "en-encoder.onnx", EN_ENC);
        put(&fx.src, "en-decoder.onnx", EN_DEC);
        put(&fx.src, "en-joiner.onnx", EN_JOIN);
        put(&fx.src, "en-tokens.txt", EN_TOK);
        put(&fx.src, "ggml-tiny-q5_1.bin", LID);
        let s = fx.scan();
        assert_eq!(found(&s.rows.english).0, "en-model");
        assert_eq!(found(&s.rows.hebrew).0, "he-model");
        assert_eq!(found(&s.rows.lid).0, LID_ID);
        assert!(s.unrecognised.is_empty(), "{:?}", s.unrecognised);
        let (r, _) = fx.import_and_apply(ALL);
        assert!(r.errors().is_empty(), "{:?}", r.errors());
        assert_eq!(
            std::fs::read(fx.models.join("en-model/decoder.int8.onnx")).unwrap(),
            EN_DEC
        );
    }

    #[test]
    fn an_empty_folder_lists_all_three_as_missing() {
        let fx = Fixture::new("empty");
        let s = fx.scan();
        assert_eq!(
            (
                s.rows.english.clone(),
                s.rows.hebrew.clone(),
                s.rows.lid.clone()
            ),
            (SlotView::Missing, SlotView::Missing, SlotView::Missing)
        );
        assert!(s.others.is_empty() && s.problems.is_empty() && s.unrecognised.is_empty());
        // Picking them anyway installs nothing and says why.
        let (r, prefs) = fx.import_and_apply(ALL);
        assert_eq!(r.errors().len(), 3);
        assert!(!r.any());
        assert_eq!(prefs.languages, en_only());
        assert!(files_in(&fx.models).is_empty());
    }

    #[test]
    fn a_partial_folder_uses_what_is_there() {
        let fx = Fixture::new("partial");
        english(&fx.src, "en");
        put(&fx.src, "lid/ggml-tiny-q5_1.bin", LID);
        // A Hebrew folder that lost a file: said so, never ignored.
        for (n, b) in [
            ("encoder.int8.onnx", HE_ENC),
            ("decoder.int8.onnx", HE_DEC),
            ("joiner.int8.onnx", HE_JOIN),
        ] {
            put(&fx.src, &format!("he/{n}"), b);
        }
        let s = fx.scan();
        assert_eq!(found(&s.rows.english).0, "en-model");
        assert_eq!(found(&s.rows.lid).0, LID_ID);
        assert_eq!(problem(&s.rows.hebrew), "tokens.txt not found");
        // English and language detection only: no Hebrew, so the language stays.
        let (r, prefs) = fx.import_and_apply(FolderPick {
            english: true,
            hebrew: false,
            lid: true,
        });
        assert_eq!(r.view.rows.hebrew, SlotResult::Skipped);
        assert!(r.errors().is_empty());
        assert!(!r.view.auto_detect);
        assert_eq!(prefs.languages, en_only());
        assert!(prefs.keyboard_language);
        assert_eq!(prefs.asr_engine, AsrEngineChoice::Parakeet);
        assert!(!fx
            .models
            .join(crate::core_adapter::ZIPFORMER_HE_DIR)
            .exists());
        assert!(fx.models.join("ggml-tiny-q5_1.bin").is_file());
    }

    #[test]
    fn auto_detect_needs_an_english_model_too() {
        let fx = Fixture::new("hebrew-only");
        hebrew(&fx.src, "he");
        put(&fx.src, "ggml-tiny-q5_1.bin", LID);
        let s = fx.scan();
        assert_eq!(s.rows.english, SlotView::Missing);
        let (r, prefs) = fx.import_and_apply(FolderPick {
            english: false,
            hebrew: true,
            lid: true,
        });
        assert!(r.errors().is_empty());
        // The default English model (Whisper base) is not on disk.
        assert!(!r.view.auto_detect);
        assert_eq!(prefs.languages, en_only());
    }

    #[test]
    fn corrupt_and_unknown_files_are_reported() {
        let fx = Fixture::new("corrupt");
        // English: right sizes, but tokens.txt has other bytes (wrong hash).
        english(&fx.src, "en");
        put(&fx.src, "en/tokens.txt", b"EN tokens LIST");
        // Hebrew: the encoder is cut short (wrong size).
        hebrew(&fx.src, "he");
        put(&fx.src, "he/encoder.int8.onnx", b"HE encoder");
        // Language detection: a different version (wrong size).
        put(&fx.src, "ggml-tiny-q5_1.bin", b"a newer language id model");
        put(&fx.src, "extra/mystery.onnx", b"some other model");
        put(&fx.src, "models.zip", b"PK not unzipped");
        put(&fx.src, "readme.md", b"not a model file, not listed");
        let s = fx.scan();
        // A wrong hash shows when the files are copied (each is read once).
        assert_eq!(found(&s.rows.english).0, "en-model");
        assert!(
            problem(&s.rows.hebrew)
                .contains("encoder.int8.onnx is 10 bytes, Hebrew model needs 18"),
            "{s:?}"
        );
        assert!(
            problem(&s.rows.lid).contains("is 25 bytes, needs 23"),
            "{s:?}"
        );
        let notes: Vec<(&str, &str)> = s
            .unrecognised
            .iter()
            .map(|n| (n.path.as_str(), n.why.as_str()))
            .collect();
        assert_eq!(
            notes,
            vec![
                ("extra/mystery.onnx", "not a model this version knows"),
                ("models.zip", "a .zip: unzip it into this folder first"),
            ]
        );
        let (r, prefs) = fx.import_and_apply(ALL);
        match &r.view.rows.english {
            SlotResult::Failed { message } => assert!(
                message.contains("tokens.txt did not match its fingerprint"),
                "{message}"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(r.errors().len(), 3);
        assert!(!r.any());
        // Nothing of the English model was kept, not even its good files.
        assert!(
            files_in(&fx.models).is_empty(),
            "{:?}",
            files_in(&fx.models)
        );
        assert_eq!(prefs.languages, en_only());
    }

    #[test]
    fn a_damaged_model_does_not_stop_the_others() {
        let fx = Fixture::new("one-bad");
        english(&fx.src, "en");
        put(&fx.src, "en/tokens.txt", b"EN tokens LIST");
        hebrew(&fx.src, "he");
        put(&fx.src, "ggml-tiny-q5_1.bin", LID);
        let (r, prefs) = fx.import_and_apply(ALL);
        assert!(matches!(r.view.rows.english, SlotResult::Failed { .. }));
        assert!(matches!(r.view.rows.hebrew, SlotResult::Imported { .. }));
        assert!(matches!(r.view.rows.lid, SlotResult::Imported { .. }));
        assert!(fx.catalog[1].complete(&fx.models));
        assert!(files_in(&fx.models.join("en-model")).is_empty());
        assert_eq!(prefs.hebrew_model, crate::prefs::HebrewModel::Zipformer);
        // Whisper base (the default English model) is not on disk.
        assert!(!r.view.auto_detect);
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let fx = Fixture::new("file");
        put(&fx.src, "ggml-tiny-q5_1.bin", LID);
        let f = fx.src.join("ggml-tiny-q5_1.bin");
        assert!(scan_folder(&f, &fx.models, &mut |_| {}, &|| false)
            .unwrap_err()
            .contains("is not a folder"));
    }

    #[test]
    fn the_real_catalogue_in_the_library_layout_maps_to_the_recommended_rows() {
        // Every catalogue model and the language-ID model, each in the folder
        // named after its id (docs/air-gapped-models.txt). Zip locations:
        // nothing is read, so only names, sizes and folders decide.
        let lid = &crate::model_download::LID_FILES[0];
        let mut cands: Vec<Candidate> = vec![];
        for e in crate::model_download::CATALOG {
            for f in e.files {
                cands.push(Candidate {
                    name: f.name.into(),
                    size: f.size,
                    rel: format!("library/{}/{}", e.id, f.name),
                    at: Loc::Zip(0),
                });
            }
        }
        cands.push(Candidate {
            name: lid.name.into(),
            size: lid.size,
            rel: format!("library/whisper-tiny-lid/{}", lid.name),
            at: Loc::Zip(0),
        });
        let d = tmp("real-catalogue");
        let k = Known {
            catalog: crate::model_download::CATALOG,
            lid,
            built: &|_| true,
        };
        let s = scan_with(&cands, k, &d, &mut |_| {}, &|| false)
            .unwrap()
            .view(&d, false);
        assert_eq!(
            found(&s.rows.english),
            (
                "parakeet-tdt-0.6b-v2-int8",
                "library/parakeet-tdt-0.6b-v2-int8"
            )
        );
        assert_eq!(
            found(&s.rows.hebrew),
            ("zipformer-he-int8", "library/zipformer-he-int8")
        );
        assert_eq!(found(&s.rows.lid), (LID_ID, "library/whisper-tiny-lid"));
        // Every other model is listed; nothing is a problem or unknown.
        assert_eq!(s.others.len(), crate::model_download::CATALOG.len() - 2);
        assert!(s.problems.is_empty() && s.unrecognised.is_empty(), "{s:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_past_the_limits_says_it_is_large() {
        // Deeper than the search goes: not looked at, and said so.
        let fx = Fixture::new("deep");
        put(&fx.src, "a/b/c/d/ggml-tiny-q5_1.bin", LID);
        let s = fx.scan();
        assert!(s.large);
        assert_eq!(s.rows.lid, SlotView::Missing);
        // More files than the limit.
        let fx = Fixture::new("many");
        for n in 0..5 {
            put(&fx.src, &format!("f{n}.txt"), b"x");
        }
        let lim = |files| Limits { depth: 4, files };
        assert!(fx.scan_limited(lim(3)).large);
        assert!(!fx.scan_limited(lim(5)).large);
        // The usual layouts are well inside them.
        let fx = Fixture::new("not-large");
        english(&fx.src, "library/en-model");
        assert!(!fx.scan().large);
    }

    #[test]
    fn a_scans_fingerprints_are_reused_by_the_import() {
        let fx = Fixture::new("hash-once");
        // The decoder's size is shared with the Hebrew one: it is hashed.
        english(&fx.src, "en");
        let scan = |seen: &mut Vec<String>| {
            scan_dir(
                &fx.src,
                &fx.models,
                LIMITS,
                fx.known(),
                &mut |f| seen.push(f.to_string()),
                &|| false,
            )
            .unwrap()
        };
        let mut seen = vec![];
        let s = scan(&mut seen);
        assert_eq!(found(&s.rows.english).0, "en-model");
        assert_eq!(seen, vec!["en/decoder.int8.onnx".to_string()]);
        // The import's own scan reads nothing to hash again.
        let mut again = vec![];
        assert_eq!(scan(&mut again), s);
        assert!(again.is_empty(), "{again:?}");
        // A changed file (same size, newer time) is hashed again. The time
        // is set: a rewrite within one clock tick keeps the old one.
        let dec = fx.src.join("en/decoder.int8.onnx");
        put(&fx.src, "en/decoder.int8.onnx", b"decoder-B");
        let later = std::fs::metadata(&dec).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&dec)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let mut changed = vec![];
        scan(&mut changed);
        assert_eq!(changed, vec!["en/decoder.int8.onnx".to_string()]);
    }

    #[test]
    fn a_stopped_scan_answers_nothing() {
        let fx = Fixture::new("stopped");
        english(&fx.src, "en");
        let e = scan_dir(
            &fx.src,
            &fx.models,
            LIMITS,
            fx.known(),
            &mut |_| {},
            &|| true,
        )
        .unwrap_err();
        assert_eq!(e, SCAN_STOPPED);
    }

    #[test]
    fn a_download_that_completes_the_set_turns_on_auto_detect() {
        // As after a checklist row's Download of the Hebrew model: the
        // English model and the language-ID model came from the folder.
        let fx = Fixture::new("download");
        english(&fx.src, "en");
        put(&fx.src, "ggml-tiny-q5_1.bin", LID);
        let (r, mut prefs) = fx.import_and_apply(FolderPick {
            english: true,
            hebrew: false,
            lid: true,
        });
        assert!(!r.view.auto_detect);
        assert!(!auto_detect_when_ready(&mut prefs, fx.appdata.to_str()));
        assert_eq!(prefs.languages, en_only());
        // The download saves the Hebrew files and points the prefs at them.
        let he = &fx.catalog[1];
        for (f, b) in he.files.iter().zip([HE_ENC, HE_DEC, HE_JOIN, HE_TOK]) {
            put(&fx.models, &format!("{}/{}", he.dest, f.name), b);
        }
        he.apply_to(&mut prefs, &fx.models);
        assert!(auto_detect_when_ready(&mut prefs, fx.appdata.to_str()));
        assert!(prefs.languages.is_empty() && !prefs.keyboard_language);
    }

    #[test]
    fn an_english_import_that_completes_the_set_turns_on_auto_detect() {
        let fx = Fixture::new("english-last");
        hebrew(&fx.src, "he");
        put(&fx.src, "ggml-tiny-q5_1.bin", LID);
        let (r, mut prefs) = fx.import_and_apply(ALL);
        // No English model yet (the default, Whisper base, is not on disk).
        assert!(!r.view.auto_detect);
        english(&fx.src, "en");
        let mut r = fx.import(FolderPick {
            english: true,
            hebrew: false,
            lid: false,
        });
        r.apply(&mut prefs, &fx.models, fx.appdata.to_str());
        assert!(r.view.auto_detect);
        assert!(prefs.languages.is_empty() && !prefs.keyboard_language);
    }

    #[test]
    fn the_rows_serialize_flat() {
        let v = serde_json::to_value(FolderImportView {
            rows: Rows {
                english: SlotResult::Imported {
                    id: "e".into(),
                    title: "E".into(),
                },
                ..Rows::default()
            },
            auto_detect: true,
            cancelled: false,
        })
        .unwrap();
        assert_eq!(v["english"]["state"], "imported");
        assert_eq!(v["lid"]["state"], "skipped");
        assert_eq!(v["auto_detect"], true);
        let p: FolderPick = serde_json::from_str(r#"{"hebrew":true}"#).unwrap();
        assert_eq!(
            p,
            FolderPick {
                english: false,
                hebrew: true,
                lid: false
            }
        );
    }
}
