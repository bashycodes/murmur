//! formatting-eval: run the eval cases through the real flow-core commit
//! path (`FlowCore::from_engines` with a text "ASR", like `flow-cli clean`),
//! optionally with the llama.cpp cleaner, optionally with the system prompt
//! and few-shot turns swapped for a candidate prompt file. Product code is
//! used unmodified; only the prompt the cleaner receives is replaced.
//!
//! usage: formatting-eval --cases cases.jsonl --session session.json
//!          [--llm model.gguf --llm-lib libflow_llm.so [--threads 4]
//!           [--prompt prompt.json]] > out.jsonl
//!
//! prompt.json: {"system": "...", "examples": [[app, category, transcript, output], ...]}
//! The example user turns are rendered in the product's user-message format.
//!
//! --tuned-guard: instead of `FlowCore::commit`, replay `Engine::finish`
//! step by step (spoken_cleanup -> layout -> close_sentence -> LLM ->
//! postprocess::apply) with one change: the guard sees input and output
//! with formatting-command words (quote/unquote, paren, brackets, dash)
//! removed, as a proposed `guard::SPOKEN` extension would. Lets the eval
//! show how much the product guard costs without editing product code.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::config::{AppCategory, FieldContext, SessionConfig};
use flow_core::FlowCore;
use serde::Deserialize;

struct TextAsr(Arc<Mutex<String>>);
impl AsrEngine for TextAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: self.0.lock().unwrap().clone(),
            ..Default::default()
        })
    }
}

#[derive(Deserialize)]
struct PromptFile {
    system: String,
    examples: Vec<(String, String, String, String)>,
}

/// Replaces system + example turns; keeps the product's per-dictation user
/// message (APP / CATEGORY / STYLE / context / TRANSCRIPT).
struct Swapped {
    inner: Arc<dyn Cleaner>,
    system: String,
    examples: Vec<(String, String)>,
}
impl Swapped {
    fn swap(&self, p: &CleanPrompt) -> CleanPrompt {
        CleanPrompt {
            system: self.system.clone(),
            examples: self.examples.clone(),
            user: p.user.clone(),
            transcript: p.transcript.clone(),
        }
    }
}
impl Cleaner for Swapped {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.inner.clean(&self.swap(p), d)
    }
    fn clean_cancellable(
        &self,
        p: &CleanPrompt,
        d: Instant,
        c: &flow_core::asr::AbortFlag,
    ) -> Result<String, CleanError> {
        self.inner.clean_cancellable(&self.swap(p), d, c)
    }
    fn prefill(&self, p: &CleanPrompt) {
        self.inner.prefill(&self.swap(p))
    }
    fn cancel_prefill(&self) {
        self.inner.cancel_prefill()
    }
}

#[derive(Deserialize)]
struct Case {
    id: String,
    app_id: String,
    app_category: AppCategory,
    input: String,
}

/// `Engine::finish` (engine.rs) minus hallucination guard, snippets and
/// timings. The guard gets the texts as the engine passes them: it strips
/// command words itself and first checks that no literal command word
/// became a mark (`guard::literal_command_marks`), which a pre-stripped
/// input would hide.
fn replica(
    cl: Option<&Arc<dyn Cleaner>>,
    cfg: &SessionConfig,
    ctx: &FieldContext,
    raw: &str,
) -> (String, Option<String>, u64, Option<String>) {
    use flow_core::cleanup::*;
    use flow_core::config::CleanupLevel;
    use flow_core::postprocess;
    let mut text = raw.to_string();
    if cfg.spoken_cleanup && cfg.cleanup_level != CleanupLevel::None {
        text = postprocess::spoken::spoken_cleanup(&text);
        text = postprocess::spoken::layout(&text, ctx.app_category);
        if text.trim().is_empty() {
            return (String::new(), Some("only_filler".into()), 0, None);
        }
    }
    if cfg.cleanup_level != CleanupLevel::None {
        text = postprocess::terminal::close_sentence(&text);
    }
    let mut fb = None;
    let mut llm_ms = 0;
    let mut raw_llm = None;
    if let (true, Some(cl)) = (should_run(cfg, ctx.app_category), cl) {
        let style = cfg.styles.for_category(ctx.app_category);
        let prompt = build_prompt(&text, cfg.cleanup_level, style, ctx, &cfg.dictionary);
        let t = Instant::now();
        let timeout = std::time::Duration::from_millis(cfg.llm_timeout_ms);
        let deadline = t + timeout;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = flow_core::asr::AbortFlag(Some(cancel.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        let c2 = cl.clone();
        std::thread::spawn(move || {
            let _ = tx.send(c2.clean_cancellable(&prompt, deadline, &flag));
        });
        match rx.recv_timeout(timeout) {
            Ok(Ok(s)) => {
                raw_llm = Some(s.clone());
                let out = lists_to_plain(&unwrap_output(&s));
                if out.is_empty() {
                    fb = Some("llm_empty".to_string());
                } else {
                    match flow_core::guard::check(&text, &out) {
                        flow_core::guard::Verdict::Ok => text = out,
                        flow_core::guard::Verdict::Unsupported(_) => fb = Some("llm_unsupported".into()),
                        flow_core::guard::Verdict::Dropped(_) => fb = Some("llm_dropped".into()),
                    }
                }
            }
            Ok(Err(e)) => fb = Some(format!("llm_error: {e}")),
            Err(_) => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                fb = Some("llm_timeout".into());
                // Let the abandoned decode stop before the next case.
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        llm_ms = t.elapsed().as_millis() as u64;
    }
    let post = postprocess::apply(&text, raw, cfg, ctx);
    (post.text, fb, llm_ms, raw_llm)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| {
        argv.iter()
            .position(|a| a == k)
            .and_then(|i| argv.get(i + 1).cloned())
    };
    let cases = std::fs::read_to_string(get("--cases").expect("--cases")).unwrap();
    let session: SessionConfig =
        serde_json::from_str(&std::fs::read_to_string(get("--session").expect("--session")).unwrap())
            .unwrap();
    let threads: usize = get("--threads").map(|t| t.parse().unwrap()).unwrap_or(4);
    let cleaner: Option<Arc<dyn Cleaner>> = get("--llm").map(|m| {
        let t = Instant::now();
        let base: Arc<dyn Cleaner> = Arc::new(
            flow_core::llama::LlamaCleaner::load(&m, threads, get("--llm-lib").as_deref())
                .expect("llm load"),
        );
        eprintln!("llm load {} ms", t.elapsed().as_millis());
        match get("--prompt") {
            None => base,
            Some(p) => {
                let pf: PromptFile =
                    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
                let examples = pf
                    .examples
                    .into_iter()
                    .map(|(app, cat, tr, out)| {
                        (format!("APP: {app} | CATEGORY: {cat}\nTRANSCRIPT: \u{ab}{tr}\u{bb}"), out)
                    })
                    .collect();
                Arc::new(Swapped {
                    inner: base,
                    system: pf.system,
                    examples,
                })
            }
        }
    });
    let text = Arc::new(Mutex::new(String::new()));
    let cleaner_ref = cleaner.clone();
    let core = FlowCore::from_engines(Arc::new(TextAsr(text.clone())), cleaner);
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    let tuned = argv.iter().any(|a| a == "--tuned-guard");
    // Warm-up: the product prefills the static prompt prefix while the user
    // speaks (Cleaner::prefill at session start / context), so no real
    // dictation pays the cold prefix. One untimed clean does the same here.
    if let Some(cl) = &cleaner_ref {
        let t = Instant::now();
        let p = flow_core::cleanup::build_prompt(
            "Warm up the cache.",
            session.cleanup_level,
            session.styles.for_category(AppCategory::Other),
            &FieldContext::default(),
            &session.dictionary,
        );
        let _ = cl.clean(&p, Instant::now() + std::time::Duration::from_secs(60));
        eprintln!("warm-up {} ms", t.elapsed().as_millis());
    }
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let c: Case = serde_json::from_str(line).unwrap();
        if tuned {
            let ctx = FieldContext {
                app_id: c.app_id.clone(),
                app_category: c.app_category,
                ..Default::default()
            };
            if let Some(cl) = &cleaner_ref {
                let style = session.styles.for_category(c.app_category);
                cl.prefill(&flow_core::cleanup::build_prompt(
                    "", session.cleanup_level, style, &ctx, &session.dictionary,
                ));
            }
            let t = Instant::now();
            let (out, fb, llm_ms, raw_llm) = replica(cleaner_ref.as_ref(), &session, &ctx, &c.input);
            println!(
                "{}",
                serde_json::json!({"id": c.id, "output": out, "fallback": fb, "llm_raw": raw_llm,
                    "llm_ms": llm_ms, "commit_ms": t.elapsed().as_millis() as u64})
            );
            continue;
        }
        *text.lock().unwrap() = c.input.clone();
        core.start_session(session.clone()).unwrap();
        core.set_context(FieldContext {
            app_id: c.app_id.clone(),
            app_category: c.app_category,
            ..Default::default()
        });
        core.push_audio(&tone);
        let t = Instant::now();
        let r = core.commit();
        let wall = t.elapsed().as_millis() as u64;
        println!(
            "{}",
            serde_json::json!({
                "id": c.id,
                "output": r.final_text,
                "status": format!("{:?}", r.status),
                "fallback": r.fallback_reason,
                "empty_reason": r.empty_reason,
                "llm_ms": r.timings.llm_ms,
                "commit_ms": wall,
            })
        );
    }
}
