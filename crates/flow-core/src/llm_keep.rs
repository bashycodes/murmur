//! Keep the on-device LLM across a speech-model reload (ticket "Load 3").
//!
//! A RAM-tight shell (Android, D33 decision 4 / D37) frees the whole engine
//! and loads a new one when the dictation language routes to another
//! speech model. The cleanup LLM is the same in both, and loading it again
//! costs ~38 s on the phone (Qwen2.5-1.5B, mostly the static prompt
//! prefill). Before freeing the old engine the shell calls [`keep`]: the
//! LLM most recently loaded by [`crate::FlowCore`] is then held here, and
//! the next load whose config names the same model, thread count and
//! plugin takes it instead of loading it again.
//!
//! A kept LLM never outlives the next load: every load takes it out
//! (reusing it, or dropping it before loading anything when it does not
//! match), and [`release`] drops it (low-memory unload).

use std::sync::{Arc, Mutex, Weak};

use crate::cleanup::Cleaner;
use crate::config::ModelConfig;

/// What makes two loads' LLMs the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LlmKey {
    pub(crate) path: String,
    threads: usize,
    lib: Option<String>,
}

impl LlmKey {
    /// `None` when `cfg` loads no LLM.
    pub(crate) fn of(cfg: &ModelConfig) -> Option<Self> {
        let path = cfg.llm_path.as_deref().filter(|p| !p.is_empty())?;
        Some(Self {
            path: path.to_string(),
            threads: cfg.threads,
            lib: cfg.llm_lib.clone(),
        })
    }
}

pub(crate) struct LlmSlot {
    /// The most recently loaded LLM (not kept alive by this).
    last: Mutex<Option<(LlmKey, Weak<dyn Cleaner>)>>,
    /// The LLM [`keep`] holds for the next load.
    kept: Mutex<Option<(LlmKey, Arc<dyn Cleaner>)>>,
}

impl LlmSlot {
    pub(crate) const fn new() -> Self {
        Self {
            last: Mutex::new(None),
            kept: Mutex::new(None),
        }
    }

    /// A load just created `c` for `key`.
    pub(crate) fn loaded(&self, key: LlmKey, c: &Arc<dyn Cleaner>) {
        *self.last.lock().unwrap() = Some((key, Arc::downgrade(c)));
    }

    /// Hold the most recently loaded LLM, if it is still alive.
    pub(crate) fn keep(&self) -> bool {
        let last = self.last.lock().unwrap();
        let alive = last
            .as_ref()
            .and_then(|(k, w)| Some((k.clone(), w.upgrade()?)));
        let kept = alive.is_some();
        // Replacing an older kept LLM drops it here.
        *self.kept.lock().unwrap() = alive;
        kept
    }

    pub(crate) fn release(&self) {
        // Dropped outside the lock: freeing a model may take a moment.
        let old = self.kept.lock().unwrap().take();
        drop(old);
    }

    /// Take the kept LLM out: it is returned when it is `key`'s, else
    /// dropped (so its memory is free before the load continues).
    pub(crate) fn take(&self, key: Option<&LlmKey>) -> Option<Arc<dyn Cleaner>> {
        let kept = self.kept.lock().unwrap().take()?;
        (Some(&kept.0) == key).then_some(kept.1)
    }
}

pub(crate) static SLOT: LlmSlot = LlmSlot::new();

/// Keep the on-device LLM of the engine about to be freed, for the next
/// load to reuse (a language switch reloads only the speech model).
/// Returns whether there was a loaded LLM to keep. Call it while the old
/// engine is still alive.
pub fn keep() -> bool {
    SLOT.keep()
}

/// Drop an LLM [`keep`] holds (low-memory unload with no load to follow).
pub fn release() {
    SLOT.release()
}

#[cfg(test)]
mod tests {
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
        }
    }

    /// The old engine's LLM as a core holds it, plus a weak handle to see
    /// whether it was freed.
    fn loaded(slot: &LlmSlot, path: &str) -> (Arc<dyn Cleaner>, Weak<dyn Cleaner>) {
        let c: Arc<dyn Cleaner> = Arc::new(Fake);
        slot.loaded(key(path), &c);
        let w = Arc::downgrade(&c);
        (c, w)
    }

    #[test]
    fn a_kept_llm_survives_the_old_engine_and_is_reused_once() {
        let slot = LlmSlot::new();
        let (core_llm, weak) = loaded(&slot, "qwen.gguf");
        assert!(slot.keep());
        drop(core_llm); // the old engine is freed
        let reused = slot.take(Some(&key("qwen.gguf"))).expect("kept");
        assert!(Weak::ptr_eq(&Arc::downgrade(&reused), &weak));
        // Taken: a later load gets nothing.
        assert!(slot.take(Some(&key("qwen.gguf"))).is_none());
    }

    #[test]
    fn without_keep_the_llm_is_freed_with_its_engine() {
        let slot = LlmSlot::new();
        let (core_llm, weak) = loaded(&slot, "qwen.gguf");
        drop(core_llm);
        assert!(weak.upgrade().is_none());
        assert!(!slot.keep(), "nothing alive to keep");
        assert!(slot.take(Some(&key("qwen.gguf"))).is_none());
    }

    #[test]
    fn another_model_or_no_llm_drops_the_kept_one() {
        for next in [Some(key("other.gguf")), None] {
            let slot = LlmSlot::new();
            let (core_llm, weak) = loaded(&slot, "qwen.gguf");
            assert!(slot.keep());
            drop(core_llm);
            assert!(slot.take(next.as_ref()).is_none());
            assert!(weak.upgrade().is_none(), "freed before the new load");
        }
        // Same file, other thread count: not the same load.
        let slot = LlmSlot::new();
        let (_c, _w) = loaded(&slot, "qwen.gguf");
        slot.keep();
        let mut k = key("qwen.gguf");
        k.threads = 2;
        assert!(slot.take(Some(&k)).is_none());
    }

    #[test]
    fn release_frees_a_kept_llm() {
        let slot = LlmSlot::new();
        let (core_llm, weak) = loaded(&slot, "qwen.gguf");
        slot.keep();
        drop(core_llm);
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
                lib: None
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
