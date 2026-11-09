//! On-device benchmark hook (debug harness `dev.murmur.debug.BenchActivity`,
//! `tools/probes/android_bench/`): run the AI-cleanup text path on a given
//! transcript with a bare core handle's own cleaner (the on-device LLM).
//!
//! Two modes, one JSON request each:
//! * `prod`: [`murmur_core::refinish`], i.e. the text half of a commit exactly
//!   as shipped (fallback rules, prompt v2, post-processing) on
//!   `input` as if ASR had returned it. Returns the product's `output` plus
//!   the LLM's raw answer and timing (recorded by a pass-through wrapper).
//! * `v4c`: the 2026-10-26 AI-cleanup design: prompt v4c on the RAW
//!   transcript, one `clean` call. Returns the raw answer for
//!   `ai-cleanup-bench --mode regard` on Linux (same shape as the PC bench's
//!   `llm_win.py` rows).
//!
//! No network, no state beyond the handle's own LLM; only text the caller
//! passes in is processed.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use murmur_core::asr::AbortFlag;
use murmur_core::cleanup::{
    self, CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope, PromptVersion,
};
use murmur_core::config::{FieldContext, SessionConfig};
use murmur_core::{CommitResult, MurmurCore};
use serde_json::{json, Value};

#[derive(serde::Deserialize)]
struct Request {
    /// `prod` (default) or `v4c`.
    #[serde(default)]
    mode: Option<String>,
    input: String,
    #[serde(default)]
    context: FieldContext,
    #[serde(default)]
    session: SessionConfig,
}

/// One LLM call as the wrapper saw it.
#[derive(Default, Clone)]
struct Call {
    raw: Option<String>,
    error: Option<String>,
    ms: u64,
}

/// Pass-through [`Cleaner`] that records the inner cleaner's answer.
struct Recorder {
    inner: Arc<dyn Cleaner>,
    calls: Mutex<Vec<Call>>,
}

impl Recorder {
    fn record(&self, t: Instant, r: &Result<String, CleanError>) {
        let (raw, error) = match r {
            Ok(s) => (Some(s.clone()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        self.calls.lock().unwrap().push(Call {
            raw,
            error,
            ms: t.elapsed().as_millis() as u64,
        });
    }
}

impl Cleaner for Recorder {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        let t = Instant::now();
        let r = self.inner.clean(p, d);
        self.record(t, &r);
        r
    }
    fn clean_cancellable(
        &self,
        p: &CleanPrompt,
        d: Instant,
        c: &AbortFlag,
    ) -> Result<String, CleanError> {
        let t = Instant::now();
        let r = self.inner.clean_cancellable(p, d, c);
        self.record(t, &r);
        r
    }
    fn clean_metered(
        &self,
        p: &CleanPrompt,
        d: Instant,
        c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        let t = Instant::now();
        let (r, u) = self.inner.clean_metered(p, d, c);
        self.record(t, &r);
        (r, u)
    }
    fn prefill(&self, p: &CleanPrompt) {
        self.inner.prefill(p)
    }
    fn cancel_prefill(&self) {
        self.inner.cancel_prefill()
    }
    fn info_json(&self) -> Option<String> {
        self.inner.info_json()
    }
    fn prompt_scope(&self) -> PromptScope {
        self.inner.prompt_scope()
    }
    fn engine(&self) -> &'static str {
        self.inner.engine()
    }
    fn model(&self) -> Option<String> {
        self.inner.model()
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.inner.timeout_ms()
    }
    fn budget_ms(&self, session_ms: u64) -> u64 {
        self.inner.budget_ms(session_ms)
    }
    fn available(&self) -> bool {
        self.inner.available()
    }
}

fn info(cl: &dyn Cleaner) -> Value {
    cl.info_json()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

/// A RAW_TRANSCRIPT result holding `text`, as a server without an LLM
/// returns it (the input of [`murmur_core::refinish`]).
fn transcript(text: &str) -> Result<CommitResult, String> {
    serde_json::from_value(json!({
        "status": "RAW_TRANSCRIPT", "asr_text": text, "final_text": "", "language": null,
        "timings": {"audio_ms": 0, "asr_ms": 0, "llm_ms": 0, "postprocess_ms": 0,
                    "total_ms": 0, "dropped_samples": 0},
        "press_enter": false, "fallback_reason": null, "truncated": false, "error": null,
    }))
    .map_err(|e| format!("bench transcript: {e}"))
}

/// Runs one request on `core`'s cleaner; `{"error": …}` when it cannot.
pub fn clean(core: &MurmurCore, req_json: &str) -> Value {
    match run(core, req_json) {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

fn run(core: &MurmurCore, req_json: &str) -> Result<Value, String> {
    let req: Request =
        serde_json::from_str(req_json).map_err(|e| format!("bad bench request: {e}"))?;
    let inner = core.cleaner().ok_or("no LLM loaded on this handle")?;
    let rec = Arc::new(Recorder {
        inner,
        calls: Mutex::new(Vec::new()),
    });
    let cfg = req.session;
    let mut ctx = req.context;
    let t = Instant::now();
    let mut out = match req.mode.as_deref().unwrap_or("prod") {
        "prod" => {
            let r = murmur_core::refinish(
                &cfg,
                &ctx,
                Some(rec.clone() as Arc<dyn Cleaner>),
                transcript(&req.input)?,
            );
            json!({"mode": "prod", "output": r.final_text, "status": r.status,
                   "fallback_reason": r.fallback_reason, "empty_reason": r.empty_reason,
                   "core_llm_ms": r.timings.llm_ms})
        }
        "v4c" => {
            ctx.app_category = murmur_core::postprocess::effective_category(cfg.platform, &ctx);
            let p = cleanup::build_prompt_versioned(
                &req.input,
                cfg.cleanup_level,
                cfg.styles.for_category(ctx.app_category),
                &ctx,
                &cfg.dictionary,
                rec.prompt_scope(),
                PromptVersion::V4c,
            )
            .with_language(cfg.forced_language());
            let _ = rec.clean(
                &p,
                Instant::now() + Duration::from_millis(cfg.llm_timeout_ms),
            );
            json!({"mode": "v4c"})
        }
        m => return Err(format!("unknown bench mode {m}")),
    };
    let wall = t.elapsed().as_millis() as u64;
    let calls = rec.calls.lock().unwrap().clone();
    let last = calls.last().cloned().unwrap_or_default();
    let o = out.as_object_mut().unwrap();
    o.insert("llm_raw".into(), json!(last.raw));
    o.insert("llm_error".into(), json!(last.error));
    o.insert("llm_ms".into(), json!(last.ms));
    o.insert("llm_calls".into(), json!(calls.len()));
    o.insert("wall_ms".into(), json!(wall));
    o.insert("llm_info".into(), info(rec.as_ref()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmur_core::asr::MockAsr;

    struct Echo;
    impl Cleaner for Echo {
        fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
            Ok(format!("Cleaned: {}", p.transcript))
        }
    }

    fn core(cl: Option<Arc<dyn Cleaner>>) -> MurmurCore {
        MurmurCore::from_engines(Arc::new(MockAsr::new("x")), cl)
    }

    fn req(mode: &str, input: &str) -> String {
        json!({"mode": mode, "input": input,
               "context": {"app_id": "com.example.mail", "app_category": "email"},
               "session": {"cleanup_level": "light", "llm_skip_clean": false, "llm_timeout_ms": 5000}})
        .to_string()
    }

    #[test]
    fn v4c_returns_the_raw_answer_for_the_raw_transcript() {
        let v = clean(&core(Some(Arc::new(Echo))), &req("v4c", "um hello there"));
        assert_eq!(v["mode"], "v4c");
        assert_eq!(v["llm_raw"], "Cleaned: um hello there");
        assert_eq!(v["llm_calls"], 1);
        assert!(v["llm_error"].is_null());
    }

    #[test]
    fn prod_runs_refinish_and_records_the_llm_call() {
        let v = clean(
            &core(Some(Arc::new(Echo))),
            &req("prod", "um hello there how are you"),
        );
        assert_eq!(v["mode"], "prod", "{v}");
        assert_eq!(v["llm_calls"], 1, "{v}");
        // The rules ran before the LLM (filler gone), as in the product.
        let raw = v["llm_raw"].as_str().unwrap();
        assert!(
            raw.starts_with("Cleaned: ") && !raw.contains(" um "),
            "{raw}"
        );
        assert!(v["output"].as_str().is_some());
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        assert!(clean(&core(None), &req("prod", "hi"))["error"]
            .as_str()
            .unwrap()
            .contains("no LLM"));
        assert!(clean(&core(Some(Arc::new(Echo))), "{")["error"]
            .as_str()
            .unwrap()
            .contains("bad bench request"));
        assert!(
            clean(&core(Some(Arc::new(Echo))), &req("zzz", "hi"))["error"]
                .as_str()
                .unwrap()
                .contains("unknown")
        );
    }
}
