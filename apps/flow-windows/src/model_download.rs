//! Model downloader (D9 exception on an explicit click, see
//! docs/DECISIONS.md D31): speech models and, since 0.11.0, AI cleanup models.
//!
//! * [`CATALOG`]: the few models the local engine already loads (a whisper.cpp
//!   ggml file, a sherpa-onnx folder, or a llama.cpp GGUF), each file with its URL
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

use crate::prefs::{AsrEngineChoice, HebrewModel, Prefs};
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
    /// sherpa-onnx online transducer (the Hebrew zipformer, D33/H2).
    #[serde(rename = "sherpa_online")]
    SherpaOnline,
    /// A GGUF instruct model for local AI cleanup (llama.cpp, `flow_llm`).
    Llm,
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
    /// File name (whisper, GGUF) or folder name (sherpa) in the models folder.
    pub dest: &'static str,
    /// The model's page, for "Download links" and the confirm dialog.
    pub page: &'static str,
    pub recommended: bool,
    /// AI cleanup only: the model recommended when local cleanup runs on a
    /// GPU (`llm_backend` probe; [`recommended_llm`]). `recommended` is then
    /// the pick for a CPU-only PC.
    pub recommended_gpu: bool,
    /// `None`: a default model (the engine for every language). `Some("he")`:
    /// the model of that language's route (D33 `languages.he`).
    pub language: Option<&'static str>,
    /// Licence and attribution the model's terms ask for (CC-BY-4.0).
    pub license: Option<&'static str>,
    pub files: &'static [ModelFile],
    /// Files saved in the models folder itself, next to `dest`, that come
    /// with this model: the Hebrew models bring the language-ID model
    /// (`ggml-tiny-q5_1.bin`, H6/D34) so auto-detect can route to them.
    /// Shared by entries; one already there and intact is not fetched again.
    pub extras: &'static [ModelFile],
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
macro_rules! zipformer_he_base {
    () => {
        "https://huggingface.co/ivrit-ai/zipformer2-66m-streaming-he/resolve/ee769a2a13b887f91572eec349458bfb0fe79e7a/sherpa-onnx/"
    };
}
macro_rules! parakeet_he_base {
    () => {
        "https://huggingface.co/krut42/voice-parakeet-he-int8/resolve/39de3b473bed626c7796c156d71adc66161ebfe1/"
    };
}
macro_rules! qwen05_base {
    () => {
        "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/9217f5db79a29953eb74d5343926648285ec7e67/"
    };
}
macro_rules! qwen15_base {
    () => {
        "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/91cad51170dc346986eccefdc2dd33a9da36ead9/"
    };
}
macro_rules! llama32_3b_base {
    () => {
        "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/"
    };
}
macro_rules! whisper_base {
    () => {
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/"
    };
}

/// Hashes computed 2026-10-30 by downloading each file once (they equal the
/// Hugging Face LFS ids at those commits; the Hebrew ones:
/// docs/findings/2026-10-30-hebrew-model-pins.md). The cleanup models (engine
/// `Llm`) are pinned from the HF API: docs/findings/2026-10-30-cleanup-model-pins.md.
/// More cleanup GGUFs are one entry each.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "parakeet-tdt-0.6b-v2-int8",
        title: "Parakeet TDT 0.6B",
        desc: "English · fastest",
        engine: ModelEngine::Parakeet,
        dest: crate::core_adapter::DEFAULT_PARAKEET_DIR,
        page: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        recommended: true,
        recommended_gpu: false,
        language: None,
        license: None,
        extras: &[],
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
        recommended_gpu: false,
        language: None,
        license: None,
        extras: &[],
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
        recommended_gpu: false,
        language: None,
        license: None,
        extras: &[],
        files: &[model_file!(whisper_base!(), "ggml-small-q5_1.bin", 190_085_487, "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb")],
    },
    CatalogEntry {
        id: ZIPFORMER_HE_ID,
        title: "Zipformer Hebrew",
        desc: "Hebrew · recommended, small and fast",
        engine: ModelEngine::SherpaOnline,
        dest: crate::core_adapter::ZIPFORMER_HE_DIR,
        page: "https://huggingface.co/ivrit-ai/zipformer2-66m-streaming-he",
        recommended: true,
        recommended_gpu: false,
        language: Some(crate::core_adapter::HEBREW),
        license: Some("CC-BY-4.0 · Yakov Kolani / ivrit.ai"),
        extras: LID_FILES,
        files: &[
            model_file!(zipformer_he_base!(), "encoder.int8.onnx", 70_478_964, "6c6febcb74a09435315a1d872a6539f6e7fc2c333533f14c74edc0e85c49ffcd"),
            model_file!(zipformer_he_base!(), "decoder.int8.onnx", 1_049_105, "feb03e7c8aa761593a9284a6fa1e91259be3702de75f630062256c1a59c5a906"),
            model_file!(zipformer_he_base!(), "joiner.int8.onnx", 771_805, "1d2c6d8e6dabc5eb52de52e95718e9bdfbffbfe7ead3808fb5ffd74e83bd9cae"),
            model_file!(zipformer_he_base!(), "tokens.txt", 17_086, "e4ebb5e72a6aeef47437a55fa2308ebbedb7c51c557de7a89bf965c23ea9af4a"),
        ],
    },
    CatalogEntry {
        id: PARAKEET_HE_ID,
        title: "Parakeet Hebrew",
        desc: "Hebrew · larger alternative",
        engine: ModelEngine::Parakeet,
        dest: crate::core_adapter::PARAKEET_HE_DIR,
        page: "https://huggingface.co/krut42/voice-parakeet-he-int8",
        recommended: false,
        recommended_gpu: false,
        language: Some(crate::core_adapter::HEBREW),
        license: Some("CC-BY-4.0 · krut42, based on thewh1teagle/parakeet-tdt-0.6b-v3-he"),
        extras: LID_FILES,
        files: &[
            model_file!(parakeet_he_base!(), "encoder.int8.onnx", 653_215_173, "7eaf80bc88fccd579a7d3950c25e50e4c7935e5b037ec2aa1eee29bca5ca3981"),
            model_file!(parakeet_he_base!(), "decoder.int8.onnx", 12_494_234, "3a1d0bbf3e393909095f826b7719fdaef42cd2e0b8b95c54518034ecc4b7d246"),
            model_file!(parakeet_he_base!(), "joiner.int8.onnx", 7_008_293, "056c5464464b38a9cbacdccb6a9ff2114919780badb5c48e0f0260af7f91ccf2"),
            model_file!(parakeet_he_base!(), "tokens.txt", 107_590, "55a91c862d0a63e8b9587a3264ced4fc76943e66098468b4638e6a6c5642e364"),
        ],
    },
    // AI cleanup (llama.cpp GGUF). The 1.5B one is saved under the name the
    // app loads when no cleanup model is set (D11); the 0.5B was the default
    // before and is still loaded when it is the only one there.
    CatalogEntry {
        id: "qwen2.5-0.5b-instruct-q4_k_m",
        title: "Qwen2.5 0.5B Instruct",
        desc: "Fastest · misses many fixes",
        engine: ModelEngine::Llm,
        dest: crate::core_adapter::LEGACY_LLM_FILE,
        page: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF",
        recommended: false,
        recommended_gpu: false,
        language: None,
        license: None,
        extras: &[],
        files: &[model_file!(qwen05_base!(), "qwen2.5-0.5b-instruct-q4_k_m.gguf", 491_400_032, "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db")],
    },
    CatalogEntry {
        id: "qwen2.5-1.5b-instruct-q4_k_m",
        title: "Qwen2.5 1.5B Instruct",
        desc: "The default · the pick without a GPU (about 1 s on a laptop CPU)",
        engine: ModelEngine::Llm,
        dest: crate::core_adapter::DEFAULT_LLM_FILE,
        page: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        recommended: true,
        recommended_gpu: false,
        language: None,
        license: None,
        extras: &[],
        files: &[model_file!(qwen15_base!(), "qwen2.5-1.5b-instruct-q4_k_m.gguf", 1_117_320_736, "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e")],
    },
    // Best local quality inside the budget on a GPU (Vulkan): 80 % on the
    // 88-suite, p50 0.62 s on an Arc 140V iGPU; too slow on a CPU.
    // docs/findings/2026-10-30-laptop-model-benchmark.md.
    CatalogEntry {
        id: LLAMA32_3B_ID,
        title: "Llama 3.2 3B Instruct",
        desc: "Most accurate · needs a GPU (Vulkan); slow on the CPU",
        engine: ModelEngine::Llm,
        dest: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        page: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF",
        recommended: false,
        recommended_gpu: true,
        language: None,
        license: Some("Llama 3.2 Community License · Built with Llama"),
        extras: &[],
        files: &[model_file!(llama32_3b_base!(), "Llama-3.2-3B-Instruct-Q4_K_M.gguf", 2_019_377_696, "6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff")],
    },
];

/// Catalogue id of Llama 3.2 3B (the GPU pick).
pub const LLAMA32_3B_ID: &str = "llama-3.2-3b-instruct-q4_k_m";

/// The cleanup model Settings recommends: Llama 3.2 3B when local cleanup
/// runs on a GPU, Qwen2.5 1.5B (the default) on a CPU-only PC (the 0.5B scores
/// 43 % on the 88-suite, the 1.5B 74 %; benchmark 2026-10-30).
pub fn recommended_llm(gpu: bool) -> &'static CatalogEntry {
    CATALOG
        .iter()
        .find(|e| e.is_llm() && if gpu { e.recommended_gpu } else { e.recommended })
        .expect("the catalogue has a CPU and a GPU cleanup pick")
}

/// The language-ID model the Hebrew entries bring along (H6/D34; hash and
/// size checked 2026-10-30 by downloading it once, equal to the HF LFS id).
const LID_FILES: &[ModelFile] = &[model_file!(
    whisper_base!(),
    "ggml-tiny-q5_1.bin",
    32_152_673,
    "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7"
)];

/// Catalogue ids of the Hebrew models (`prefs.hebrew_model`).
pub const ZIPFORMER_HE_ID: &str = "zipformer-he-int8";
pub const PARAKEET_HE_ID: &str = "parakeet-tdt-0.6b-v3-he-int8";

pub fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

impl CatalogEntry {
    /// Bytes of everything a download fetches (model + extras).
    pub fn size(&self) -> u64 {
        self.all_files().map(|(f, _)| f.size).sum()
    }

    /// The model's files, then the extras (`true` = saved in the models
    /// folder itself).
    pub(crate) fn all_files(&self) -> impl Iterator<Item = (&'static ModelFile, bool)> {
        self.files.iter().map(|f| (f, false)).chain(self.extras.iter().map(|f| (f, true)))
    }

    fn present(&self, models_dir: &Path, f: &ModelFile, extra: bool) -> bool {
        std::fs::metadata(self.file_path(models_dir, f, extra)).is_ok_and(|m| m.is_file() && m.len() == f.size)
    }

    /// The model and its extras are all there.
    pub fn complete(&self, models_dir: &Path) -> bool {
        self.all_files().all(|(f, x)| self.present(models_dir, f, x))
    }

    /// Bytes a download would still fetch.
    pub fn missing(&self, models_dir: &Path) -> u64 {
        self.all_files().filter(|(f, x)| !self.present(models_dir, f, *x)).map(|(f, _)| f.size).sum()
    }

    pub(crate) fn is_dir(&self) -> bool {
        matches!(self.engine, ModelEngine::Parakeet | ModelEngine::SherpaOnline)
    }

    /// An AI cleanup model (Settings → AI cleanup), not a speech model.
    pub fn is_llm(&self) -> bool {
        self.engine == ModelEngine::Llm
    }

    /// Where the model ends up: the file, or the sherpa folder.
    pub fn dest_path(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.dest)
    }

    pub(crate) fn file_path(&self, models_dir: &Path, f: &ModelFile, extra: bool) -> PathBuf {
        if extra {
            models_dir.join(f.name)
        } else if self.is_dir() {
            self.dest_path(models_dir).join(f.name)
        } else {
            self.dest_path(models_dir)
        }
    }

    /// Every model file present with its pinned size (cheap; the hash was
    /// checked when it was downloaded). Extras don't count: the model works
    /// without them ([`CatalogEntry::complete`]).
    pub fn installed(&self, models_dir: &Path) -> bool {
        self.files.iter().all(|f| self.present(models_dir, f, false))
    }

    /// Point the local engine at the downloaded model; a language model
    /// (Hebrew) becomes that language's route and leaves the default alone.
    pub fn apply_to(&self, prefs: &mut Prefs, models_dir: &Path) {
        if self.language.is_some() {
            prefs.hebrew_model_dir = None;
            prefs.hebrew_model = match self.engine {
                ModelEngine::SherpaOnline => HebrewModel::Zipformer,
                _ => HebrewModel::ParakeetHe,
            };
            return;
        }
        let p = Some(self.dest_path(models_dir).display().to_string());
        match self.engine {
            // The cleanup model: turn local cleanup on with this file.
            ModelEngine::Llm => {
                prefs.llm_enabled = true;
                prefs.llm_model_path = p;
            }
            // No default-model entry runs on the online engine.
            ModelEngine::SherpaOnline => {}
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

    /// Before deleting this cleanup model: when the prefs use it, turn local
    /// cleanup off and forget the path (the formatting rules keep working).
    /// True when the prefs changed.
    pub fn forget_in(&self, prefs: &mut Prefs, models_dir: &Path, appdata: Option<&str>) -> bool {
        if !self.is_llm() || prefs.resolved_llm_path(appdata) != self.dest_path(models_dir) {
            return false;
        }
        let was = (prefs.llm_enabled, prefs.llm_model_path.clone());
        prefs.llm_enabled = false;
        prefs.llm_model_path = None;
        was != (prefs.llm_enabled, prefs.llm_model_path.clone())
    }

    /// Delete the downloaded files (and `.part` leftovers). Only cleanup
    /// models can be deleted from the app; missing files are fine.
    pub fn delete(&self, models_dir: &Path) -> Result<(), String> {
        if !self.is_llm() {
            return Err("only AI cleanup models can be deleted here".into());
        }
        for f in self.files {
            let p = self.file_path(models_dir, f, false);
            for q in [part_path(&p, f), p] {
                match std::fs::remove_file(&q) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("could not delete {}: {e}", q.display())),
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileView {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Saved next to the model, optional for an import (the Hebrew models'
    /// language-ID model); absent for the model's own files.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub extra: bool,
}

/// `model_catalog` row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogView {
    pub id: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub engine: ModelEngine,
    pub recommended: bool,
    /// AI cleanup: recommended when cleanup runs on a GPU.
    pub recommended_gpu: bool,
    /// `"he"` for a Hebrew model, absent for a default model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<&'static str>,
    /// Bytes, all files together.
    pub size: u64,
    pub page: &'static str,
    /// Full path of the file / folder it is saved as.
    pub dest: String,
    pub installed: bool,
    /// Installed with its extras (the Hebrew models' language-ID model).
    pub complete: bool,
    /// Bytes a download would still fetch (0 when complete).
    pub missing: u64,
    /// This build can download it (feature `download`) and run its engine.
    pub available: bool,
    /// This build runs its engine, so it can be imported from a folder,
    /// zip or file (`model_import`), with or without feature `download`.
    pub importable: bool,
    /// A server is set (D41): downloads come from its library (Settings
    /// lists it), never from Hugging Face, so `available` is false here.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_server: bool,
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
            recommended_gpu: e.recommended_gpu,
            language: e.language,
            license: e.license,
            size: e.size(),
            page: e.page,
            dest: e.dest_path(models_dir).display().to_string(),
            installed: e.installed(models_dir),
            complete: e.complete(models_dir),
            missing: e.missing(models_dir),
            available: can_download && engine_built(e.engine),
            importable: engine_built(e.engine),
            from_server: false,
            files: e
                .all_files()
                .map(|(f, extra)| FileView {
                    name: f.name,
                    url: f.url,
                    size: f.size,
                    sha256: f.sha256,
                    extra,
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
    /// Copying from a local folder / zip / file (`model_import`).
    Importing,
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

/// Download `e` into `models_dir` from Hugging Face (see the module docs).
/// Emits progress through `emit`, but not the final `done` / `error` /
/// `cancelled`: the caller sends that once prefs are updated.
pub fn run(
    e: &CatalogEntry,
    models_dir: &Path,
    fetch: &dyn Fetch,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Stop> {
    run_from(e, models_dir, &HfSource(fetch), cancel, emit)
}

/// Where a catalogue model's files come from (D41): Hugging Face, or the
/// configured server's library (`server_library`). Never both: a set
/// server replaces Hugging Face.
pub trait FileSource {
    /// Bring `f` to `target`, checked against its pinned size and SHA-256,
    /// writing through `part`; `tick` gets this file's bytes so far. On
    /// any error nothing is left at `target`.
    fn fetch(
        &self,
        f: &ModelFile,
        target: &Path,
        part: &Path,
        cancel: &AtomicBool,
        tick: &mut dyn FnMut(u64),
    ) -> Result<(), Stop>;
}

/// Hugging Face through a [`Fetch`] (D31): stream into `part`, compare
/// the hash, rename.
pub struct HfSource<'a>(pub &'a dyn Fetch);

impl FileSource for HfSource<'_> {
    fn fetch(
        &self,
        f: &ModelFile,
        target: &Path,
        part: &Path,
        cancel: &AtomicBool,
        tick: &mut dyn FnMut(u64),
    ) -> Result<(), Stop> {
        let digest = match fetch_file(f, part, self.0, cancel, tick) {
            Ok(d) => d,
            Err(s) => {
                let _ = std::fs::remove_file(part);
                return Err(s);
            }
        };
        if digest != f.sha256 {
            let _ = std::fs::remove_file(part);
            return Err(Stop::Failed(format!(
                "{} did not match its fingerprint, so it was deleted",
                f.name
            )));
        }
        std::fs::rename(part, target).map_err(|er| {
            let _ = std::fs::remove_file(part);
            Stop::Failed(format!("could not save the file: {er}"))
        })
    }
}

/// [`run`] with any [`FileSource`].
pub fn run_from(
    e: &CatalogEntry,
    models_dir: &Path,
    src: &dyn FileSource,
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
    for (f, extra) in e.all_files() {
        let target = e.file_path(models_dir, f, extra);
        // Already there and intact (an earlier, cancelled try): keep it.
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == f.size) {
            report(done, State::Verifying);
            if intact(&target, f) {
                done += f.size;
                report(done, State::Downloading);
                continue;
            }
        }
        let part = part_path(&target, f);
        src.fetch(f, &target, &part, cancel, &mut |n| report(done + n, State::Downloading))?;
        report(done + f.size, State::Verifying);
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
    copy_verified(&mut src, f, part, "download of", cancel, tick)
}

/// Where `f` is written before it is checked: `<name>.part` next to `target`.
pub(crate) fn part_path(target: &Path, f: &ModelFile) -> PathBuf {
    target.with_file_name(format!("{}.part", f.name))
}

/// `target` holds exactly `f` (pinned size and SHA-256).
pub(crate) fn intact(target: &Path, f: &ModelFile) -> bool {
    std::fs::metadata(target).is_ok_and(|m| m.is_file() && m.len() == f.size)
        && sha256_file(target).is_ok_and(|h| h == f.sha256)
}

/// Copy `src` into `part` (created or truncated), hashing it on the way;
/// returns the hex SHA-256. Fails on more or fewer bytes than `f.size`
/// (`what` names the source in a read error: "download of", "reading").
/// The caller compares the hash and deletes `part` on any error. Shared by
/// downloads and imports (`model_import`).
pub(crate) fn copy_verified(
    src: &mut dyn Read,
    f: &ModelFile,
    part: &Path,
    what: &str,
    cancel: &AtomicBool,
    tick: &mut dyn FnMut(u64),
) -> Result<String, Stop> {
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
            .map_err(|e| Stop::Failed(format!("{what} {} stopped: {e}", f.name)))?;
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

/// Whether this build runs `e`'s engine (a model for it can be used).
pub fn engine_built(e: ModelEngine) -> bool {
    match e {
        ModelEngine::Whisper => cfg!(feature = "whisper"),
        ModelEngine::Parakeet | ModelEngine::SherpaOnline => cfg!(feature = "parakeet"),
        // llama.cpp is a plugin (flow_llm.dll) loaded by the local engine.
        ModelEngine::Llm => cfg!(feature = "local-engine"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn catalogue_is_pinned_to_hugging_face_commits() {
        assert_eq!(CATALOG.len(), 8);
        let mut ids = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(ids.insert(e.id), "duplicate id {}", e.id);
            assert!(e.page.starts_with("https://huggingface.co/"));
            crate::shell::check_external_url(e.page).unwrap();
            for (f, _) in e.all_files() {
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
        // The sherpa loaders need encoder, decoder, joiner and tokens.txt.
        for id in ["parakeet-tdt-0.6b-v2-int8", ZIPFORMER_HE_ID, PARAKEET_HE_ID] {
            let pk: Vec<_> = entry(id).unwrap().files.iter().map(|f| f.name).collect();
            for want in ["encoder", "decoder", "joiner", "tokens.txt"] {
                assert!(pk.iter().any(|n| n.starts_with(want)), "{id}: {want}");
            }
        }
        // Hebrew models: the folders the Hebrew route loads, CC-BY credit.
        assert_eq!(entry(ZIPFORMER_HE_ID).unwrap().dest, crate::core_adapter::ZIPFORMER_HE_DIR);
        assert_eq!(entry(PARAKEET_HE_ID).unwrap().dest, crate::core_adapter::PARAKEET_HE_DIR);
        for e in CATALOG {
            assert_eq!(e.language.is_some(), e.license.is_some_and(|l| l.starts_with("CC-BY-4.0")), "{}", e.id);
        }
        // Llama's licence asks for "Built with Llama" where it is offered.
        assert!(entry(LLAMA32_3B_ID).unwrap().license.unwrap().contains("Built with Llama"));
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
            extras: &[],
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
    fn extras_land_in_the_models_folder_and_are_shared() {
        let (mut e, mut mem) = entry_of(ModelEngine::SherpaOnline, vec![("encoder.int8.onnx", b"enc"), ("tokens.txt", b"tok")]);
        let lid: &[u8] = b"tiny lid model";
        let url: &'static str = Box::leak(format!("https://huggingface.co/w/resolve/{}/lid.bin", "b".repeat(40)).into_boxed_str());
        mem.files.insert(url, lid.to_vec());
        e.extras = Box::leak(vec![ModelFile { name: "lid.bin", url, size: lid.len() as u64, sha256: sha(lid) }].into_boxed_slice());
        let d = tmp("extras");
        assert_eq!(e.missing(&d), 3 + 3 + lid.len() as u64);
        run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(files_in(&d), vec!["lid.bin", "pk-dir/encoder.int8.onnx", "pk-dir/tokens.txt"]);
        assert!(e.installed(&d) && e.complete(&d) && e.missing(&d) == 0);
        // A model installed before extras existed: installed, not complete;
        // the download fetches only the extra.
        std::fs::remove_file(d.join("lid.bin")).unwrap();
        assert!(e.installed(&d) && !e.complete(&d));
        assert_eq!(e.missing(&d), lid.len() as u64);
        mem.opens.lock().unwrap().clear();
        run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(*mem.opens.lock().unwrap(), vec![url.to_string()]);
        assert!(e.complete(&d));
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
    fn hebrew_downloads_choose_the_hebrew_route_and_keep_the_default() {
        let d = Path::new("/m");
        let zip = entry(ZIPFORMER_HE_ID).unwrap();
        // Model + the 32 MB language-ID model (H6).
        assert_eq!(zip.size(), 72_316_960 + 32_152_673);
        assert_eq!(entry(PARAKEET_HE_ID).unwrap().size(), 672_825_290 + 32_152_673);
        assert_eq!(zip.extras[0].name, crate::core_adapter::LID_FILE);
        let mut prefs = Prefs {
            asr_engine: AsrEngineChoice::Parakeet,
            hebrew_model: HebrewModel::None,
            ..Prefs::default()
        };
        let before = prefs.clone();
        zip.apply_to(&mut prefs, d);
        assert_eq!(prefs.hebrew_model, HebrewModel::Zipformer);
        assert_eq!(prefs.asr_engine, before.asr_engine);
        assert_eq!(prefs.parakeet_model_dir, before.parakeet_model_dir);
        entry(PARAKEET_HE_ID).unwrap().apply_to(&mut prefs, d);
        assert_eq!(prefs.hebrew_model, HebrewModel::ParakeetHe);
        assert_eq!(prefs.asr_engine, AsrEngineChoice::Parakeet);
        // Downloaded into its own folder (4 files).
        assert_eq!(zip.dest_path(d), d.join("zipformer2-66m-streaming-he-int8"));
        assert!(zip.is_dir());
    }

    #[test]
    fn cleanup_models_are_pinned_ggufs() {
        let llm: Vec<_> = CATALOG.iter().filter(|e| e.is_llm()).collect();
        assert_eq!(
            llm.iter().map(|e| e.id).collect::<Vec<_>>(),
            ["qwen2.5-0.5b-instruct-q4_k_m", "qwen2.5-1.5b-instruct-q4_k_m", LLAMA32_3B_ID]
        );
        for e in &llm {
            // One GGUF saved as itself in the models folder, from Qwen's repo.
            assert_eq!(e.files.len(), 1, "{}", e.id);
            assert!(e.extras.is_empty() && e.language.is_none() && !e.is_dir());
            assert_eq!(e.files[0].name, e.dest);
            assert!(e.dest.ends_with(".gguf"));
            assert!(
                e.files[0].url.starts_with("https://huggingface.co/Qwen/")
                    || e.files[0].url.starts_with("https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/"),
                "{}",
                e.files[0].url
            );
            assert_eq!(e.dest_path(Path::new("/m")), Path::new("/m").join(e.dest));
        }
        // The 1.5B lands where the app looks without a configured path (D11);
        // the 0.5B keeps the old default name, so an existing copy still loads.
        assert_eq!(llm[1].dest, crate::core_adapter::DEFAULT_LLM_FILE);
        assert_eq!(llm[0].dest, crate::core_adapter::LEGACY_LLM_FILE);
        assert_eq!(llm[0].size(), 491_400_032);
        assert_eq!(llm[1].size(), 1_117_320_736);
        // docs/findings/2026-10-30-laptop-model-benchmark.md § Catalogue entries.
        assert_eq!(llm[2].size(), 2_019_377_696);
        assert_eq!(llm[2].files[0].sha256, "6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff");
        assert!(llm[2].files[0].url.contains("/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/"));
        // Exactly one pick each for a CPU-only PC and for a GPU.
        assert_eq!(llm.iter().filter(|e| e.recommended).count(), 1);
        assert_eq!(llm.iter().filter(|e| e.recommended_gpu).count(), 1);
        // Speech models never carry the GPU flag.
        assert!(CATALOG.iter().filter(|e| !e.is_llm()).all(|e| !e.recommended_gpu));
    }

    #[test]
    fn cleanup_recommendation_follows_the_gpu() {
        assert_eq!(recommended_llm(true).id, LLAMA32_3B_ID);
        assert_eq!(recommended_llm(false).id, "qwen2.5-1.5b-instruct-q4_k_m");
        // The CPU pick is also the default file the app loads without a
        // configured path (D11).
        assert_eq!(recommended_llm(false).dest, crate::core_adapter::DEFAULT_LLM_FILE);
        let v = catalog_view(&tmp("rec"), true, |_| true);
        let r = |id: &str| v.iter().find(|r| r.id == id).map(|r| (r.recommended, r.recommended_gpu)).unwrap();
        assert_eq!(r(LLAMA32_3B_ID), (false, true));
        assert_eq!(r("qwen2.5-1.5b-instruct-q4_k_m"), (true, false));
        assert_eq!(r("qwen2.5-0.5b-instruct-q4_k_m"), (false, false));
    }

    #[test]
    fn a_cleanup_download_turns_local_cleanup_on_and_keeps_speech() {
        let d = Path::new("/m");
        let e = entry("qwen2.5-1.5b-instruct-q4_k_m").unwrap();
        let mut prefs = Prefs {
            asr_engine: AsrEngineChoice::Parakeet,
            llm_enabled: false,
            ..Prefs::default()
        };
        let before = prefs.clone();
        e.apply_to(&mut prefs, d);
        assert!(prefs.llm_enabled);
        let want = d.join("qwen2.5-1.5b-instruct-q4_k_m.gguf");
        assert_eq!(prefs.llm_model_path.as_deref(), Some(want.display().to_string().as_str()));
        assert_eq!(prefs.resolved_llm_path(None), want);
        assert_eq!(
            (prefs.asr_engine, &prefs.model_path, &prefs.parakeet_model_dir, prefs.hebrew_model),
            (before.asr_engine, &before.model_path, &before.parakeet_model_dir, before.hebrew_model)
        );
        // The engine loads it: the cleanup spec changed.
        assert!(crate::core_adapter::needs_llm_reload(&before, &prefs));
        let spec = crate::core_adapter::LlmSpec::from_prefs(&prefs, None).unwrap();
        assert_eq!(spec.model_path, want);
        assert_eq!(spec.lib_path, before.llm_lib_path.map(PathBuf::from));
    }

    #[test]
    fn deleting_a_cleanup_model_forgets_it_and_removes_its_file() {
        let (e, mem) = entry_of(ModelEngine::Llm, vec![("m.gguf", b"gguf bytes")]);
        let d = tmp("llm");
        run(&e, &d, &mem, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(files_in(&d), vec!["m.gguf"]);
        std::fs::write(d.join("m.gguf.part"), b"left over").unwrap();
        let mut prefs = Prefs::default();
        e.apply_to(&mut prefs, &d);
        // Another cleanup model in use: nothing to forget.
        let mut other = Prefs {
            llm_enabled: true,
            llm_model_path: Some("/x/other.gguf".into()),
            ..Prefs::default()
        };
        assert!(!e.forget_in(&mut other, &d, None));
        assert!(other.llm_enabled);
        assert!(e.forget_in(&mut prefs, &d, None));
        assert!(!prefs.llm_enabled && prefs.llm_model_path.is_none());
        e.delete(&d).unwrap();
        assert!(files_in(&d).is_empty(), "{:?}", files_in(&d));
        assert!(!e.installed(&d));
        e.delete(&d).unwrap(); // already gone: fine
        // Speech models are never deleted from here.
        assert!(entry("whisper-base").unwrap().delete(&d).is_err());
        let _ = std::fs::remove_dir_all(&d);
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
        let pk = v.iter().find(|r| r.id == "parakeet-tdt-0.6b-v2-int8").unwrap();
        assert!(!pk.available && !pk.installed);
        assert_eq!(pk.size, 661_190_513);
        assert!(v.iter().filter(|r| r.engine == ModelEngine::Whisper).all(|r| r.available));
        assert!(catalog_view(&d, false, |_| true).iter().all(|r| !r.available));
        // No downloader (client-only build): every model can still be imported.
        assert!(catalog_view(&d, false, |_| true).iter().all(|r| r.importable));
        assert!(!pk.importable);
        let he = serde_json::to_value(v.iter().find(|r| r.id == ZIPFORMER_HE_ID).unwrap()).unwrap();
        assert_eq!(he["language"], "he");
        assert_eq!(he["engine"], "sherpa_online");
        assert!(he["license"].as_str().unwrap().contains("ivrit.ai"));
        assert_eq!(he["files"][4]["extra"], true);
        assert!(he["files"][0].get("extra").is_none());
        let q = serde_json::to_value(v.iter().find(|r| r.id == "qwen2.5-0.5b-instruct-q4_k_m").unwrap()).unwrap();
        assert_eq!(q["engine"], "llm");
        let en = serde_json::to_value(pk).unwrap();
        assert!(en.get("language").is_none() && en.get("license").is_none());
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
