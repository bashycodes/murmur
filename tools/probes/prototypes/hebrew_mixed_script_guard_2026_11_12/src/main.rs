//! Dated probe (2026-11-12, `feat/hebrew-mixed-script-guard`): runs D65's
//! `script_lock::added_foreign` over stored cleanup answers to Hebrew
//! dictations and prints each one it would reject.
//!
//! usage: mixed-script-scan CASES_DIR RESULTS_DIR...
//!   CASES_DIR: the `he_cases.jsonl` / `he_holdout.jsonl` inputs
//!     (`tools/probes/prototypes/hebrew_stays_hebrew_2026_11_07`).
//!   RESULTS_DIR: directories of runner `*.jsonl` rows (`id`, `text`,
//!     `raw`), e.g. that probe's `results/` and the GPU bench's
//!     (`gpu_cleanup_model_2026_11_12/results`, on `feat/gpu-cleanup-model`).
//!
//! Only answers the share check (`drifted_from`) already passes count: the
//! rest fall back anyway. No dictionary or screen words (none in the
//! cases). The inputs are made up; so are the answers' contexts.

use std::collections::HashMap;
use std::path::PathBuf;

use murmur_core::script_lock::{self, Script};

fn rows(path: &PathBuf) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cases, results)) = args.split_first() else {
        eprintln!("usage: mixed-script-scan CASES_DIR RESULTS_DIR...");
        std::process::exit(2);
    };
    let mut inputs: HashMap<String, String> = HashMap::new();
    for f in ["he_cases.jsonl", "he_holdout.jsonl"] {
        for v in rows(&PathBuf::from(cases).join(f)) {
            if let (Some(i), Some(t)) = (v["id"].as_str(), v["input"].as_str()) {
                inputs.insert(i.into(), t.into());
            }
        }
    }
    let mut files: Vec<PathBuf> = results
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    let (mut checked, mut flagged) = (0, 0);
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy();
        for v in rows(f) {
            let Some(src) = v["id"].as_str().and_then(|i| inputs.get(i)) else {
                continue;
            };
            if script_lock::dominant(src) != Some(Script::Hebrew) {
                continue;
            }
            for key in ["text", "raw"] {
                let Some(t) = v[key].as_str() else { continue };
                if script_lock::drifted_from(Script::Hebrew, src, t) {
                    continue;
                }
                checked += 1;
                if let Some(why) = script_lock::added_foreign(Script::Hebrew, src, t, &[]) {
                    flagged += 1;
                    println!("{name} {} {key}: {why} :: {t:?}", v["id"]);
                }
            }
        }
    }
    println!("checked {checked}, flagged {flagged}");
}
