//! The guardrail against 120 real LLM outputs from the cleanup benchmark,
//! each judged A (paste-ready), M (safe miss) or H (harmful) by hand.
//! Harmful outputs must be rejected; paste-ready ones mostly accepted.
//! The engine applies the guard after list flattening (`lists_to_plain`),
//! so the fixture outputs get the same treatment here.

use flow_core::cleanup::lists_to_plain;
use flow_core::guard::{check, Verdict};

#[test]
fn guard_rejects_harm_and_keeps_good_edits() {
    let j: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bench_llm_outputs.json")).unwrap();
    let mut stats = std::collections::BTreeMap::<(&str, bool), usize>::new();
    let mut old = std::collections::BTreeMap::<(&str, bool), usize>::new();
    let mut missed_harm = Vec::new();
    let mut rejected_good = Vec::new();
    for r in j["rows"].as_array().unwrap() {
        let (input, output) = (r["input"].as_str().unwrap(), r["output"].as_str().unwrap());
        let jd = r["judgement"].as_str().unwrap();
        let out = lists_to_plain(output);
        let v = check(input, &out);
        let ok = v == Verdict::Ok;
        let old_ok = flow_core::align::divergence_ratio(input, &out) <= 0.6;
        *old.entry((jd, old_ok)).or_default() += 1;
        *stats.entry((jd, ok)).or_default() += 1;
        let tag = format!("{} {} {:?} | {:?}", r["model"], r["id"], v, out);
        if jd == "H" && ok {
            missed_harm.push(tag);
        } else if jd == "A" && !ok {
            rejected_good.push(tag);
        }
    }
    eprintln!("(judgement, accepted) -> count: {stats:?}");
    eprintln!("old 03 §7.1 guard (ratio <= 0.6): {old:?}");
    eprintln!("missed harm: {missed_harm:#?}");
    eprintln!("rejected good: {rejected_good:#?}");
    let a_total: usize = stats
        .iter()
        .filter(|((j, _), _)| *j == "A")
        .map(|(_, n)| n)
        .sum();
    assert!(missed_harm.is_empty(), "{missed_harm:#?}");
    assert!(
        rejected_good.len() * 10 <= a_total,
        "{} of {a_total} good outputs rejected",
        rejected_good.len()
    );
}
