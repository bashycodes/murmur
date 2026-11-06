//! wispr-parity-clean (numbers copy of the lists/corrections runner): replay raw transcripts
//! through the real `FlowCore` commit path with one cleaner.
//!
//!   wispr-parity-clean --cases CASES.jsonl --session SESSION.json
//!       --mode rules|local|server|cloud [--llm MODEL.gguf --llm-lib SO]
//!       [--threads N] [--timeout-ms MS] [--override PROMPT.json]
//!       [--dump-prompt OUT.json] [--language CODE]
//!
//! `--override`: {"system": "...", "examples": [[user, assistant], ...]}
//! replaces the system prompt and the few-shot turns of every prompt the
//! core hands the cleaner (the per-dictation user message is untouched), so
//! prompt variants run on the real commit path without a rebuild.
//! `--dump-prompt`: write the first real prompt (system, examples, user).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::{AbortFlag, AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::FlowCore;

/// The transcript to "recognise" and the language to report (`--language`,
/// default en).
struct TextAsr(Arc<Mutex<String>>, String);
impl AsrEngine for TextAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: self.0.lock().unwrap().clone(),
            language: Some(self.1.clone()),
            ..Default::default()
        })
    }
}

#[derive(Clone)]
struct Override {
    system: String,
    examples: Vec<(String, String)>,
}

/// A cleaner that reports itself as another engine.
struct As {
    inner: Arc<dyn Cleaner>,
    engine: &'static str,
    timeout_ms: Option<u64>,
    over: Option<Override>,
    dump: Mutex<Option<String>>,
    last_prompt_chars: Mutex<usize>,
}

impl As {
    fn fix(&self, p: &CleanPrompt) -> CleanPrompt {
        let mut p = p.clone();
        if let Some(o) = &self.over {
            p.system = o.system.clone();
            p.examples = o.examples.clone();
        }
        *self.last_prompt_chars.lock().unwrap() = p.system.len()
            + p.examples.iter().map(|(u, a)| u.len() + a.len()).sum::<usize>()
            + p.user.len();
        if let Some(path) = self.dump.lock().unwrap().take() {
            let v = serde_json::json!({"system": p.system, "examples": p.examples, "user": p.user});
            std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
        p
    }
}

impl Cleaner for As {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.inner.clean(&self.fix(p), d)
    }
    fn clean_cancellable(&self, p: &CleanPrompt, d: Instant, c: &AbortFlag) -> Result<String, CleanError> {
        self.inner.clean_cancellable(&self.fix(p), d, c)
    }
    fn prefill(&self, p: &CleanPrompt) {
        self.inner.prefill(&self.fix(p))
    }
    fn cancel_prefill(&self) {
        self.inner.cancel_prefill()
    }
    fn info_json(&self) -> Option<String> {
        self.inner.info_json()
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.timeout_ms
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| argv.iter().position(|a| a == k).and_then(|i| argv.get(i + 1).cloned());
    let mode = get("--mode").expect("--mode rules|local|server|cloud");
    let cases = std::fs::read_to_string(get("--cases").expect("--cases")).expect("cases");
    let session: SessionConfig =
        serde_json::from_str(&std::fs::read_to_string(get("--session").expect("--session")).expect("session"))
            .expect("session json");
    let threads: usize = get("--threads").map_or(4, |t| t.parse().expect("--threads"));
    let timeout_ms: Option<u64> = get("--timeout-ms").map(|t| t.parse().expect("--timeout-ms"));
    let over = get("--override").map(|p| {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).expect("override")).expect("json");
        Override {
            system: v["system"].as_str().expect("system").to_string(),
            examples: v["examples"]
                .as_array()
                .expect("examples")
                .iter()
                .map(|e| (e[0].as_str().unwrap().to_string(), e[1].as_str().unwrap().to_string()))
                .collect(),
        }
    });
    let mut wrapper: Option<Arc<As>> = None;
    let cleaner: Option<Arc<dyn Cleaner>> = match mode.as_str() {
        "rules" => None,
        m @ ("local" | "server" | "cloud") => {
            let t = Instant::now();
            let llm = flow_core::llama::LlamaCleaner::load(
                &get("--llm").expect("--llm"),
                threads,
                get("--llm-lib").as_deref(),
            )
            .expect("llm load");
            eprintln!("llm load {} ms", t.elapsed().as_millis());
            let engine = match m {
                "local" => "local",
                "server" => "server",
                _ => "cloud",
            };
            let w = Arc::new(As {
                inner: Arc::new(llm),
                engine,
                timeout_ms,
                over,
                dump: Mutex::new(get("--dump-prompt")),
                last_prompt_chars: Mutex::new(0),
            });
            wrapper = Some(w.clone());
            Some(w)
        }
        m => panic!("unknown --mode {m}"),
    };
    let text = Arc::new(Mutex::new(String::new()));
    let core = FlowCore::from_engines(Arc::new(TextAsr(text.clone(), get("--language").unwrap_or_else(|| "en".into()))), cleaner);
    // One second of tone so VAD and the empty-audio checks pass.
    let tone: Vec<i16> = (0..16_000).map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16).collect();
    let mut warmed = false;
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let c: serde_json::Value = serde_json::from_str(line).expect("case json");
        let ctx: FieldContext = serde_json::from_value(serde_json::json!({
            "app_category": c.get("app_category").cloned().unwrap_or("other".into()),
            "context_known": true
        }))
        .expect("ctx");
        let run = |input: &str| {
            *text.lock().unwrap() = input.to_string();
            core.start_session(session.clone()).expect("session");
            core.set_context(ctx.clone());
            core.push_audio(&tone);
            let t = Instant::now();
            let r = core.commit();
            (r, t.elapsed().as_millis() as u64)
        };
        if !warmed {
            // The product prefills the static prompt prefix at model load;
            // here the prefix is prefilled with a generous deadline (a
            // 3k-token prefix takes ~30 s on this shared box), so the first
            // cases do not time out on it.
            if let Some(w) = &wrapper {
                let v = if mode == "cloud" {
                    flow_core::cleanup::PromptVersion::V5
                } else {
                    flow_core::cleanup::PromptVersion::V2
                };
                let p = flow_core::cleanup::build_prompt_versioned(
                    "This is a warm up sentence for the cache.",
                    session.cleanup_level,
                    session.styles.for_category(ctx.app_category),
                    &ctx,
                    &session.dictionary,
                    flow_core::cleanup::PromptScope::FULL,
                    v,
                );
                let t = Instant::now();
                let _ = w.clean(&p, Instant::now() + std::time::Duration::from_secs(300));
                eprintln!("warm {} ms", t.elapsed().as_millis());
            }
            let _ = run("This is a warm up sentence for the cache.");
            warmed = true;
        }
        let (r, commit_ms) = run(c["input"].as_str().unwrap_or_default());
        let chars = wrapper.as_ref().map(|w| *w.last_prompt_chars.lock().unwrap());
        println!(
            "{}",
            serde_json::json!({
                "id": c["id"], "text": r.final_text, "status": r.status,
                "fallback_reason": r.fallback_reason, "cloud_fallback": r.cloud_fallback,
                "fallback_detail": r.fallback_detail,
                "cleanup_route": r.cleanup_route, "llm_ms": r.timings.llm_ms, "commit_ms": commit_ms,
                "prompt_chars": chars
            })
        );
    }
}
