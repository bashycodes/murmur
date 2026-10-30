//! Speech-model downloader (D9 exception on an explicit click, see
//! docs/DECISIONS.md D31).
//!
//! * [`CATALOG`]: the few models the local engine already loads (a whisper.cpp
//!   ggml file, or the Parakeet sherpa-onnx folder), each file with its URL
//!   pinned to a Hugging Face commit, its size and its SHA-256. Nothing else
//!   can be downloaded: the command takes a catalogue id, never a URL.
//! * [`run`]: download one entry into the models folder. Each file goes to
//!   `<name>.part`, is hashed while it streams, and is renamed over `<name>`
//!   only when size and SHA-256 match; a mismatch deletes it. A file that is
//!   already there and verifies is not fetched again (a retry after a cancel
//!   resumes at the next file).
//! * [`Downloads`]: one download at a time, cancellable.
//! * `HttpFetch` (feature `download`): reqwest over rustls, https only,
//!   redirects only within Hugging Face (`huggingface.co`, `*.hf.co`).
//!   Client-only builds (`local,app`) have no feature `download`, so no
//!   network code: there the UI shows the links instead.
//!
//! No download ever starts by itself: only `model_download_start`, which the
//! page sends after the user pressed Download in the confirm dialog.

use crate::prefs::{AsrEngineChoice, Prefs};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelEngine {
    Whisper,
    Parakeet,
}

#[derive(Debug)]
pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

#[derive(Debug)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub engine: ModelEngine,
    /// File name (whisper) or folder name (Parakeet) in the models folder.
    pub dest: &'static str,
    /// The model's page, for "Download links" and the confirm dialog.
    pub page: &'static str,
    pub recommended: bool,
    pub files: &'static [ModelFile],
}

macro_rules! model_file {
    ($base:expr, $name:literal, $size:literal, $sha:literal) => {
        ModelFile {
            name: $name,
            url: concat!($base, $name),
            size: $size,
            sha256: $sha,
        }
    };
}

macro_rules! parakeet_base {
    () => {
        "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/"
    };
}
macro_rules! whisper_base {
    () => {
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/"
    };
}

/// Hashes computed 2026-10-30 by downloading each file once (they equal the
/// Hugging Face LFS ids at those commits).
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "parakeet-tdt-0.6b-v2-int8",
        title: "Parakeet TDT 0.6B",
        desc: "English · fastest",
        engine: ModelEngine::Parakeet,
        dest: crate::core_adapter::DEFAULT_PARAKEET_DIR,
        page: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        recommended: true,
        files: &[
            model_file!(parakeet_base!(), "encoder.int8.onnx", 652_184_296, "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab"),
            model_file!(parakeet_base!(), "decoder.int8.onnx", 7_257_753, "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e"),
            model_file!(parakeet_base!(), "joiner.int8.onnx", 1_739_080, "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2"),
            model_file!(parakeet_base!(), "tokens.txt", 9_384, "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d"),
        ],
    },
    CatalogEntry {
        id: "whisper-base",
        title: "Whisper base",
        desc: "Many languages",
        engine: ModelEngine::Whisper,
        dest: crate::core_adapter::DEFAULT_MODEL_FILE,
        page: "https://huggingface.co/ggerganov/whisper.cpp",
        recommended: false,
        files: &[model_file!(whisper_base!(), "ggml-base.bin", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe")],
    },
    CatalogEntry {
        id: "whisper-small-q5_1",
        title: "Whisper small",
        desc: "Many languages · more accurate, slower",
        engine: ModelEngine::Whisper,
        dest: "ggml-small-q5_1.bin",
        page: "https://huggingface.co/ggerganov/whisper.cpp",
        recommended: false,
        files: &[model_file!(whisper_base!(), "ggml-small-q5_1.bin", 190_085_487, "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb")],
    },
];

pub fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

impl CatalogEntry {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    fn is_dir(&self) -> bool {
        self.engine == ModelEngine::Parakeet
    }

    /// Where the model ends up: the file, or the Parakeet folder.
    pub fn dest_path(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.dest)
    }

    fn file_path(&self, models_dir: &Path, f: &ModelFile) -> PathBuf {
        if self.is_dir() {
            self.dest_path(models_dir).join(f.name)
        } else {
            self.dest_path(models_dir)
        }
    }

    /// Every file present with its pinned size (cheap; the hash was checked
    /// when it was downloaded).
    pub fn installed(&self, models_dir: &Path) -> bool {
        self.files.iter().all(|f| {
            std::fs::metadata(self.file_path(models_dir, f)).is_ok_and(|m| m.is_file() && m.len() == f.size)
        })
    }

    /// Point the local engine at the downloaded model.
    pub fn apply_to(&self, prefs: &mut Prefs, models_dir: &Path) {
        let p = Some(self.dest_path(models_dir).display().to_string());
        match self.engine {
            ModelEngine::Whisper => {
                prefs.asr_engine = AsrEngineChoice::Whisper;
                prefs.model_path = p;
            }
            ModelEngine::Parakeet => {
                prefs.asr_engine = AsrEngineChoice::Parakeet;
                prefs.parakeet_model_dir = p;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileView {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// `model_catalog` row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogView {
    pub id: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub engine: ModelEngine,
    pub recommended: bool,
    /// Bytes, all files together.
    pub size: u64,
    pub page: &'static str,
    /// Full path of the file / folder it is saved as.
    pub dest: String,
    pub installed: bool,
    /// This build can download it (feature `download`) and run its engine.
    pub available: bool,
    pub files: Vec<FileView>,
}

/// `engine_built`: whether this build can run the entry's engine.
pub fn catalog_view(
    models_dir: &Path,
    can_download: bool,
    engine_built: impl Fn(ModelEngine) -> bool,
) -> Vec<CatalogView> {
    CATALOG
        .iter()
        .map(|e| CatalogView {
            id: e.id,
            title: e.title,
            desc: e.desc,
            engine: e.engine,
            recommended: e.recommended,
            size: e.size(),
            page: e.page,
            dest: e.dest_path(models_dir).display().to_string(),
            installed: e.installed(models_dir),
            available: can_download && engine_built(e.engine),
            files: e
                .files
                .iter()
                .map(|f| FileView {
                    name: f.name,
                    url: f.url,
                    size: f.size,
                    sha256: f.sha256,
                })
                .collect(),
        })
        .collect()
}

// ------------------------------------------------------------ progress

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Downloading,
    Verifying,
    Done,
    Error,
    Cancelled,
}

/// The `model:download` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub id: String,
    pub received: u64,
    pub total: u64,
    pub state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The model's path, on `done`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Opens a download. The real one is `HttpFetch`; tests use in-memory bytes.
pub trait Fetch {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, String>;
}

/// https and a Hugging Face host (the catalogue's start URLs).
pub fn check_download_url(url: &str) -> Result<(), String> {
    let host = url
        .strip_prefix("https://")
        .and_then(|r| r.split(['/', '?', '#']).next())
        .unwrap_or_default();
    if host == "huggingface.co" {
        Ok(())
    } else {
        Err(format!("not a model download URL: {url}"))
    }
}

/// Hosts a download may be redirected to: Hugging Face and its CDN.
pub fn redirect_host_ok(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "huggingface.co" || h.ends_with(".huggingface.co") || h.ends_with(".hf.co")
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Why [`run`] stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Cancelled,
    Failed(String),
}

/// How often progress is reported while bytes arrive.
const TICK: Duration = Duration::from_millis(250);

/// Download `e` into `models_dir` (see the module docs). Emits progress
/// through `emit`, but not the final `done` / `error` / `cancelled`: the
/// caller sends that once prefs are updated.
pub fn run(
    e: &CatalogEntry,
    models_dir: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Stop> {
    let total = e.size();
    let id = e.id.to_string();
    let mut report = |received: u64, state: State| {
        emit(Progress {
            id: id.clone(),
            received,
            total,
            state,
            error: None,
            path: None,
        })
    };
    let io = |what: &str, err: std::io::Error| Stop::Failed(format!("{what}: {err}"));
    let dir = if e.is_dir() {
        e.dest_path(models_dir)
    } else {
        models_dir.to_path_buf()
    };
    std::fs::create_dir_all(&dir).map_err(|er| io("could not create the models folder", er))?;
    let mut done: u64 = 0;
    report(0, State::Downloading);
    for f in e.files {
        let target = e.file_path(models_dir, f);
        // Already there and intact (an earlier, cancelled try): keep it.
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == f.size) {
            report(done, State::Verifying);
            if sha256_file(&target).is_ok_and(|h| h == f.sha256) {
                done += f.size;
                report(done, State::Downloading);
                continue;
            }
        }
        let part = target.with_file_name(format!("{}.part", f.name));
        let got = fetch_file(f, &part, fetch, cancel, &mut |n| report(done + n, State::Downloading));
        let digest = match got {
            Ok(d) => d,
            Err(s) => {
                let _ = std::fs::remove_file(&part);
                return Err(s);
            }
        };
        report(done + f.size, State::Verifying);
        if digest != f.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(Stop::Failed(format!(
                "{} did not match its fingerprint, so it was deleted",
                f.name
            )));
        }
        std::fs::rename(&part, &target).map_err(|er| {
            let _ = std::fs::remove_file(&part);
            io("could not save the file", er)
        })?;
        done += f.size;
    }
    Ok(e.dest_path(models_dir))
}

/// Stream one file into `part`, hashing it; returns the hex SHA-256.
fn fetch_file(
    f: &ModelFile,
    part: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    tick: &mut dyn FnMut(u64),
) -> Result<String, Stop> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Stop::Cancelled);
    }
    check_download_url(f.url).map_err(Stop::Failed)?;
    let mut src = fetch.open(f.url).map_err(Stop::Failed)?;
    let mut out = std::fs::File::create(part).map_err(|e| Stop::Failed(format!("could not write {}: {e}", f.name)))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 << 10];
    let mut n: u64 = 0;
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Stop::Cancelled);
        }
        let k = src
            .read(&mut buf)
            .map_err(|e| Stop::Failed(format!("download of {} stopped: {e}", f.name)))?;
        if k == 0 {
            break;
        }
        n += k as u64;
        if n > f.size {
            return Err(Stop::Failed(format!("{} is larger than expected", f.name)));
        }
        h.update(&buf[..k]);
        out.write_all(&buf[..k])
            .map_err(|e| Stop::Failed(format!("could not write {}: {e}", f.name)))?;
        if last.elapsed() >= TICK {
            last = Instant::now();
            tick(n);
        }
    }
    if n != f.size {
        return Err(Stop::Failed(format!(
            "{} ended early ({n} of {} bytes)",
            f.name, f.size
        )));
    }
    out.sync_all()
        .map_err(|e| Stop::Failed(format!("could not write {}: {e}", f.name)))?;
    Ok(hex(&h.finalize()))
}

// ------------------------------------------------------------ one at a time

/// The running download (at most one).
#[derive(Default)]
pub struct Downloads {
    active: Mutex<Option<(String, Arc<AtomicBool>)>>,
}

impl Downloads {
    /// Claim the slot for `id`; its cancel flag.
    pub fn begin(&self, id: &str) -> Result<Arc<AtomicBool>, String> {
        let mut a = self.active.lock().unwrap();
        if let Some((other, _)) = a.as_ref() {
            return Err(if other == id {
                "this model is already downloading".into()
            } else {
                "another model is downloading; wait for it or cancel it".into()
            });
        }
        let flag = Arc::new(AtomicBool::new(false));
        *a = Some((id.to_string(), flag.clone()));
        Ok(flag)
    }

    /// Ask the download of `id` to stop; false when it isn't running.
    pub fn cancel(&self, id: &str) -> bool {
        match self.active.lock().unwrap().as_ref() {
            Some((a, flag)) if a == id => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    pub fn finish(&self, id: &str) {
        let mut a = self.active.lock().unwrap();
        if a.as_ref().is_some_and(|(x, _)| x == id) {
            *a = None;
        }
    }

    pub fn running(&self) -> Option<String> {
        self.active.lock().unwrap().as_ref().map(|(id, _)| id.clone())
    }
}

// ------------------------------------------------------------ HTTP

#[cfg(feature = "download")]
pub struct HttpFetch {
    client: reqwest::blocking::Client,
}

#[cfg(feature = "download")]
impl HttpFetch {
    pub fn new() -> Result<Self, String> {
        let policy = reqwest::redirect::Policy::custom(|a| {
            let ok = a.url().scheme() == "https" && a.url().host_str().is_some_and(redirect_host_ok);
            if a.previous().len() >= 8 {
                a.error("too many redirects")
            } else if ok {
                a.follow()
            } else {
                a.error("redirected outside Hugging Face")
            }
        });
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .redirect(policy)
            .connect_timeout(Duration::from_secs(20))
            // Blocking reqwest applies this to each read too: a stalled
            // download fails after 60 s without data.
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("murmur/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(HttpFetch { client })
    }
}

#[cfg(feature = "download")]
impl Fetch for HttpFetch {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, String> {
        check_download_url(url)?;
        let r = self
            .client
            .get(url)
            .send()
            .map_err(|e| format!("could not reach Hugging Face: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("Hugging Face answered {}", r.status()));
        }
        Ok(Box::new(r))
    }
}

pub const CAN_DOWNLOAD: bool = cfg!(feature = "download");

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn catalogue_is_pinned_to_hugging_face_commits() {
        assert!((2..=4).contains(&CATALOG.len()));
        let mut ids = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(ids.insert(e.id), "duplicate id {}", e.id);
            assert!(e.page.starts_with("https://huggingface.co/"));
            crate::shell::check_external_url(e.page).unwrap();
            for f in e.files {
                check_download_url(f.url).unwrap();
                // .../resolve/<40-hex commit>/<name>: never a moving branch.
                let rev = f.url.split("/resolve/").nth(1).unwrap().split('/').next().unwrap();
                assert!(rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()), "{}", f.url);
                assert!(f.url.ends_with(&format!("/{}", f.name)));
                assert_eq!(f.sha256.len(), 64);
                assert!(f.sha256.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
                assert!(f.size > 0);
            }
        }
        // The defaults the engine looks for without a configured path.
        assert_eq!(entry("whisper-base").unwrap().dest, crate::core_adapter::DEFAULT_MODEL_FILE);
        assert_eq!(entry("parakeet-tdt-0.6b-v2-int8").unwrap().dest, crate::core_adapter::DEFAULT_PARAKEET_DIR);
        // The Parakeet loader needs encoder, decoder, joiner and tokens.txt.
        let pk: Vec<_> = entry("parakeet-tdt-0.6b-v2-int8").unwrap().files.iter().map(|f| f.name).collect();
        for want in ["encoder", "decoder", "joiner", "tokens.txt"] {
            assert!(pk.iter().any(|n| n.starts_with(want)), "{want}");
        }
    }

    #[test]
    fn download_and_redirect_hosts() {
        assert!(check_download_url("http://huggingface.co/x").is_err());
        assert!(check_download_url("https://huggingface.co.evil.com/x").is_err());
        assert!(check_download_url("https://evil.com/huggingface.co").is_err());
        assert!(redirect_host_ok("us.aws.cdn.hf.co"));
        assert!(redirect_host_ok("cdn-lfs.huggingface.co"));
        assert!(!redirect_host_ok("hf.co.evil.com"));
        assert!(!redirect_host_ok("evilhf.co"));
    }

    /// Serves fixed bytes per URL; counts opens.
    struct Mem {
        files: HashMap<&'static str, Vec<u8>>,
        opens: Mutex<Vec<String>>,
        cancel_after_open: Option<Arc<AtomicBool>>,
    }

    impl Fetch for Mem {
        fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, String> {
            self.opens.lock().unwrap().push(url.to_string());
            if let Some(c) = &self.cancel_after_open {
                c.store(true, Ordering::SeqCst);
            }
            let b = self.files.get(url).cloned().ok_or("404")?;
            Ok(Box::new(std::io::Cursor::new(b)))
        }
    }

    fn sha(b: &[u8]) -> &'static str {
        Box::leak(hex(&Sha256::digest(b)).into_boxed_str())
    }

    fn entry_of(engine: ModelEngine, files: Vec<(&'static str, &'static [u8])>) -> (CatalogEntry, Mem) {
        let mut map = HashMap::new();
        let fs: Vec<ModelFile> = files
            .into_iter()
            .map(|(name, b)| {
                let url: &'static str = Box::leak(format!("https://huggingface.co/r/resolve/{}/{name}", "a".repeat(40)).into_boxed_str());
                map.insert(url, b.to_vec());
                ModelFile {
                    name,
                    url,
                    size: b.len() as u64,
                    sha256: sha(b),
                }
            })
            .collect();
        let e = CatalogEntry {
            id: "t",
            title: "T",
            desc: "",
            engine,
            dest: if engine == ModelEngine::Parakeet { "pk-dir" } else { "m.bin" },
            page: "https://huggingface.co/r",
            recommended: false,
            files: Box::leak(fs.into_boxed_slice()),
        };
        (
            e,
            Mem {
                files: map,
                opens: Mutex::new(vec![]),
                cancel_after_open: None,
            },
        )
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("flow-dl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn files_in(d: &Path) -> Vec<String> {
        // '/'-separated on every OS so the expectations hold on Windows too.
        let mut v: Vec<String> =
            walk(d).into_iter().map(|p| p.strip_prefix(d).unwrap().display().to_string().replace('\\', "/")).collect();
        v.sort();
        v
    }

    fn walk(d: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }

    #[test]
    fn a_single_file_lands_only_after_its_hash_matches() {
        let (e, mem) = entry_of(ModelEngine::Whisper, vec![("m.bin", b"ggml model bytes")]);
        let d = tmp("single");
        let mut seen = vec![];
        let p = run(&e, &d, &mem, &AtomicBool::new(false), &mut |p| seen.push(p.state)).unwrap();
        assert_eq!(p, d.join("m.bin"));
        assert_eq!(std::fs::read(&p).unwrap(), b"ggml model bytes");
        assert_eq!(files_in(&d), vec!["m.bin"]);
        assert_eq!(seen.first(), Some(&State::Downloading));
        assert!(seen.contains(&State::Verifying));
        assert!(e.installed(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_wrong_hash_deletes_the_file() {
        let (mut e, mem) = entry_of(ModelEngine::Whisper, vec![("m.bin", b"tampered bytes!!")]);
        let mut fs: Vec<ModelFile> = e.files.iter().map(|f| ModelFile { sha256: sha(b"the real bytes!!"), ..*f }).collect();
        e.files = Box::leak(std::mem::take(&mut fs).into_boxed_slice());
        let d = tmp("badhash");
        let r = run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {});
        assert!(matches!(&r, Err(Stop::Failed(m)) if m.contains("fingerprint")), "{r:?}");
        assert!(files_in(&d).is_empty(), "{:?}", files_in(&d));
        assert!(!e.installed(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn short_or_long_bodies_fail() {
        for body in [&b"short"[..], &b"much too long body"[..]] {
            let (mut e, mut mem) = entry_of(ModelEngine::Whisper, vec![("m.bin", b"exactly 12 b")]);
            let url = e.files[0].url;
            mem.files.insert(url, body.to_vec());
            e.files = Box::leak(vec![ModelFile { ..e.files[0] }].into_boxed_slice());
            let d = tmp("size");
            assert!(matches!(run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {}), Err(Stop::Failed(_))));
            assert!(files_in(&d).is_empty());
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn parakeet_fills_its_folder_and_a_retry_skips_verified_files() {
        let (e, mut mem) = entry_of(
            ModelEngine::Parakeet,
            vec![("encoder.int8.onnx", b"enc"), ("decoder.int8.onnx", b"dec"), ("tokens.txt", b"tok")],
        );
        let d = tmp("parakeet");
        // First try: the last file is missing upstream.
        let tok = mem.files.remove(e.files[2].url).unwrap();
        assert!(run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {}).is_err());
        assert_eq!(files_in(&d), vec!["pk-dir/decoder.int8.onnx", "pk-dir/encoder.int8.onnx"]);
        assert!(!e.installed(&d));
        // Retry: only tokens.txt is fetched.
        mem.files.insert(e.files[2].url, tok);
        mem.opens.lock().unwrap().clear();
        let p = run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(p, d.join("pk-dir"));
        assert_eq!(*mem.opens.lock().unwrap(), vec![e.files[2].url.to_string()]);
        assert!(e.installed(&d));
        let mut prefs = Prefs::default();
        e.apply_to(&mut prefs, &d);
        assert_eq!(prefs.asr_engine, AsrEngineChoice::Parakeet);
        assert_eq!(prefs.parakeet_model_dir.as_deref(), Some(d.join("pk-dir").display().to_string().as_str()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cancel_stops_and_leaves_no_part_file() {
        let (e, mut mem) = entry_of(ModelEngine::Whisper, vec![("m.bin", b"model")]);
        let flag = Arc::new(AtomicBool::new(false));
        mem.cancel_after_open = Some(flag.clone());
        let d = tmp("cancel");
        assert_eq!(run(&e, &d, &mem, &flag, &mut |_| {}), Err(Stop::Cancelled));
        assert!(files_in(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn whisper_download_points_prefs_at_the_file() {
        let e = entry("whisper-small-q5_1").unwrap();
        let mut prefs = Prefs {
            asr_engine: AsrEngineChoice::Parakeet,
            ..Prefs::default()
        };
        e.apply_to(&mut prefs, Path::new("/m"));
        assert_eq!(prefs.asr_engine, AsrEngineChoice::Whisper);
        assert_eq!(prefs.model_path.as_deref(), Some(Path::new("/m").join("ggml-small-q5_1.bin").display().to_string().as_str()));
    }

    #[test]
    fn one_download_at_a_time() {
        let d = Downloads::default();
        let flag = d.begin("a").unwrap();
        assert!(d.begin("a").is_err());
        assert!(d.begin("b").is_err());
        assert!(!d.cancel("b"));
        assert!(d.cancel("a"));
        assert!(flag.load(Ordering::SeqCst));
        d.finish("a");
        assert!(d.running().is_none());
        assert!(d.begin("b").is_ok());
    }

    #[test]
    fn catalogue_view_and_event_shape() {
        let d = tmp("view");
        let v = catalog_view(&d, true, |e| e == ModelEngine::Whisper);
        assert_eq!(v.len(), CATALOG.len());
        let pk = v.iter().find(|r| r.engine == ModelEngine::Parakeet).unwrap();
        assert!(!pk.available && !pk.installed);
        assert_eq!(pk.size, 661_190_513);
        assert!(v.iter().filter(|r| r.engine == ModelEngine::Whisper).all(|r| r.available));
        assert!(catalog_view(&d, false, |_| true).iter().all(|r| !r.available));
        let j = serde_json::to_value(Progress {
            id: "whisper-base".into(),
            received: 1,
            total: 2,
            state: State::Downloading,
            error: None,
            path: None,
        })
        .unwrap();
        assert_eq!(j, serde_json::json!({"id": "whisper-base", "received": 1, "total": 2, "state": "downloading"}));
    }
}
