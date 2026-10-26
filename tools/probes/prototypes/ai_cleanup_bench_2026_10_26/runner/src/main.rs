//! ai-cleanup-bench: the 2026-10-26 all-in-one AI cleanup benchmark.
//! Derived from formatting_eval_2026_10_24/runner (same text "ASR", same
//! prompt swap, same warm-up/prefill); flow-core is used unmodified.
//!
//! usage: ai-cleanup-bench --mode prod|ai|rules --cases cases.jsonl --session s.json
//!          [--llm model.gguf --llm-lib libflow_llm.so [--threads 4]]
//!          [--prompt prompt_v4.json] [--timeout-ms 8000] > out.jsonl
//!
//! * `prod`: `FlowCore::commit` exactly as shipped (fallback rules, then the
//!   product prompt v2 + guard + `llm_skip_clean`). The baseline row.
//! * `rules`: the fallback rules alone (`spoken_cleanup` + layout + closing
//!   period + post-processing), no LLM.
//! * `ai`: the owner's 2026-10-26 design. The LLM gets the RAW transcript
//!   (no fallback rules before it, no skip); its answer is checked by the
//!   product guard (`strict`) and by the loosened prototype
//!   (`loose_guard.rs`); on a reject the fallback rules produce the text.
//!   Post-processing (dictionary, cursor context) runs after, as in the
//!   product. One LLM call per case gives all three columns: `out_none`
//!   (no guard), `out_strict`, `out_loose`.
//!
//! * `regard --from previous_ai.jsonl`: re-judge a previous `ai` run's LLM
//!   answers with the current guards (no model needed).
//!
//! prompt.json: {"system": "...", "examples": [[app, category, transcript,
//! output] or [app, category, transcript, output, style], ...]}; examples are
//! rendered in the product's user-message format (STYLE line for casual).

mod loose_guard;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::cleanup::{
    build_prompt, lists_to_plain, unwrap_output, CleanError, CleanPrompt, Cleaner, PromptVersion,
};
use flow_core::config::{AppCategory, CleanupLevel, FieldContext, SessionConfig};
use flow_core::guard::Verdict;
use flow_core::postprocess;
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
    examples: Vec<Vec<String>>,
}

/// Same text as the product's casual `style_rule` (cleanup.rs), so the
/// example turns look like real user messages.
const CASUAL: &str = "casual: sentence case, drop optional commas, final period optional";

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
    fn clean_cancellable(&self, p: &CleanPrompt, d: Instant, c: &flow_core::asr::AbortFlag) -> Result<String, CleanError> {
        self.inner.clean_cancellable(&self.swap(p), d, c)
    }
    fn prefill(&self, p: &CleanPrompt) {
        self.inner.prefill(&self.swap(p))
    }
    fn cancel_prefill(&self) {
        self.inner.cancel_prefill()
    }
    fn prompt_scope(&self) -> flow_core::cleanup::PromptScope {
        self.inner.prompt_scope()
    }
}

#[derive(Deserialize)]
struct Case {
    id: String,
    app_id: String,
    app_category: AppCategory,
    input: String,
}

/// The fallback rules exactly as `Engine::finish` runs them before the LLM.
fn rules(raw: &str, cat: AppCategory) -> String {
    let t = postprocess::spoken::spoken_cleanup(raw);
    let t = postprocess::spoken::layout(&t, cat);
    if t.trim().is_empty() {
        return String::new();
    }
    postprocess::terminal::close_sentence(&t)
}

fn verdict(v: &Verdict) -> serde_json::Value {
    match v {
        Verdict::Ok => serde_json::Value::Null,
        Verdict::Unsupported(s) => serde_json::json!(format!("unsupported: {s}")),
        Verdict::Dropped(s) => serde_json::json!(format!("dropped: {s}")),
    }
}

/// Guard the answer both ways, fall back to the rules on a reject,
/// post-process.
fn judge(
    c: &Case,
    ctx: &FieldContext,
    session: &SessionConfig,
    llm_raw: Option<String>,
    err: Option<String>,
    llm_ms: u64,
) -> serde_json::Value {
    let fallback_text = rules(&c.input, c.app_category);
    let post = |s: &str| postprocess::apply(s, &c.input, session, ctx).text;
    let out = llm_raw.as_deref().map(|s| lists_to_plain(&unwrap_output(s)));
    // An empty answer is valid only for an all-filler input; the fallback
    // rules decide that.
    let (vs, vl) = match &out {
        Some(o) if o.is_empty() && fallback_text.is_empty() => (Verdict::Ok, Verdict::Ok),
        Some(o) if o.is_empty() => (Verdict::Dropped("empty".into()), Verdict::Dropped("empty".into())),
        Some(o) => (flow_core::guard::check(&c.input, o), loose_guard::check(&c.input, o)),
        None => (Verdict::Dropped("no answer".into()), Verdict::Dropped("no answer".into())),
    };
    let pick = |v: &Verdict| match (v, &out) {
        (Verdict::Ok, Some(o)) => post(o),
        _ => post(&fallback_text),
    };
    serde_json::json!({"id": c.id,
        "out_none": out.as_deref().map(post).unwrap_or_else(|| post(&fallback_text)),
        "out_strict": pick(&vs), "out_loose": pick(&vl),
        "verdict_strict": verdict(&vs), "verdict_loose": verdict(&vl),
        "error": err, "llm_raw": llm_raw, "llm_ms": llm_ms})
}

/// `--dump-prompt PATH`: write a flow-core prompt version in the
/// prompt.json shape, so `score.py --overlap` checks its few-shots.
fn dump_prompt(version: PromptVersion, path: &str) {
    let p = flow_core::cleanup::build_prompt_versioned(
        "",
        CleanupLevel::Light,
        flow_core::config::Style::Default,
        &FieldContext::default(),
        &flow_core::dictionary::Dictionary::default(),
        flow_core::cleanup::PromptScope::FULL,
        version,
    );
    let examples: Vec<serde_json::Value> = p
        .examples
        .iter()
        .map(|(u, a)| {
            let head = u.lines().next().unwrap();
            let (app, cat) = head.trim_start_matches("APP: ").split_once(" | CATEGORY: ").unwrap();
            let t = u.split_once("TRANSCRIPT: \u{ab}").unwrap().1.trim_end_matches('\u{bb}');
            let mut e = vec![app, cat, t, a.as_str()];
            if u.contains("\nSTYLE: casual") {
                e.push("casual");
            }
            serde_json::json!(e)
        })
        .collect();
    let v = serde_json::json!({
        "_note": format!("{version:?}: generated from flow-core by `ai-cleanup-bench --dump-prompt` (do not edit; the source is crates/flow-core/src/cleanup)."),
        "system": p.system, "examples": examples});
    std::fs::write(path, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap();
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| argv.iter().position(|a| a == k).and_then(|i| argv.get(i + 1).cloned());
    // `--prompt-version v4c`: flow-core's built-in prompt v4c (system,
    // few-shots and its own casual STYLE line) instead of a --prompt file.
    let version = match get("--prompt-version").as_deref() {
        None | Some("v2") => PromptVersion::V2,
        Some("v4c") => PromptVersion::V4c,
        Some(v) => panic!("unknown --prompt-version {v}"),
    };
    if let Some(p) = get("--dump-prompt") {
        return dump_prompt(version, &p);
    }
    let mode = get("--mode").expect("--mode prod|ai|rules");
    let cases = std::fs::read_to_string(get("--cases").expect("--cases")).unwrap();
    let session: SessionConfig =
        serde_json::from_str(&std::fs::read_to_string(get("--session").expect("--session")).unwrap()).unwrap();
    let threads: usize = get("--threads").map(|t| t.parse().unwrap()).unwrap_or(4);
    let timeout = Duration::from_millis(get("--timeout-ms").map(|t| t.parse().unwrap()).unwrap_or(8000));
    // Phase 1d: `--openrouter MODEL` uses the product's OpenRouterCleaner
    // (D28 privacy defaults zdr:true + data_collection deny, sort latency).
    // The key comes only from the env (OPENROUTER_KEY) and is never printed.
    let key = flow_router::SecretString::new(std::env::var("OPENROUTER_KEY").unwrap_or_default());
    let max_spend: f64 = get("--max-spend").map(|t| t.parse().unwrap()).unwrap_or(0.0);
    let or_cleaner: Option<Arc<flow_cloud::OpenRouterCleaner>> = get("--openrouter").map(|m| {
        assert!(!key.is_empty(), "--openrouter needs OPENROUTER_KEY in the env");
        let settings = flow_cloud::settings::CleanupSettings {
            model: m,
            timeout_ms: timeout.as_millis() as u64,
            ..Default::default()
        };
        Arc::new(flow_cloud::OpenRouterCleaner::new(settings, key.clone()))
    });
    let key_usage = || flow_cloud::keyinfo::fetch(&key).ok().and_then(|k| k.usage);
    let usage_start = or_cleaner.as_ref().and_then(|_| key_usage());
    if let Some(u) = usage_start {
        eprintln!("openrouter key usage at start: ${u:.4}");
    }
    let base_cleaner: Option<Arc<dyn Cleaner>> = match &or_cleaner {
        Some(o) => Some(o.clone() as Arc<dyn Cleaner>),
        None => get("--llm").map(|m| {
            let t = Instant::now();
            let b: Arc<dyn Cleaner> = Arc::new(
                flow_core::llama::LlamaCleaner::load(&m, threads, get("--llm-lib").as_deref()).expect("llm load"),
            );
            eprintln!("llm load {} ms", t.elapsed().as_millis());
            b
        }),
    };
    let cleaner: Option<Arc<dyn Cleaner>> = base_cleaner.map(|base| {
        match get("--prompt") {
            None => base,
            Some(p) => {
                let pf: PromptFile = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
                let examples = pf
                    .examples
                    .into_iter()
                    .map(|e| {
                        let style = match e.get(4).map(String::as_str) {
                            Some("casual") => format!("STYLE: {CASUAL}\n"),
                            _ => String::new(),
                        };
                        (format!("APP: {} | CATEGORY: {}\n{style}TRANSCRIPT: \u{ab}{}\u{bb}", e[0], e[1], e[2]), e[3].clone())
                    })
                    .collect();
                Arc::new(Swapped { inner: base, system: pf.system, examples })
            }
        }
    });
    // Warm-up: the product prefills the static prefix at model load and
    // while the user speaks; one untimed clean does the same here.
    // (Not for OpenRouter: there is no local cache to warm, and the
    // per-case `prefill` warms the connection as in the product.)
    if let (Some(cl), None) = (&cleaner, &or_cleaner) {
        let t = Instant::now();
        let p = build_prompt(
            "Warm up the cache.",
            session.cleanup_level,
            session.styles.for_category(AppCategory::Other),
            &FieldContext::default(),
            &session.dictionary,
        );
        let _ = cl.clean(&p, Instant::now() + Duration::from_secs(120));
        eprintln!("warm-up {} ms", t.elapsed().as_millis());
    }
    let prev: std::collections::HashMap<String, serde_json::Value> = get("--from")
        .map(|p| {
            std::fs::read_to_string(p)
                .unwrap()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    let v: serde_json::Value = serde_json::from_str(l).unwrap();
                    (v["id"].as_str().unwrap().to_string(), v)
                })
                .collect()
        })
        .unwrap_or_default();
    let text = Arc::new(Mutex::new(String::new()));
    let core = FlowCore::from_engines(Arc::new(TextAsr(text.clone())), cleaner.clone());
    let mut spent = 0.0f64;
    let tone: Vec<i16> = (0..16_000).map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16).collect();
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let c: Case = serde_json::from_str(line).unwrap();
        let ctx = FieldContext { app_id: c.app_id.clone(), app_category: c.app_category, ..Default::default() };
        match mode.as_str() {
            "prod" => {
                *text.lock().unwrap() = c.input.clone();
                core.start_session(session.clone()).unwrap();
                core.set_context(ctx);
                core.push_audio(&tone);
                let t = Instant::now();
                let r = core.commit();
                println!(
                    "{}",
                    serde_json::json!({"id": c.id, "output": r.final_text, "fallback": r.fallback_reason,
                        "empty_reason": r.empty_reason, "llm_ms": r.timings.llm_ms,
                        "commit_ms": t.elapsed().as_millis() as u64})
                );
            }
            "rules" => {
                let t = Instant::now();
                let out = postprocess::apply(&rules(&c.input, c.app_category), &c.input, &session, &ctx).text;
                println!("{}", serde_json::json!({"id": c.id, "output": out, "commit_ms": t.elapsed().as_millis() as u64}));
            }
            "ai" => {
                let cl = cleaner.as_ref().expect("--mode ai needs --llm");
                let style = session.styles.for_category(c.app_category);
                cl.prefill(&flow_core::cleanup::build_prompt_versioned(
                    "",
                    session.cleanup_level,
                    style,
                    &ctx,
                    &session.dictionary,
                    flow_core::cleanup::PromptScope::FULL,
                    version,
                ));
                // A cloud backend narrows the prompt (D28 minimal scope:
                // no app id), as the engine does.
                let prompt = flow_core::cleanup::build_prompt_versioned(
                    &c.input,
                    CleanupLevel::Light,
                    style,
                    &ctx,
                    &session.dictionary,
                    cl.prompt_scope(),
                    version,
                );
                let t = Instant::now();
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let flag = flow_core::asr::AbortFlag(Some(cancel.clone()));
                let (tx, rx) = std::sync::mpsc::channel();
                let c2 = cl.clone();
                let deadline = t + timeout;
                std::thread::spawn(move || {
                    let _ = tx.send(c2.clean_cancellable(&prompt, deadline, &flag));
                });
                let res = rx.recv_timeout(timeout);
                let llm_ms = t.elapsed().as_millis() as u64;
                let (llm_raw, err) = match res {
                    Ok(Ok(s)) => (Some(s), None),
                    Ok(Err(e)) => (None, Some(format!("llm_error: {e}"))),
                    Err(_) => {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        // Let the abandoned call stop (and record its cost).
                        let _ = rx.recv_timeout(Duration::from_secs(3));
                        (None, Some("llm_timeout".to_string()))
                    }
                };
                let mut row = judge(&c, &ctx, &session, llm_raw, err, llm_ms);
                if let Some(o) = &or_cleaner {
                    // Model, usage.cost, ttft, latency, finish reason; no text, no key.
                    let call = o.last_call();
                    let cost = call.as_ref().and_then(|k| k.cost_usd).unwrap_or(0.0);
                    spent += cost;
                    row["cloud"] = serde_json::to_value(&call).unwrap();
                    row["spent_usd"] = serde_json::json!(spent);
                    if max_spend > 0.0 && spent >= max_spend {
                        println!("{row}");
                        eprintln!("spend cap reached: ${spent:.4} >= ${max_spend:.4}; stopping");
                        break;
                    }
                }
                println!("{row}");
            }
            "regard" => {
                // Re-judge a previous `ai` run's answers with the current
                // guards (a guard change needs no new LLM run).
                let o = prev.get(&c.id).expect("case missing in --from");
                let llm_raw = o["llm_raw"].as_str().map(str::to_string);
                let err = o["error"].as_str().map(str::to_string);
                println!("{}", judge(&c, &ctx, &session, llm_raw, err, o["llm_ms"].as_u64().unwrap_or(0)));
            }
            m => panic!("unknown mode {m}"),
        }
    }
    if or_cleaner.is_some() {
        eprintln!("openrouter: summed usage.cost ${spent:.4}");
        // The key's own usage counter lags a little behind the calls.
        std::thread::sleep(Duration::from_secs(5));
        if let (Some(a), Some(b)) = (usage_start, key_usage()) {
            eprintln!("openrouter key usage at end: ${b:.4} (this run ${:.4})", b - a);
        }
    }
}
