//! How long a [`crate::FlowCore`] load took, per model and per phase
//! (ticket "Load 3"), so a shell can log it and a phone load can be
//! measured: `docs/findings/2026-11-05-phone-model-load-time.md`.

use std::time::Instant;

use serde::Serialize;

/// One step of one model's load, in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Phase {
    pub name: String,
    pub ms: u64,
}

/// One model of a load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelTiming {
    /// `asr`, `lid` or `llm`.
    pub kind: &'static str,
    /// ASR only: the `languages` map key (D33) of a language model; absent
    /// for the default model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `whisper`, `parakeet`, `sherpa_online`, `mock`, `llama`.
    pub backend: &'static str,
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
        self.models.iter().find(|m| m.kind == "llm")
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
                    kind: "asr",
                    language: Some("he".into()),
                    backend: "sherpa_online",
                    ms: 2200,
                    phases: vec![Phase {
                        name: "sessions".into(),
                        ms: 2190,
                    }],
                    reused: false,
                },
                ModelTiming {
                    kind: "llm",
                    language: None,
                    backend: "llama",
                    ms: 0,
                    phases: Vec::new(),
                    reused: true,
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
