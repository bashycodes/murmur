//! Model library (D40): the server lists the models its owner tagged and
//! serves their files over the session port, so clients on an air-gapped
//! network can download models from their own server.
//!
//! * `GET /v1/models` → [`ModelList`] (bearer auth, like `/v1/health`).
//! * `GET /v1/models/{id}/files/{name}` → the file's bytes; HTTP `Range`
//!   (one range) → `206` + `Content-Range`. [`file_path`] builds the path.
//!
//! Ids and file names only ever contain [`is_safe_name`] characters (plus
//! `/` between the parts of a file in a subfolder), so a path needs no
//! percent-encoding. Unknown fields are ignored, as everywhere in v1.

use serde::{Deserialize, Serialize};

pub const PATH_MODELS: &str = "/v1/models";

/// What a model is for. The tag file's `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// Speech to text (the default engine for every language).
    Stt,
    /// Speech to text for the Hebrew route (D33).
    Hebrew,
    /// A cleanup LLM (GGUF).
    Llm,
    /// Language identification (D34).
    Lid,
    /// Voice activity detection.
    Vad,
}

impl ModelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stt => "stt",
            Self::Hebrew => "hebrew",
            Self::Llm => "llm",
            Self::Lid => "lid",
            Self::Vad => "vad",
        }
    }
}

/// The engines the apps load, as a tag's `engine` names them: whisper.cpp
/// ggml, sherpa-onnx NeMo transducer, sherpa-onnx streaming transducer,
/// llama.cpp GGUF. A tag may name another engine; no app can use it yet.
pub const ENGINES: &[&str] = &["whisper", "parakeet", "sherpa_online", "gguf"];

/// [`ENGINES`] for a message: `"whisper", "parakeet", "sherpa_online" or "gguf"`.
pub fn engines_hint() -> String {
    let q: Vec<String> = ENGINES.iter().map(|e| format!("\"{e}\"")).collect();
    match q.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => q.concat(),
    }
}

/// A model of the apps' download catalogue: the ids a tag's `catalogue_id`
/// may name. The pins live with each app's catalogue
/// (`apps/murmur-windows/src/model_download.rs` `CATALOG`, Android
/// `ServerCatalog`); tests keep both in step with this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogueModel {
    pub id: &'static str,
    pub kind: ModelKind,
    /// One of [`ENGINES`].
    pub engine: &'static str,
}

const fn cat(id: &'static str, kind: ModelKind, engine: &'static str) -> CatalogueModel {
    CatalogueModel { id, kind, engine }
}

pub const CATALOGUE: &[CatalogueModel] = &[
    cat("parakeet-tdt-0.6b-v2-int8", ModelKind::Stt, "parakeet"),
    cat("parakeet-unified-en-0.6b-int8", ModelKind::Stt, "parakeet"),
    cat("whisper-base", ModelKind::Stt, "whisper"),
    cat("whisper-small-q5_1", ModelKind::Stt, "whisper"),
    cat("whisper-base-en-q5_1", ModelKind::Stt, "whisper"),
    cat("whisper-base-q5_1", ModelKind::Stt, "whisper"),
    cat("whisper-tiny-en-q5_1", ModelKind::Stt, "whisper"),
    cat("whisper-small-en-q5_1", ModelKind::Stt, "whisper"),
    cat("whisper-large-v3-turbo-q5_0", ModelKind::Stt, "whisper"),
    cat("zipformer-he-int8", ModelKind::Hebrew, "sherpa_online"),
    cat(
        "parakeet-tdt-0.6b-v3-he-int8",
        ModelKind::Hebrew,
        "parakeet",
    ),
    cat(
        "ivrit-whisper-large-v3-turbo-q5_0",
        ModelKind::Hebrew,
        "whisper",
    ),
    cat("qwen2.5-0.5b-instruct-q4_k_m", ModelKind::Llm, "gguf"),
    cat("qwen2.5-1.5b-instruct-q4_k_m", ModelKind::Llm, "gguf"),
    cat("llama-3.2-3b-instruct-q4_k_m", ModelKind::Llm, "gguf"),
    cat("gemma-3-12b-it-q4_k_m", ModelKind::Llm, "gguf"),
];

/// Catalogue ids only a server runs, so no app catalogue pins them: Gemma 3
/// 12B, the cleanup model for a server with a GPU of more than 10 GB (D31).
/// Its pin is in `docs/colleague-install.md`. Whoever runs the server
/// downloads it under the Gemma Terms of Use, and Murmur never hands it on
/// (D80).
pub const SERVER_ONLY: &[&str] = &["gemma-3-12b-it-q4_k_m"];

/// The catalogue model `id` names, if any.
pub fn catalogue(id: &str) -> Option<&'static CatalogueModel> {
    CATALOGUE.iter().find(|c| c.id == id)
}

/// One file of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFile {
    /// Path inside the model folder, `/`-separated (`tokens.txt`).
    pub name: String,
    pub size: u64,
    /// SHA-256 of the file, lowercase hex. Also the download's `ETag`.
    pub sha256: String,
}

/// One tagged model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryModel {
    /// The model's folder name in the library; stable while the folder is.
    pub id: String,
    pub title: String,
    pub kind: ModelKind,
    /// BCP-47-ish language (`en`, `he`) or `None` for any / many.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The app catalogue id this model is a copy of (`whisper-base`).
    /// Clients verify such a model against their own pinned hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogue_id: Option<String>,
    /// How to load it (`whisper`, `parakeet`, `sherpa_online`, `gguf`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    pub files: Vec<LibraryFile>,
}

impl LibraryModel {
    /// Total bytes of the model's files.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// `GET /v1/models` response body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<LibraryModel>,
}

/// The download path of one file of a model.
pub fn file_path(id: &str, name: &str) -> String {
    format!("{PATH_MODELS}/{id}/files/{name}")
}

/// A model id or one part of a file name: 1–128 of `A-Z a-z 0-9 . _ -`,
/// not starting with `.` (no hidden files, no `.` / `..`).
pub fn is_safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_round_trips_and_omits_empty_options() {
        let l = ModelList {
            models: vec![LibraryModel {
                id: "whisper-base".into(),
                title: "Whisper base".into(),
                kind: ModelKind::Stt,
                language: None,
                catalogue_id: Some("whisper-base".into()),
                engine: Some("whisper".into()),
                files: vec![LibraryFile {
                    name: "ggml-base.bin".into(),
                    size: 3,
                    sha256: "ab".repeat(32),
                }],
            }],
        };
        let j = serde_json::to_string(&l).unwrap();
        assert!(j.contains(r#""kind":"stt""#), "{j}");
        assert!(!j.contains("language"), "{j}");
        assert_eq!(serde_json::from_str::<ModelList>(&j).unwrap(), l);
        assert_eq!(l.models[0].size(), 3);
    }

    #[test]
    fn catalogue_ids_are_unique_and_engines_known() {
        let mut ids = std::collections::HashSet::new();
        for c in CATALOGUE {
            assert!(ids.insert(c.id), "duplicate {}", c.id);
            assert!(is_safe_name(c.id), "{}", c.id);
            assert!(ENGINES.contains(&c.engine), "{}: {}", c.id, c.engine);
        }
        for id in SERVER_ONLY {
            assert!(catalogue(id).is_some(), "{id}");
        }
        assert_eq!(catalogue("whisper-base").unwrap().engine, "whisper");
        assert!(catalogue("nope").is_none());
        assert_eq!(
            engines_hint(),
            r#""whisper", "parakeet", "sherpa_online" or "gguf""#
        );
    }

    #[test]
    fn safe_names() {
        for ok in ["whisper-base", "a.b_c-1", "ggml-base.bin"] {
            assert!(is_safe_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            ".hidden",
            "a/b",
            "a b",
            "ü",
            &"x".repeat(129),
        ] {
            assert!(!is_safe_name(bad), "{bad:?}");
        }
        assert_eq!(file_path("m", "d/t.txt"), "/v1/models/m/files/d/t.txt");
    }
}
