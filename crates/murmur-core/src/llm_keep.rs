//! Keep the on-device LLM across a speech-model reload (ticket "Load 3").
//!
//! A RAM-tight shell (Android, D33 decision 4 / D37) frees the whole engine
//! and loads a new one when the dictation language routes to another
//! speech model. The cleanup LLM is the same in both, and loading it again
//! costs ~38 s on the phone (Qwen2.5-1.5B, mostly the static prompt
//! prefill). Before freeing the old engine the shell calls
//! [`crate::MurmurCore::keep_llm`]: that engine's own LLM is then held here,
//! and the next load whose config names the same model, thread count and
//! plugin takes it instead of loading it again.
//!
//! A kept LLM never outlives the next load that finishes: a load takes it
//! out first (dropping it at once when it does not match), uses it, or
//! puts it back when the speech model fails to load, so a retry still
//! finds it. [`release`] drops it (low-memory unload).

use std::sync::{Arc, Mutex};

use crate::cleanup::Cleaner;
use crate::config::ModelConfig;

/// What makes two loads' LLMs the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LlmKey {
    pub(crate) path: String,
    threads: usize,
    lib: Option<String>,
    draft: Option<usize>,
}

impl LlmKey {
    /// `None` when `cfg` loads no LLM.
    pub(crate) fn of(cfg: &ModelConfig) -> Option<Self> {
        let path = cfg.llm_path.as_deref().filter(|p| !p.is_empty())?;
        Some(Self {
            path: path.to_string(),
            threads: cfg.threads,
            lib: cfg.llm_lib.clone(),
            draft: cfg.llm_draft,
        })
    }
}

/// An LLM and the load it belongs to.
pub(crate) type KeptLlm = (LlmKey, Arc<dyn Cleaner>);

/// Holds at most one LLM between a freed engine and the next load.
pub(crate) struct LlmSlot(Mutex<Option<KeptLlm>>);

impl LlmSlot {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Hold `llm` for the next load (replacing a kept one, which is
    /// dropped outside the lock: freeing a model may take a moment).
    pub(crate) fn keep(&self, llm: KeptLlm) {
        let old = self.0.lock().unwrap().replace(llm);
        drop(old);
    }

    pub(crate) fn release(&self) {
        let old = self.0.lock().unwrap().take();
        drop(old);
    }

    /// Take the kept LLM out: it is returned when it is `key`'s, else
    /// dropped (so its memory is free before the load continues).
    pub(crate) fn take(&self, key: Option<&LlmKey>) -> Option<KeptLlm> {
        let kept = self.0.lock().unwrap().take()?;
        (Some(&kept.0) == key).then_some(kept)
    }
}

/// The process's slot (one engine at a time on a RAM-tight shell).
pub(crate) static SLOT: LlmSlot = LlmSlot::new();

/// Drop an LLM [`crate::MurmurCore::keep_llm`] holds (low-memory unload with
/// no load to follow).
pub fn release() {
    SLOT.release()
}

#[cfg(test)]
mod tests {
    use std::sync::Weak;
    use std::time::Instant;

    use super::*;
    use crate::cleanup::{CleanError, CleanPrompt};

    struct Fake;
    impl Cleaner for Fake {
        fn clean(&self, p: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
            Ok(p.transcript.clone())
        }
    }

    fn key(path: &str) -> LlmKey {
        LlmKey {
            path: path.into(),
            threads: 4,
            lib: None,
            draft: None,
        }
    }

    /// An engine's LLM, plus a weak handle to see whether it was freed.
    fn llm() -> (Arc<dyn Cleaner>, Weak<dyn Cleaner>) {
        let c: Arc<dyn Cleaner> = Arc::new(Fake);
        let w = Arc::downgrade(&c);
        (c, w)
    }

    #[test]
    fn a_kept_llm_survives_its_engine_and_is_reused_once() {
        let slot = LlmSlot::new();
        let (engine_llm, weak) = llm();
        slot.keep((key("qwen.gguf"), engine_llm));
        let (_, reused) = slot.take(Some(&key("qwen.gguf"))).expect("kept");
        assert!(Weak::ptr_eq(&Arc::downgrade(&reused), &weak));
        // Taken: a later load gets nothing.
        assert!(slot.take(Some(&key("qwen.gguf"))).is_none());
    }

    #[test]
    fn keeping_another_llm_frees_the_older_one() {
        let slot = LlmSlot::new();
        let (a, weak_a) = llm();
        let (b, weak_b) = llm();
        slot.keep((key("a.gguf"), a));
        slot.keep((key("b.gguf"), b));
        assert!(weak_a.upgrade().is_none());
        assert!(weak_b.upgrade().is_some());
    }

    #[test]
    fn another_model_or_no_llm_drops_the_kept_one() {
        for next in [Some(key("other.gguf")), None] {
            let slot = LlmSlot::new();
            let (c, weak) = llm();
            slot.keep((key("qwen.gguf"), c));
            assert!(slot.take(next.as_ref()).is_none());
            assert!(weak.upgrade().is_none(), "freed before the new load");
        }
        // Same file, other thread count: not the same load.
        let slot = LlmSlot::new();
        slot.keep((key("qwen.gguf"), llm().0));
        let mut k = key("qwen.gguf");
        k.threads = 2;
        assert!(slot.take(Some(&k)).is_none());
    }

    #[test]
    fn release_frees_a_kept_llm() {
        let slot = LlmSlot::new();
        let (c, weak) = llm();
        slot.keep((key("qwen.gguf"), c));
        assert!(weak.upgrade().is_some());
        slot.release();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn key_follows_the_model_config() {
        let cfg = ModelConfig {
            llm_path: Some("m.gguf".into()),
            threads: 3,
            ..Default::default()
        };
        assert_eq!(
            LlmKey::of(&cfg),
            Some(LlmKey {
                path: "m.gguf".into(),
                threads: 3,
                lib: None,
                draft: None
            })
        );
        for none in [None, Some(String::new())] {
            let cfg = ModelConfig {
                llm_path: none,
                ..Default::default()
            };
            assert_eq!(LlmKey::of(&cfg), None);
        }
    }
}
