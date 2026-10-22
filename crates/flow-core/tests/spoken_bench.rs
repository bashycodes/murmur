//! The 20-case cleanup benchmark set (docs/model-benchmarks.md, copied from
//! tools/probes/bench/cleanup_cases.json on the benchmark branch) run
//! through the deterministic spoken-cleanup pass, with no LLM.
//!
//! Similarity = 1 - char edit distance / max length, as in the benchmark
//! (pasting the raw input scores 0.712). The pass must beat raw on
//! average, never make a case worse, and get the rule-shaped cases exact.

use flow_core::cleanup::lists_to_plain;
use flow_core::config::AppCategory;
use flow_core::postprocess::spoken::{layout, spoken_cleanup};

fn category(c: &str) -> AppCategory {
    match c {
        "email" => AppCategory::Email,
        "work" => AppCategory::WorkChat,
        "personal" => AppCategory::PersonalChat,
        _ => AppCategory::Other,
    }
}

fn lev(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let c = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn sim(a: &str, b: &str) -> f64 {
    let n = a.chars().count().max(b.chars().count());
    if n == 0 {
        1.0
    } else {
        1.0 - lev(a, b) as f64 / n as f64
    }
}

/// Cases whose expected output is reachable by rules alone.
const EXACT: &[&str] = &[
    "c03", "c06", "c07", "c09", "c11", "c12", "c14", "c15", "c16", "c17", "c18", "c19",
];

#[test]
fn bench_cases_improve_without_an_llm() {
    let j: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bench_cleanup_cases.json")).unwrap();
    let (mut raw_sum, mut det_sum, mut n) = (0.0, 0.0, 0.0);
    let mut worse = Vec::new();
    let mut not_exact = Vec::new();
    for c in j["cases"].as_array().unwrap() {
        let id = c["id"].as_str().unwrap();
        let input = c["input"].as_str().unwrap();
        // The expected lists are the LLM's HTML; the pipeline pastes them
        // as `1.` / `- ` lines (cleanup::lists_to_plain).
        let want = &lists_to_plain(c["expected"].as_str().unwrap());
        let cat = category(c["category"].as_str().unwrap_or("other"));
        let got = layout(&spoken_cleanup(input), cat);
        let (rs, ds) = (sim(input, want), sim(&got, want));
        eprintln!("{id} raw {rs:.3} det {ds:.3} | {got:?}");
        raw_sum += rs;
        det_sum += ds;
        n += 1.0;
        if ds + 0.02 < rs {
            worse.push(format!("{id}: {got:?}"));
        }
        if EXACT.contains(&id) && got != *want {
            not_exact.push(format!("{id}: {got:?} != {want:?}"));
        }
    }
    let (raw, det) = (raw_sum / n, det_sum / n);
    eprintln!("mean sim: raw {raw:.3}, deterministic {det:.3}");
    assert!(worse.is_empty(), "{worse:#?}");
    assert!(not_exact.is_empty(), "{not_exact:#?}");
    assert!(det >= raw + 0.15, "raw {raw:.3} det {det:.3}");
}
