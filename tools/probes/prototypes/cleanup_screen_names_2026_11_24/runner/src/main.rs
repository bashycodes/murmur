//! screen-names-clean (from the server_cleanup_speed_2026_11_07 runner):
//! replay synthetic transcripts with on-screen words through the real
//! `MurmurCore` commit path (murmur-server's: rules, then prompt v2 on the
//! local model) and print one JSON row per case.
//!
//!   screen-names-clean --cases CASES.jsonl --session SESSION.json
//!       --llm MODEL.gguf --llm-lib SO --arm new|old|none [--threads N]
//!
//! Arms:
//! * `new`  the prompt this checkout builds (D108: a filtered SCREEN NAMES
//!   line);
//! * `old`  the same prompt rewritten to what main sent before D108: no
//!   SCREEN NAMES line, every raw screen word appended to the DICTIONARY
//!   line (at most 200 entries);
//! * `none` no screen words in the context at all.
//!
//! Case rows: {id, variant, input, screen: [..], planted: [..]}.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use murmur_core::asr::{AbortFlag, AsrEngine, AsrError, AsrOutput, AsrRequest};
use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use murmur_core::config::{FieldContext, SessionConfig};
use murmur_core::MurmurCore;

/// The transcript to "recognise".
struct TextAsr(Arc<Mutex<String>>);
impl AsrEngine for TextAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: self.0.lock().unwrap().clone(),
            language: Some("en".into()),
            ..Default::default()
        })
    }
}

/// Labels of the per-dictation lines after the DICTIONARY line.
const AFTER_DICT: [&str; 5] = [
    "TEXT BEFORE CURSOR: ",
    "SELECTED TEXT",
    "TEXT AFTER CURSOR: ",
    "LANGUAGE: ",
    "TRANSCRIPT: ",
];

/// The pre-D108 user message: the SCREEN NAMES line and its rule go, the
/// raw screen words join the DICTIONARY line.
fn old_user(p: &CleanPrompt) -> String {
    let screen: Vec<String> = p
        .source
        .as_ref()
        .map(|s| s.ctx.screen_words.clone())
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut dict: Vec<String> = Vec::new();
    let mut skip_rule = false;
    for l in p.user.lines() {
        if skip_rule {
            skip_rule = false;
            continue;
        }
        if l.starts_with("SCREEN NAMES: ") {
            skip_rule = true;
            continue;
        }
        if let Some(d) = l.strip_prefix("DICTIONARY: ") {
            dict.push(d.to_string());
            continue;
        }
        lines.push(l.to_string());
    }
    dict.extend(screen);
    dict.truncate(200);
    let dict: Vec<String> = dict.into_iter().filter(|d| !d.trim().is_empty()).collect();
    if !dict.is_empty() {
        let at = lines
            .iter()
            .position(|l| AFTER_DICT.iter().any(|a| l.starts_with(a)))
            .unwrap_or(lines.len());
        lines.insert(at, format!("DICTIONARY: {}", dict.join(", ")));
    }
    lines.join("\n")
}

/// A cleaner that reports itself as the server's and may rewrite the prompt.
struct As {
    inner: Arc<dyn Cleaner>,
    old: bool,
    last_user: Mutex<String>,
}

impl As {
    fn fix(&self, p: &CleanPrompt) -> CleanPrompt {
        let mut p = p.clone();
        if self.old {
            p.user = old_user(&p);
        }
        *self.last_user.lock().unwrap() = p.user.clone();
        p
    }
}

impl Cleaner for As {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.inner.clean(&self.fix(p), d)
    }
    fn clean_cancellable(
        &self,
        p: &CleanPrompt,
        d: Instant,
        c: &AbortFlag,
    ) -> Result<String, CleanError> {
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
        "server"
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| {
        argv.iter()
            .position(|a| a == k)
            .and_then(|i| argv.get(i + 1).cloned())
    };
    let arm = get("--arm").expect("--arm new|old|none");
    assert!(
        matches!(arm.as_str(), "new" | "old" | "none"),
        "--arm {arm}"
    );
    let cases = std::fs::read_to_string(get("--cases").expect("--cases")).expect("cases");
    let session: SessionConfig = serde_json::from_str(
        &std::fs::read_to_string(get("--session").expect("--session")).expect("session"),
    )
    .expect("session json");
    let threads: usize = get("--threads").map_or(4, |t| t.parse().expect("--threads"));
    let t = Instant::now();
    let llm = murmur_core::llama::LlamaCleaner::load(
        &get("--llm").expect("--llm"),
        threads,
        get("--llm-lib").as_deref(),
    )
    .expect("llm load");
    eprintln!("llm load {} ms", t.elapsed().as_millis());
    let w = Arc::new(As {
        inner: Arc::new(llm),
        old: arm == "old",
        last_user: Mutex::new(String::new()),
    });
    let text = Arc::new(Mutex::new(String::new()));
    let core = MurmurCore::from_engines(
        Arc::new(TextAsr(text.clone())),
        Some(w.clone() as Arc<dyn Cleaner>),
    );
    // One second of tone so VAD and the empty-audio checks pass.
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    let run = |input: &str, ctx: &FieldContext| {
        *text.lock().unwrap() = input.to_string();
        core.start_session(session.clone()).expect("session");
        core.set_context(ctx.clone());
        core.push_audio(&tone);
        core.commit()
    };
    // The product prefills the static prefix at model load; here it is
    // prefilled once with a generous deadline so the first case does not
    // time out on it.
    let warm_ctx = FieldContext {
        context_known: true,
        ..Default::default()
    };
    let p = murmur_core::cleanup::build_prompt_versioned(
        "This is a warm up sentence for the cache.",
        session.cleanup_level,
        session.styles.for_category(warm_ctx.app_category),
        &warm_ctx,
        &session.dictionary,
        murmur_core::cleanup::PromptScope::FULL,
        murmur_core::cleanup::PromptVersion::V2,
    );
    let t = Instant::now();
    let _ = w.clean(&p, Instant::now() + std::time::Duration::from_secs(600));
    eprintln!("warm {} ms", t.elapsed().as_millis());
    let _ = run("This is a warm up sentence for the cache.", &warm_ctx);
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let c: serde_json::Value = serde_json::from_str(line).expect("case json");
        let screen: Vec<String> = if arm == "none" {
            Vec::new()
        } else {
            serde_json::from_value(c["screen"].clone()).expect("screen")
        };
        let ctx = FieldContext {
            context_known: true,
            app_category: murmur_core::config::AppCategory::PersonalChat,
            screen_words: screen,
            ..Default::default()
        };
        w.last_user.lock().unwrap().clear();
        let r = run(c["input"].as_str().unwrap_or_default(), &ctx);
        let user = std::mem::take(&mut *w.last_user.lock().unwrap());
        let last = w
            .info_json()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .map(|v| v["last"].clone());
        // The core counts the prompt it built; the `old` arm rewrites it, so
        // its counts are the rewritten DICTIONARY line's entries (the
        // session's dictionary is empty: the screen words, at most 200).
        let counts = if arm == "old" {
            let n = ctx.screen_words.iter().take(200);
            (n.filter(|w| !w.trim().is_empty()).count(), 0)
        } else {
            let c = r.timings.prompt_counts;
            (c.dict_terms, c.screen_names)
        };
        let names = user
            .lines()
            .find_map(|l| l.strip_prefix("SCREEN NAMES: "))
            .map(str::to_string);
        println!(
            "{}",
            serde_json::json!({
                "id": c["id"], "variant": c["variant"], "arm": arm,
                "planted": c["planted"], "input": c["input"],
                "text": r.final_text, "status": r.status,
                "fallback_reason": r.fallback_reason,
                "llm_ms": r.timings.llm_ms,
                "dict_terms_in_prompt": counts.0,
                "screen_names_in_prompt": counts.1,
                "user_chars": user.chars().count(),
                "screen_names": names, "last": last
            })
        );
    }
}
