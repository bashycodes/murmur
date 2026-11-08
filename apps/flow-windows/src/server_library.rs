//! Models from the configured server's library (D41; server side D40).
//!
//! When a server is set, model downloads come from its library by default
//! and never from Hugging Face (no fallback); the user can pick Hugging
//! Face instead ("Download from", D79). With no server, the D31 catalogue
//! downloads from Hugging Face as before ([`Origin`]).
//!
//! * A server model tagged with a catalogue id is that catalogue model:
//!   every file must be on the server with the size and SHA-256 pinned in
//!   this build, or it is refused ("differs from the built-in fingerprint").
//!   It installs where the catalogue puts it ([`download_catalogue`]).
//! * Any other model is custom: checked against the server's listed
//!   SHA-256 (trusted through the key and the TLS pin), saved under
//!   `<models>\server\<id>\` and pointed at by the prefs of its slot
//!   ([`install_custom`]). Its `kind` + `engine` pick the slot ([`plan`]);
//!   one the app can't load is listed with the reason and can't be chosen.
//!
//! The HTTP client is `flow_remote::library` (shared with Android).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use flow_protocol::library::{LibraryFile, LibraryModel, ModelKind};
use flow_remote::library::{ErrorKind, LibraryClient, LibraryError};
use flow_router::ServerConfig;
use serde::Serialize;

use crate::model_download::{
    self as md, CatalogEntry, ModelEngine, ModelFile, Progress, State, Stop,
};
use crate::prefs::{AsrEngineChoice, HebrewModel, ModelSource, Prefs};

/// Where model downloads come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// No server set, or the user picked Hugging Face: the D31 catalogue.
    HuggingFace,
    /// A server is set and picked (the default): its library only.
    Server(ServerConfig),
}

impl Origin {
    /// `source` (the "Download from" choice, D79) counts only while a
    /// server is set.
    pub fn of(server: Option<ServerConfig>, source: ModelSource) -> Self {
        match server {
            Some(s) if s.is_configured() && source == ModelSource::Server => Origin::Server(s),
            _ => Origin::HuggingFace,
        }
    }

    pub fn is_server(&self) -> bool {
        matches!(self, Origin::Server(_))
    }
}

/// `model_catalog` rows for `origin`: from the server, a catalogue model
/// downloads through the library client whenever this build runs its
/// engine (whether the server has it is the server listing's answer, shown
/// as "Not on your server"); from Hugging Face, the rows are unchanged.
pub fn catalog_for(mut rows: Vec<md::CatalogView>, origin: &Origin) -> Vec<md::CatalogView> {
    if origin.is_server() {
        for r in &mut rows {
            r.from_server = true;
            r.available = r.importable;
        }
    }
    rows
}

/// The app slot a library model fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// Speech to text, every language: whisper.cpp ggml file.
    SttWhisper,
    /// Speech to text, every language: Parakeet (sherpa-onnx) folder.
    SttParakeet,
    /// The Hebrew route: sherpa-onnx streaming transducer folder.
    HebrewZipformer,
    /// The Hebrew route: Parakeet folder.
    HebrewParakeet,
    /// The Hebrew route: whisper.cpp ggml file (ivrit turbo; GPU when the
    /// Vulkan plugin loads).
    HebrewWhisper,
    /// AI cleanup: llama.cpp GGUF file.
    Llm,
    /// Language identification (auto-detect → Hebrew): whisper ggml file.
    Lid,
}

impl Slot {
    pub fn engine(self) -> ModelEngine {
        match self {
            Slot::SttWhisper | Slot::Lid | Slot::HebrewWhisper => ModelEngine::Whisper,
            Slot::SttParakeet | Slot::HebrewParakeet => ModelEngine::Parakeet,
            Slot::HebrewZipformer => ModelEngine::SherpaOnline,
            Slot::Llm => ModelEngine::Llm,
        }
    }

    /// A single model file (else a folder of sherpa-onnx files).
    fn file_ext(self) -> Option<&'static str> {
        match self {
            Slot::SttWhisper | Slot::Lid | Slot::HebrewWhisper => Some(".bin"),
            Slot::Llm => Some(".gguf"),
            _ => None,
        }
    }

    fn of_catalogue(e: &CatalogEntry) -> Slot {
        match (e.engine, e.language.is_some()) {
            (ModelEngine::Llm, _) => Slot::Llm,
            (ModelEngine::SherpaOnline, _) => Slot::HebrewZipformer,
            (ModelEngine::Parakeet, true) => Slot::HebrewParakeet,
            (ModelEngine::Parakeet, false) => Slot::SttParakeet,
            (ModelEngine::Whisper, true) => Slot::HebrewWhisper,
            (ModelEngine::Whisper, false) => Slot::SttWhisper,
        }
    }
}

/// How a library model can be used here.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub slot: Slot,
    /// The catalogue model it is a copy of (pins checked).
    pub catalogue: Option<&'static CatalogEntry>,
}

/// The slot of `m`, or why it can't be used in this build.
pub fn plan(m: &LibraryModel, engine_built: &dyn Fn(ModelEngine) -> bool) -> Result<Plan, String> {
    if let Some(e) = m.catalogue_id.as_deref().and_then(md::entry) {
        pins_match(e, m)?;
        let slot = Slot::of_catalogue(e);
        if !engine_built(slot.engine()) {
            return Err(format!(
                "this build can't run {} models",
                engine_name(slot.engine())
            ));
        }
        return Ok(Plan {
            slot,
            catalogue: Some(e),
        });
    }
    let engine = m.engine.as_deref().unwrap_or("");
    let slot = match (m.kind, engine) {
        (_, "") => {
            return Err(format!(
                "its tag has no engine (add engine = {} to model.toml)",
                flow_protocol::library::engines_hint()
            ))
        }
        (ModelKind::Stt, "whisper") => Slot::SttWhisper,
        (ModelKind::Stt, "parakeet") => Slot::SttParakeet,
        (ModelKind::Stt, "sherpa_online") => {
            return Err(
                "this app runs streaming (sherpa_online) models only as the Hebrew model".into(),
            )
        }
        (ModelKind::Hebrew, "sherpa_online") => Slot::HebrewZipformer,
        (ModelKind::Hebrew, "parakeet") => Slot::HebrewParakeet,
        (ModelKind::Hebrew, "whisper") => Slot::HebrewWhisper,
        (ModelKind::Hebrew, e) => {
            return Err(format!(
                "the Hebrew model must be sherpa_online, parakeet or whisper, not {e}"
            ))
        }
        (ModelKind::Llm, "gguf") => Slot::Llm,
        (ModelKind::Lid, "whisper") => Slot::Lid,
        (ModelKind::Lid, e) => {
            return Err(format!("language detection takes a whisper model, not {e}"))
        }
        (ModelKind::Vad, _) => return Err("this app has no slot for a voice-activity model".into()),
        (_, e) => return Err(format!("this app can't load the {e} engine")),
    };
    match slot.file_ext() {
        Some(ext) => {
            if m.files.iter().filter(|f| f.name.ends_with(ext)).count() != 1 {
                return Err(format!("expected exactly one {ext} file"));
            }
        }
        None => {
            let top = |pre: &str| {
                m.files.iter().any(|f| {
                    !f.name.contains('/') && f.name.starts_with(pre) && f.name.ends_with(".onnx")
                })
            };
            let tokens = m.files.iter().any(|f| f.name == "tokens.txt");
            if !(tokens && top("encoder") && top("decoder") && top("joiner")) {
                return Err(
                    "expected encoder, decoder and joiner .onnx files and tokens.txt".into(),
                );
            }
        }
    }
    if !engine_built(slot.engine()) {
        return Err(format!(
            "this build can't run {} models",
            engine_name(slot.engine())
        ));
    }
    Ok(Plan {
        slot,
        catalogue: None,
    })
}

/// Server model `m` holds every file of catalogue entry `e` with its
/// pinned size and SHA-256 (the extras are optional).
pub fn pins_match(e: &CatalogEntry, m: &LibraryModel) -> Result<(), String> {
    for f in e.files {
        match m.files.iter().find(|s| s.name == f.name) {
            None => return Err(format!("the server's copy has no {}", f.name)),
            Some(s) if s.sha256 != f.sha256 || s.size != f.size => {
                return Err(format!(
                    "the server's {} differs from the fingerprint built into this version",
                    f.name
                ))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn engine_name(e: ModelEngine) -> &'static str {
    match e {
        ModelEngine::Whisper => "Whisper",
        ModelEngine::Parakeet => "Parakeet",
        ModelEngine::SherpaOnline => "streaming (sherpa-onnx)",
        ModelEngine::Llm => "AI cleanup (GGUF)",
    }
}

/// Folder of custom server model `id`: `<models>\server\<id>`.
pub fn custom_dir(models_dir: &Path, id: &str) -> PathBuf {
    models_dir.join("server").join(id)
}

/// What the prefs point at for `m` in `slot`: its file or its folder.
pub fn target(m: &LibraryModel, p: &Plan, models_dir: &Path) -> PathBuf {
    if let Some(e) = p.catalogue {
        return e.dest_path(models_dir);
    }
    let dir = custom_dir(models_dir, &m.id);
    match p.slot.file_ext() {
        Some(ext) => match m.files.iter().find(|f| f.name.ends_with(ext)) {
            Some(f) => join_rel(&dir, &f.name),
            None => dir,
        },
        None => dir,
    }
}

fn join_rel(dir: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part))
}

fn present(p: &Path, size: u64) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() == size)
}

/// Every file of custom model `m` is on disk with its listed size.
fn custom_installed(m: &LibraryModel, models_dir: &Path) -> bool {
    let dir = custom_dir(models_dir, &m.id);
    m.files
        .iter()
        .all(|f| present(&join_rel(&dir, &f.name), f.size))
}

/// Point the prefs at `path` for `slot`.
pub fn apply(slot: Slot, path: &Path, prefs: &mut Prefs) {
    let p = Some(path.display().to_string());
    match slot {
        Slot::SttWhisper => {
            prefs.asr_engine = AsrEngineChoice::Whisper;
            prefs.model_path = p;
        }
        Slot::SttParakeet => {
            prefs.asr_engine = AsrEngineChoice::Parakeet;
            prefs.parakeet_model_dir = p;
        }
        Slot::HebrewZipformer => {
            prefs.hebrew_model = HebrewModel::Zipformer;
            prefs.hebrew_model_dir = p;
        }
        Slot::HebrewParakeet => {
            prefs.hebrew_model = HebrewModel::ParakeetHe;
            prefs.hebrew_model_dir = p;
        }
        Slot::HebrewWhisper => {
            prefs.hebrew_model = HebrewModel::IvritTurbo;
            prefs.hebrew_model_dir = p;
        }
        Slot::Llm => {
            prefs.llm_enabled = true;
            prefs.llm_model_path = p;
        }
        Slot::Lid => prefs.lid_model_path = p,
    }
}

/// The prefs use `path` for `slot`.
pub fn in_use(slot: Slot, path: &Path, prefs: &Prefs, appdata: Option<&str>) -> bool {
    match slot {
        Slot::SttWhisper => {
            prefs.asr_engine == AsrEngineChoice::Whisper
                && prefs.resolved_model_path(appdata) == path
        }
        Slot::SttParakeet => {
            prefs.asr_engine == AsrEngineChoice::Parakeet
                && prefs.resolved_parakeet_dir(appdata) == path
        }
        Slot::HebrewZipformer => {
            prefs.hebrew_model == HebrewModel::Zipformer
                && prefs.resolved_hebrew_dir(appdata).as_deref() == Some(path)
        }
        Slot::HebrewParakeet => {
            prefs.hebrew_model == HebrewModel::ParakeetHe
                && prefs.resolved_hebrew_dir(appdata).as_deref() == Some(path)
        }
        Slot::HebrewWhisper => {
            prefs.hebrew_model == HebrewModel::IvritTurbo
                && prefs.resolved_hebrew_dir(appdata).as_deref() == Some(path)
        }
        Slot::Llm => prefs.llm_enabled && prefs.resolved_llm_path(appdata) == path,
        Slot::Lid => prefs.resolved_lid_path(appdata) == path,
    }
}

/// `server_models` row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServerModelView {
    pub id: String,
    pub title: String,
    pub kind: ModelKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// The catalogue model it is a copy of; its download goes through
    /// `model_download_start` with this id (and shows on that row too).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalogue_id: Option<String>,
    /// Bytes, all files.
    pub size: u64,
    pub files: usize,
    /// Where it fills in (absent when it can't be used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<Slot>,
    /// Why it can't be used here (then `slot` is absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// File or folder it is saved as.
    pub dest: String,
    pub installed: bool,
    /// The prefs use it.
    pub in_use: bool,
}

/// `server_models`: the library or why it could not be read.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ServerLibraryView {
    /// A server is set (downloads come from it, never Hugging Face).
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub models: Vec<ServerModelView>,
}

pub fn views(
    models: &[LibraryModel],
    models_dir: &Path,
    prefs: &Prefs,
    appdata: Option<&str>,
    engine_built: &dyn Fn(ModelEngine) -> bool,
) -> Vec<ServerModelView> {
    models
        .iter()
        .map(|m| {
            let planned = plan(m, engine_built);
            let (slot, reason, dest, installed, used) = match &planned {
                Ok(p) => {
                    let t = target(m, p, models_dir);
                    let installed = match p.catalogue {
                        Some(e) => e.installed(models_dir),
                        None => custom_installed(m, models_dir),
                    };
                    let used = installed && in_use(p.slot, &t, prefs, appdata);
                    (Some(p.slot), None, t, installed, used)
                }
                Err(r) => (
                    None,
                    Some(r.clone()),
                    custom_dir(models_dir, &m.id),
                    false,
                    false,
                ),
            };
            ServerModelView {
                id: m.id.clone(),
                title: m.title.clone(),
                kind: m.kind,
                language: m.language.clone(),
                engine: m.engine.clone(),
                catalogue_id: m.catalogue_id.clone().filter(|c| md::entry(c).is_some()),
                size: m.size(),
                files: m.files.len(),
                slot,
                reason,
                dest: dest.display().to_string(),
                installed,
                in_use: used,
            }
        })
        .collect()
}

fn stop_of(e: LibraryError) -> Stop {
    match e.kind {
        ErrorKind::Cancelled => Stop::Cancelled,
        _ => Stop::Failed(e.message),
    }
}

/// A catalogue model's files from the server's library: each pinned file
/// is looked up by its pin (SHA-256) in the listing, the model's own copy
/// first, then any other model (the Hebrew models' language-ID extra can
/// come from a `lid` model). Downloads are checked against the pins.
pub struct ServerSource<'a> {
    pub client: &'a LibraryClient,
    /// The server model that is a copy of the catalogue entry.
    pub model: &'a LibraryModel,
    pub all: &'a [LibraryModel],
}

impl md::FileSource for ServerSource<'_> {
    fn fetch(
        &self,
        f: &ModelFile,
        target: &Path,
        part: &Path,
        cancel: &AtomicBool,
        tick: &mut dyn FnMut(u64),
    ) -> Result<(), Stop> {
        let pinned = |s: &LibraryFile| s.sha256 == f.sha256 && s.size == f.size;
        let found = std::iter::once(self.model)
            .chain(self.all.iter())
            .find_map(|m| m.files.iter().find(|s| pinned(s)).map(|s| (m, s)));
        let Some((m, s)) = found else {
            return Err(Stop::Failed(format!(
                "{} is not on your server with the fingerprint built into this version",
                f.name
            )));
        };
        // Expected = the pin (equal to the listing here, checked above).
        let want = LibraryFile {
            name: s.name.clone(),
            size: f.size,
            sha256: f.sha256.to_string(),
        };
        self.client
            .download(&m.id, &want, target, part, tick, &|| {
                cancel.load(Ordering::SeqCst)
            })
            .map_err(stop_of)
    }
}

/// Download catalogue model `e` from where downloads come from now: the
/// server's library when one is set (its copy must match the pins; no
/// Hugging Face fallback), else Hugging Face through `hf()`. `hf` is only
/// called for [`Origin::HuggingFace`].
pub fn download_catalogue(
    e: &CatalogEntry,
    models_dir: &Path,
    origin: &Origin,
    client_name: &str,
    hf: &dyn Fn() -> Result<Box<dyn md::Fetch + Send>, String>,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Stop> {
    let cfg = match origin {
        Origin::HuggingFace => {
            let fetch = hf().map_err(Stop::Failed)?;
            return md::run(e, models_dir, fetch.as_ref(), cancel, emit);
        }
        Origin::Server(cfg) => cfg,
    };
    let client = LibraryClient::new(cfg.clone(), client_name).map_err(stop_of)?;
    let list = client.list().map_err(stop_of)?.models;
    let copies: Vec<&LibraryModel> = list
        .iter()
        .filter(|m| m.catalogue_id.as_deref() == Some(e.id))
        .collect();
    if copies.is_empty() {
        return Err(Stop::Failed(format!(
            "{} is not on your server: add it to the server's library (docs/server-setup.md §10), or download it from Hugging Face instead",
            e.title
        )));
    }
    let model = copies
        .iter()
        .copied()
        .find(|m| pins_match(e, m).is_ok())
        .ok_or_else(|| {
            let why = pins_match(e, copies[0]).err().unwrap_or_default();
            Stop::Failed(format!("{} on your server can't be used: {why}", e.title))
        })?;
    let src = ServerSource {
        client: &client,
        model,
        all: &list,
    };
    md::run_from(e, models_dir, &src, cancel, emit)
}

/// Download custom server model `m` (planned as `p`, no catalogue entry)
/// into `<models>\server\<id>\`; returns the path its slot points at.
/// Progress uses id `server:<id>`.
pub fn install_custom(
    client: &LibraryClient,
    m: &LibraryModel,
    p: &Plan,
    models_dir: &Path,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Stop> {
    let total = m.size();
    let id = progress_id(&m.id);
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
    let dir = custom_dir(models_dir, &m.id);
    let mut done = 0u64;
    report(0, State::Downloading);
    for f in &m.files {
        if cancel.load(Ordering::SeqCst) {
            return Err(Stop::Cancelled);
        }
        let dest = join_rel(&dir, &f.name);
        let part = dest.with_file_name(format!(
            "{}.part",
            dest.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        client
            .download(
                &m.id,
                f,
                &dest,
                &part,
                &mut |n| report(done + n, State::Downloading),
                &|| cancel.load(Ordering::SeqCst),
            )
            .map_err(stop_of)?;
        done += f.size;
        report(done, State::Verifying);
    }
    Ok(target(m, p, models_dir))
}

/// The `model:download` id of a custom server model.
pub fn progress_id(id: &str) -> String {
    format!("server:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, sha: &str) -> LibraryFile {
        LibraryFile {
            name: name.into(),
            size,
            sha256: sha.into(),
        }
    }

    fn model(
        id: &str,
        kind: ModelKind,
        engine: Option<&str>,
        files: Vec<LibraryFile>,
    ) -> LibraryModel {
        LibraryModel {
            id: id.into(),
            title: id.into(),
            kind,
            language: None,
            catalogue_id: None,
            engine: engine.map(Into::into),
            files,
        }
    }

    fn sherpa_files() -> Vec<LibraryFile> {
        [
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
        ]
        .iter()
        .map(|n| file(n, 1, &"a".repeat(64)))
        .collect()
    }

    const ALL: &dyn Fn(ModelEngine) -> bool = &|_| true;

    #[test]
    fn kinds_and_engines_map_to_slots() {
        let one = |n: &str| vec![file(n, 1, &"a".repeat(64))];
        let cases = [
            (
                model("w", ModelKind::Stt, Some("whisper"), one("ggml-x.bin")),
                Slot::SttWhisper,
            ),
            (
                model("p", ModelKind::Stt, Some("parakeet"), sherpa_files()),
                Slot::SttParakeet,
            ),
            (
                model(
                    "z",
                    ModelKind::Hebrew,
                    Some("sherpa_online"),
                    sherpa_files(),
                ),
                Slot::HebrewZipformer,
            ),
            (
                model("h", ModelKind::Hebrew, Some("parakeet"), sherpa_files()),
                Slot::HebrewParakeet,
            ),
            (
                model("hw", ModelKind::Hebrew, Some("whisper"), one("ggml-he.bin")),
                Slot::HebrewWhisper,
            ),
            (
                model("l", ModelKind::Llm, Some("gguf"), one("m.gguf")),
                Slot::Llm,
            ),
            (
                model("d", ModelKind::Lid, Some("whisper"), one("ggml-tiny.bin")),
                Slot::Lid,
            ),
        ];
        for (m, want) in cases {
            assert_eq!(plan(&m, ALL).unwrap().slot, want, "{}", m.id);
        }
    }

    #[test]
    fn unusable_models_say_why() {
        let one = |n: &str| vec![file(n, 1, &"a".repeat(64))];
        let why = |m: LibraryModel| plan(&m, ALL).unwrap_err();
        assert!(
            why(model("v", ModelKind::Vad, Some("silero"), one("v.onnx")))
                .contains("voice-activity")
        );
        let no_engine = why(model("x", ModelKind::Stt, None, one("x.bin")));
        assert!(no_engine.contains("no engine"), "{no_engine}");
        for e in flow_protocol::library::ENGINES {
            assert!(no_engine.contains(&format!("\"{e}\"")), "{no_engine}");
        }
        assert!(why(model(
            "x",
            ModelKind::Stt,
            Some("ctranslate2"),
            one("x.bin")
        ))
        .contains("ctranslate2"));
        assert!(why(model(
            "x",
            ModelKind::Stt,
            Some("sherpa_online"),
            sherpa_files()
        ))
        .contains("Hebrew"));
        assert!(
            why(model("x", ModelKind::Lid, Some("parakeet"), sherpa_files())).contains("whisper")
        );
        assert!(why(model("x", ModelKind::Stt, Some("whisper"), vec![])).contains(".bin"));
        assert!(why(model(
            "x",
            ModelKind::Hebrew,
            Some("parakeet"),
            one("encoder.onnx")
        ))
        .contains("tokens.txt"));
        // An engine this build lacks.
        let m = model("w", ModelKind::Stt, Some("whisper"), one("ggml-x.bin"));
        assert!(plan(&m, &|e| e != ModelEngine::Whisper)
            .unwrap_err()
            .contains("can't run Whisper"));
    }

    #[test]
    fn catalogue_copies_are_held_to_the_built_in_pins() {
        let e = md::entry("whisper-base").unwrap();
        let f = &e.files[0];
        let mut m = model(
            "wb",
            ModelKind::Stt,
            Some("whisper"),
            vec![file(f.name, f.size, f.sha256)],
        );
        m.catalogue_id = Some("whisper-base".into());
        let p = plan(&m, ALL).unwrap();
        assert_eq!(p.slot, Slot::SttWhisper);
        assert!(p.catalogue.is_some());
        m.files[0].sha256 = "0".repeat(64);
        assert!(plan(&m, ALL)
            .unwrap_err()
            .contains("differs from the fingerprint"));
        m.files.clear();
        assert!(plan(&m, ALL).unwrap_err().contains("has no ggml-base.bin"));
        // An unknown catalogue id is a custom model (listed hash).
        let mut c = model(
            "c",
            ModelKind::Llm,
            Some("gguf"),
            vec![file("c.gguf", 1, &"a".repeat(64))],
        );
        c.catalogue_id = Some("not-in-this-version".into());
        assert!(plan(&c, ALL).unwrap().catalogue.is_none());
        // The Hebrew catalogue models map to the Hebrew slots.
        assert_eq!(
            Slot::of_catalogue(md::entry(md::ZIPFORMER_HE_ID).unwrap()),
            Slot::HebrewZipformer
        );
        assert_eq!(
            Slot::of_catalogue(md::entry(md::PARAKEET_HE_ID).unwrap()),
            Slot::HebrewParakeet
        );
        assert_eq!(
            Slot::of_catalogue(md::entry(md::IVRIT_TURBO_ID).unwrap()),
            Slot::HebrewWhisper
        );
        assert_eq!(
            Slot::of_catalogue(md::entry("whisper-base").unwrap()),
            Slot::SttWhisper
        );
        assert_eq!(
            Slot::of_catalogue(md::entry("parakeet-tdt-0.6b-v2-int8").unwrap()),
            Slot::SttParakeet
        );
    }

    #[test]
    fn apply_points_each_slot_and_in_use_sees_it() {
        let d = std::env::temp_dir().join("flow-srvlib-apply");
        for slot in [
            Slot::SttWhisper,
            Slot::SttParakeet,
            Slot::HebrewZipformer,
            Slot::HebrewParakeet,
            Slot::HebrewWhisper,
            Slot::Llm,
            Slot::Lid,
        ] {
            let mut p = Prefs::default();
            let path = d.join(format!("{slot:?}"));
            assert!(!in_use(slot, &path, &p, None), "{slot:?}");
            apply(slot, &path, &mut p);
            assert!(in_use(slot, &path, &p, None), "{slot:?}");
        }
        // A catalogue Hebrew model takes the route back from a custom one.
        let mut p = Prefs::default();
        apply(Slot::HebrewParakeet, &d, &mut p);
        md::entry(md::ZIPFORMER_HE_ID).unwrap().apply_to(&mut p, &d);
        assert_eq!(p.hebrew_model, HebrewModel::Zipformer);
        assert_eq!(p.hebrew_model_dir, None);
    }

    #[test]
    fn views_list_sizes_slots_and_reasons() {
        let d = std::env::temp_dir().join("flow-srvlib-views");
        let _ = std::fs::remove_dir_all(&d);
        let ok = model(
            "my-llm",
            ModelKind::Llm,
            Some("gguf"),
            vec![file("m.gguf", 3, &"a".repeat(64))],
        );
        let bad = model(
            "vad",
            ModelKind::Vad,
            Some("silero"),
            vec![file("v.onnx", 5, &"a".repeat(64))],
        );
        let v = views(&[ok.clone(), bad], &d, &Prefs::default(), None, ALL);
        assert_eq!(v[0].slot, Some(Slot::Llm));
        assert_eq!(v[0].size, 3);
        assert!(!v[0].installed && !v[0].in_use);
        assert!(v[0].dest.ends_with("m.gguf"));
        assert_eq!(v[1].slot, None);
        assert!(v[1].reason.is_some());
        // Installed + used.
        let t = custom_dir(&d, "my-llm").join("m.gguf");
        std::fs::create_dir_all(t.parent().unwrap()).unwrap();
        std::fs::write(&t, b"xyz").unwrap();
        let mut prefs = Prefs::default();
        apply(Slot::Llm, &t, &mut prefs);
        let v = views(&[ok], &d, &prefs, None, ALL);
        assert!(v[0].installed && v[0].in_use);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn download_from_picks_the_origin_only_while_a_server_is_set() {
        use ModelSource::{HuggingFace as Hf, Server as Srv};
        assert_eq!(Origin::of(None, Srv), Origin::HuggingFace);
        assert_eq!(
            Origin::of(Some(ServerConfig::default()), Srv),
            Origin::HuggingFace,
            "empty host = not set"
        );
        let s = ServerConfig {
            host: "10.0.0.2".into(),
            ..Default::default()
        };
        assert_eq!(Origin::of(Some(s.clone()), Srv), Origin::Server(s.clone()));
        assert_eq!(Origin::of(Some(s.clone()), Hf), Origin::HuggingFace);
        assert_eq!(
            Prefs::default().model_source,
            Srv,
            "the server is the default"
        );
    }

    #[test]
    fn catalogue_rows_download_from_the_server_when_it_is_picked() {
        let d = std::env::temp_dir().join("flow-srvlib-catalog-for");
        let rows = || md::catalog_view(&d, false, |e| e != ModelEngine::Llm);
        let hf = catalog_for(rows(), &Origin::HuggingFace);
        assert!(
            hf.iter().all(|r| !r.from_server && !r.available),
            "no download feature"
        );
        let srv = catalog_for(
            rows(),
            &Origin::Server(ServerConfig {
                host: "10.0.0.2".into(),
                ..Default::default()
            }),
        );
        assert!(srv.iter().all(|r| r.from_server));
        // Downloadable from the server exactly when this build runs the engine.
        assert!(srv
            .iter()
            .all(|r| r.available == (r.engine != ModelEngine::Llm)));
    }

    #[test]
    fn a_picked_server_replaces_hugging_face_and_hugging_face_skips_the_server() {
        let hf_calls = std::sync::atomic::AtomicUsize::new(0);
        let hf = || -> Result<Box<dyn md::Fetch + Send>, String> {
            hf_calls.fetch_add(1, Ordering::SeqCst);
            Err("no".into())
        };
        let unreachable = ServerConfig {
            host: "127.0.0.1".into(),
            port: 1,
            connect_timeout_ms: 500,
            ..Default::default()
        };
        let e = md::entry("whisper-base").unwrap();
        let d = std::env::temp_dir().join("flow-srvlib-origin");
        // Server picked: Hugging Face is never asked, even when the server
        // can't be reached.
        let r = download_catalogue(
            e,
            &d,
            &Origin::of(Some(unreachable.clone()), ModelSource::Server),
            "t",
            &hf,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(r, Err(Stop::Failed(_))));
        assert_eq!(hf_calls.load(Ordering::SeqCst), 0);
        // Hugging Face picked with the server set: Hugging Face.
        let _ = download_catalogue(
            e,
            &d,
            &Origin::of(Some(unreachable), ModelSource::HuggingFace),
            "t",
            &hf,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(hf_calls.load(Ordering::SeqCst), 1);
    }
}
