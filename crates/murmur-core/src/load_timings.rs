//! How long a [`crate::MurmurCore`] load took, per model and per phase
//! (ticket "Load 3"), so a shell can log it and a phone load can be
//! measured: `docs/findings/2026-11-05-phone-model-load-time.md`.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// One step of one model's load, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Phase {
    pub name: &'static str,
    pub ms: u64,
}

/// What a [`ModelTiming`] loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Asr,
    Lid,
    Llm,
}

/// The engine that loaded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBackend {
    Mock,
    Whisper,
    Parakeet,
    SherpaOnline,
    Llama,
}

/// One model of a load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelTiming {
    pub kind: ModelKind,
    /// ASR only: the `languages` map key (D33) of a language model; absent
    /// for the default model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub backend: LoadBackend,
    /// Wall time of this model's load (0 when reused).
    pub ms: u64,
    /// The backend's own steps, when it reports them: sherpa models
    /// `files`, `model_type`, `vocab`, `beam_sessions`, `greedy_sessions`,
    /// `sessions`; the LLM `model` and `warm` (static prompt prefill).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<Phase>,
    /// LLM only: the previous engine's model was kept, not loaded again
    /// ([`crate::llm_keep`]).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reused: bool,
    /// Where the model runs, when the load chose a device: a whisper model
    /// with a GPU plugin configured (`ModelConfig::whisper_lib`, D84). Its
    /// `gpu_error` says why a GPU that was asked for is not used (the
    /// plugin did not load, or its GPU failed and the CPU took over).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs_on: Option<WhisperBackend>,
}

/// Where a whisper model runs: from the GPU plugin's JSON
/// (`murmur_whisper_info` or `murmur_whisper_backends`,
/// `crate::whisper_plugin`), or [`WhisperBackend::cpu`] for the built-in
/// engine.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WhisperBackend {
    /// "Vulkan", "CUDA", "CPU", …
    pub backend: String,
    /// The GPU's name, or "CPU".
    pub device: String,
    /// Set when a GPU was asked for or found but is not used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_error: Option<String>,
}

impl WhisperBackend {
    pub fn from_info(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let backend = v.get("backend")?.as_str()?.to_string();
        Some(Self {
            device: v
                .get("device")
                .and_then(|d| d.as_str())
                .unwrap_or(&backend)
                .to_string(),
            gpu_error: v
                .get("gpu_error")
                .and_then(|e| e.as_str())
                .map(str::to_string),
            backend,
        })
    }

    /// The CPU (the built-in engine, or the plugin without a GPU).
    pub fn cpu() -> Self {
        Self {
            backend: "CPU".into(),
            device: "CPU".into(),
            gpu_error: None,
        }
    }

    /// The model runs on a GPU (any backend but the CPU).
    pub fn is_gpu(&self) -> bool {
        !self.backend.eq_ignore_ascii_case("cpu")
    }
}

/// Every model of one load, in load order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LoadTimings {
    /// Wall time of the whole load.
    pub total_ms: u64,
    pub models: Vec<ModelTiming>,
}

impl LoadTimings {
    /// The LLM's entry, if this load has one.
    pub fn llm(&self) -> Option<&ModelTiming> {
        self.models.iter().find(|m| m.kind == ModelKind::Llm)
    }

    /// One JSON object (the shells' log line).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

pub(crate) fn ms_since(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_leaves_out_empty_fields() {
        let t = LoadTimings {
            total_ms: 2300,
            models: vec![
                ModelTiming {
                    kind: ModelKind::Asr,
                    language: Some("he".into()),
                    backend: LoadBackend::SherpaOnline,
                    ms: 2200,
                    phases: vec![Phase {
                        name: "sessions",
                        ms: 2190,
                    }],
                    reused: false,
                    runs_on: None,
                },
                ModelTiming {
                    kind: ModelKind::Llm,
                    language: None,
                    backend: LoadBackend::Llama,
                    ms: 0,
                    phases: Vec::new(),
                    reused: true,
                    runs_on: None,
                },
            ],
        };
        assert_eq!(
            t.to_json(),
            r#"{"total_ms":2300,"models":[{"kind":"asr","language":"he","backend":"sherpa_online","ms":2200,"phases":[{"name":"sessions","ms":2190}]},{"kind":"llm","backend":"llama","ms":0,"reused":true}]}"#
        );
        assert!(t.llm().unwrap().reused);
        assert_eq!(LoadTimings::default().llm(), None);
    }
}
