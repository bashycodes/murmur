//! Behaviour probe for the text post-processing rewrite (2026-11-09).
//!
//! Runs invented inputs (`cases.json` next to this file) through the stable
//! public API only (`postprocess::apply`, `effective_category`,
//! `learn::learn_edit`, `DictMatcher::whole_snippet`) and prints one JSON
//! line per case. Run it on two trees and diff the outputs:
//!
//! ```text
//! cp tools/probes/prototypes/cleanroom_2026_11_09/probe.rs crates/flow-core/tests/zz_probe.rs
//! PROBE_CASES=$PWD/tools/probes/prototypes/cleanroom_2026_11_09/cases.json \
//!   cargo test -p flow-core --test zz_probe -- --nocapture > out.jsonl
//! rm crates/flow-core/tests/zz_probe.rs
//! ```
//!
//! Not promoted; the spec it informed is `docs/specs/text-postprocess.md`.

use std::sync::{Arc, Mutex};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::config::{AppCategory, FieldContext, Platform, SessionConfig};
use flow_core::dictionary::Dictionary;
use flow_core::learn::learn_edit;
use flow_core::postprocess::{self, DictMatcher};
use flow_core::FlowCore;
use serde_json::{json, Value};

struct TextAsr(Arc<Mutex<String>>);
impl AsrEngine for TextAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: self.0.lock().unwrap().clone(),
            ..Default::default()
        })
    }
}

/// Eval mode (`PROBE_FE_CASES` = formatting-eval style JSONL, `PROBE_FE_SESSION`
/// = session JSON): each case's `input` as the transcript through the real
/// `FlowCore::commit` path with no LLM (the fallback rules + post-processing),
/// one `{"id", "output"}` line per case, as formatting_eval's score.py reads.
fn eval_mode(cases: &str, session: &str) {
    let session: SessionConfig = serde_json::from_str(&std::fs::read_to_string(session).unwrap()).unwrap();
    let text = Arc::new(Mutex::new(String::new()));
    let core = FlowCore::from_engines(Arc::new(TextAsr(text.clone())), None);
    let tone: Vec<i16> = (0..16_000).map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16).collect();
    for line in std::fs::read_to_string(cases).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let c: Value = serde_json::from_str(line).unwrap();
        *text.lock().unwrap() = c["input"].as_str().unwrap().to_string();
        core.start_session(session.clone()).unwrap();
        core.set_context(FieldContext {
            app_id: c["app_id"].as_str().unwrap_or("").into(),
            app_category: serde_json::from_value(c["app_category"].clone()).unwrap_or(AppCategory::Other),
            ..Default::default()
        });
        core.push_audio(&tone);
        let r = core.commit();
        println!("{}", json!({"id": c["id"], "output": r.final_text}));
    }
}

#[test]
fn probe() {
    if let (Ok(c), Ok(s)) = (std::env::var("PROBE_FE_CASES"), std::env::var("PROBE_FE_SESSION")) {
        return eval_mode(&c, &s);
    }
    let path = std::env::var("PROBE_CASES").expect("PROBE_CASES");
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for c in cases {
        let id = c["id"].as_str().unwrap_or("?");
        let kind = c["kind"].as_str().unwrap_or("apply");
        let cfg: SessionConfig = serde_json::from_value(c.get("cfg").cloned().unwrap_or(json!({}))).unwrap();
        let ctx: FieldContext = serde_json::from_value(c.get("ctx").cloned().unwrap_or(json!({}))).unwrap();
        let out = match kind {
            "apply" => {
                let text = c["text"].as_str().unwrap();
                let raw = c.get("raw").and_then(Value::as_str).unwrap_or(text);
                let o = postprocess::apply(text, raw, &cfg, &ctx);
                json!({"out": o.text, "enter": o.press_enter})
            }
            "category" => {
                let p: Platform = serde_json::from_value(c.get("platform").cloned().unwrap_or(json!("windows"))).unwrap();
                json!({"out": postprocess::effective_category(p, &ctx)})
            }
            "snippet" => {
                let d: Dictionary = serde_json::from_value(c["dict"].clone()).unwrap();
                let m = DictMatcher::new(&d.entries, &[]);
                json!({"out": m.whole_snippet(c["text"].as_str().unwrap())})
            }
            "learn" => {
                let d: Dictionary = serde_json::from_value(c.get("dict").cloned().unwrap_or(json!([]))).unwrap();
                let r = learn_edit(c["pasted"].as_str().unwrap(), c["edited"].as_str().unwrap(), &d);
                json!({"out": r})
            }
            _ => json!({"out": "unknown kind"}),
        };
        println!("{}", json!({"id": id, "r": out}));
    }
}
