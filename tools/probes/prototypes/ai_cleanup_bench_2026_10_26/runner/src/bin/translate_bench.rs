//! Translation hotkey check (2026-11-13, D90). Runs `translate_cases.jsonl`
//! (synthetic: Hebrew dictations to translate into English and English ones
//! into Hebrew) through `murmur_core::translate::run` as shipped.
//!
//! * `--llm M.gguf --llm-lib libmurmur_llm.so`: the on-device model, to
//!   show why translation routes to the server or cloud first.
//! * `--openrouter MODEL` (key from `OPENROUTER_KEY` in the env, never
//!   printed): the product's OpenRouter cleaner.
//!
//! One JSON row per case on stdout: id, source, target, the answer (or error
//! kind), its dominant script, pass (the answer is in the target's script),
//! and the share of `keep` words (names, numbers) found in the answer.

use std::sync::Arc;
use std::time::{Duration, Instant};

use murmur_core::cleanup::Cleaner;
use murmur_core::dictionary::Dictionary;
use murmur_core::script_lock;
use murmur_core::translate;
use serde_json::{json, Value};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn cleaner(timeout: Duration) -> Arc<dyn Cleaner> {
    if let Some(m) = arg("--openrouter") {
        let key =
            murmur_router::SecretString::new(std::env::var("OPENROUTER_KEY").unwrap_or_default());
        assert!(
            !key.is_empty(),
            "--openrouter needs OPENROUTER_KEY in the env"
        );
        let settings = murmur_cloud::settings::CleanupSettings {
            model: m.clone(),
            timeout_ms: timeout.as_millis() as u64,
            ..Default::default()
        };
        let api = murmur_cloud::api::Api::new();
        let info = murmur_cloud::catalog::lookup_model(&api, &m).ok();
        return Arc::new(murmur_cloud::OpenRouterCleaner::new(settings, key).with_model_info(info));
    }
    let threads: usize = arg("--threads").map_or(4, |t| t.parse().unwrap());
    Arc::new(
        murmur_core::llama::LlamaCleaner::load(
            &arg("--llm").expect("--llm or --openrouter"),
            threads,
            arg("--llm-lib").as_deref(),
        )
        .expect("llm load"),
    )
}

fn main() {
    let cases = std::fs::read_to_string(arg("--cases").expect("--cases")).unwrap();
    let timeout = Duration::from_millis(arg("--timeout-ms").map_or(25_000, |t| t.parse().unwrap()));
    let cl = cleaner(timeout);
    let dict = Dictionary::default();
    let (mut pass, mut n) = (0, 0);
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let c: Value = serde_json::from_str(line).unwrap();
        let (text, source, target) = (
            c["text"].as_str().unwrap(),
            c["source"].as_str(),
            c["target"].as_str().unwrap(),
        );
        let t0 = Instant::now();
        let r = translate::run(Some(cl.clone()), text, source, target, &dict, timeout);
        let ms = t0.elapsed().as_millis();
        let (answer, err) = match &r {
            Ok(t) => (t.as_str(), None),
            Err(e) => ("", Some(e.kind())),
        };
        let want = translate::script_of(target);
        let got = script_lock::dominant(answer);
        let ok = err.is_none() && want.is_some() && got == want;
        let keep: Vec<&str> = c["keep"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let kept = keep.iter().filter(|k| answer.contains(**k)).count();
        n += 1;
        pass += ok as usize;
        println!(
            "{}",
            json!({"id": c["id"], "source": source, "target": target, "script": format!("{got:?}"),
                   "error": err, "pass": ok, "kept": format!("{kept}/{}", keep.len()),
                   "ms": ms, "answer": answer})
        );
    }
    eprintln!("{pass}/{n} answered in the target's script");
}
