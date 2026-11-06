//! Real llama.cpp cleanup through the whole commit pipeline (mock ASR text
//! → LlamaCleaner plugin → post-processing; no output guard, D59).
//!
//! Skips with a note when the model or the plugin is missing:
//! * model: `FLOW_LLM_MODEL`, else `models/qwen2.5-0.5b-instruct-q4_k_m.gguf`
//!   (`tools/fetch-models.sh --llm`);
//! * plugin: `libflow_llm.so`, built by `cargo test --workspace --all-features`
//!   (or `cargo build -p flow-llm`) next to the test binary.
//!
//! Two passes over 8 messy dictations:
//! 1. at the real budget (`SessionConfig::llm_timeout_ms`, 1.5 s, or
//!    `FLOW_LLM_BUDGET_MS`): latency is printed, the safety contract asserted;
//! 2. at a generous budget (`FLOW_LLM_QUALITY_BUDGET_MS`, 30 s): quality is
//!    graded per case and at least `FLOW_LLM_MIN_GOOD` (3) must pass.
//!
//! The safety contract: never ERROR, never an empty paste, never a
//! preamble/HTML/label pasted, cleanup never over budget. Pass 1 is not
//! graded on quality because on a shared CPU a timeout (→ RAW_TRANSCRIPT,
//! which is the designed fallback) depends on the neighbours' load.
#![cfg(feature = "llm")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use flow_core::config::{AppCategory, AsrBackend, FieldContext, ModelConfig, SessionConfig};
use flow_core::{CommitStatus, FlowCore};

fn model_path() -> Option<PathBuf> {
    let p = std::env::var_os("FLOW_LLM_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../models/qwen2.5-0.5b-instruct-q4_k_m.gguf")
        });
    p.is_file().then_some(p)
}

fn plugin_present() -> bool {
    flow_core::llama::plugin_candidates(None)
        .iter()
        .any(|p| p.is_absolute() && p.is_file())
}

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

struct Case {
    name: &'static str,
    asr: &'static str,
    cat: AppCategory,
    /// Every needle must appear (case-insensitive) in a good cleanup.
    want: &'static [&'static str],
    /// No needle may appear (case-insensitive, word-ish) in a good cleanup.
    not: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        name: "fillers+time",
        asr: "um so i was thinking uh we should meet tomorrow at like three thirty pm",
        cat: AppCategory::Other,
        want: &["meet tomorrow", "3:30"],
        not: &[" um", " uh"],
    },
    Case {
        name: "scratch that",
        asr: "send the report to john on friday scratch that send it to sarah on monday",
        cat: AppCategory::WorkChat,
        want: &["sarah", "monday"],
        not: &["john", "scratch that"],
    },
    Case {
        name: "spoken punctuation",
        asr: "can you grab milk comma eggs comma and bread period thanks",
        cat: AppCategory::PersonalChat,
        want: &["milk,", "eggs,"],
        not: &["comma", "period"],
    },
    Case {
        name: "numbered list",
        asr: "first we need to update the docs second fix the login bug and third ship the release",
        cat: AppCategory::Document,
        want: &["1.", "2.", "3.", "login bug"],
        not: &["<li>", "<ol>"],
    },
    Case {
        name: "no wait",
        asr: "the meeting is on tuesday no wait wednesday at ten am",
        cat: AppCategory::Other,
        want: &["wednesday", "10"],
        not: &["tuesday", "no wait"],
    },
    Case {
        name: "question not answered",
        asr: "what time does the grocery store close on sundays",
        cat: AppCategory::AiPrompt,
        want: &["grocery store", "sunday", "?"],
        not: &["closes at", "typically", "i don't"],
    },
    Case {
        name: "stutters+you know",
        asr: "i i think the the problem is is in the cache layer you know",
        cat: AppCategory::WorkChat,
        want: &["problem is in the cache layer"],
        not: &["the the", "is is"],
    },
    Case {
        name: "email paragraphs",
        asr: "hi bob new paragraph thanks for the update i will review it tonight new paragraph best alice",
        cat: AppCategory::Email,
        want: &["bob", "review it tonight", "alice", "\n"],
        not: &["new paragraph"],
    },
];

/// The model tests share 4 cores; running them in parallel would make each
/// one measure the other's load.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn has(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

#[test]
fn messy_dictation_is_cleaned_safely_and_fast() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(model) = model_path() else {
        eprintln!("SKIP: LLM model missing (tools/fetch-models.sh --llm or FLOW_LLM_MODEL)");
        return;
    };
    if !plugin_present() && std::env::var_os("FLOW_LLM_LIB").is_none() {
        eprintln!("SKIP: libflow_llm plugin not built (cargo build -p flow-llm)");
        return;
    }
    let budget_ms = env_u64(
        "FLOW_LLM_BUDGET_MS",
        SessionConfig::default().llm_timeout_ms,
    );

    let t = Instant::now();
    // One core per case, re-created engines would hide the warm path; the
    // mock text is swapped per case through a shared cell instead.
    let text = Arc::new(std::sync::Mutex::new(String::new()));
    let text_ref = text.clone();
    let asr = Arc::new(SwapAsr(text.clone()));
    let cleaner = flow_core::llama::LlamaCleaner::load(
        model.to_str().unwrap(),
        flow_core::config::default_threads(),
        None,
    )
    .expect("load llm");
    let load_ms = t.elapsed().as_millis();
    // The plugin says where the model runs; a CPU build runs it on the CPU.
    let backend = cleaner.backend().expect("plugin reports its backend");
    let (probe_path, probed) = flow_core::llama::probe_backend(None).expect("probe");
    assert_eq!(probe_path, cleaner.plugin_path);
    assert_eq!(
        probed.is_gpu(),
        backend.is_gpu(),
        "{probed:?} vs {backend:?}"
    );
    if !backend.is_gpu() {
        assert_eq!(backend.device, "CPU");
    }
    eprintln!(
        "model {} | plugin {} | load {load_ms} ms | {}",
        model.display(),
        cleaner.plugin_path.display(),
        cleaner.info
    );
    let core = FlowCore::from_engines(asr, Some(Arc::new(cleaner)));

    let (_, f, p50, max) = run_pass(&core, &text_ref, budget_ms);
    eprintln!(
        "^ real budget: formatted {f}/{} | llm p50 {p50} ms max {max} ms | budget {budget_ms} ms",
        CASES.len()
    );
    // Quality is a property of model + prompt, not of how loaded the shared
    // CPU is, so it is graded on a pass with a generous budget.
    let qbudget = env_u64("FLOW_LLM_QUALITY_BUDGET_MS", 30_000);
    let (good, f, p50, max) = run_pass(&core, &text_ref, qbudget);
    let min_good = env_u64("FLOW_LLM_MIN_GOOD", 3) as usize;
    eprintln!(
        "^ quality {good}/{} (min {min_good}) | formatted {f}/{} | llm p50 {p50} ms max {max} ms",
        CASES.len(),
        CASES.len()
    );
    assert!(good >= min_good, "only {good} cases cleaned as expected");
}

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(d)
}

/// One dictation per case; asserts the safety contract and returns
/// (cases meeting expectations, formatted count, p50 and max llm ms).
fn run_pass(
    core: &FlowCore,
    text: &std::sync::Mutex<String>,
    budget_ms: u64,
) -> (usize, usize, u64, u64) {
    let mut good = 0;
    let mut formatted = 0;
    let mut lat = Vec::new();
    for c in CASES {
        *text.lock().unwrap() = c.asr.to_string();
        core.start_session(SessionConfig {
            llm_timeout_ms: budget_ms,
            ..Default::default()
        })
        .unwrap();
        core.set_context(FieldContext {
            app_category: c.cat,
            ..Default::default()
        });
        for ch in speech(800).chunks(640) {
            core.push_audio(ch);
        }
        let r = core.commit();
        // Safety contract.
        assert!(
            matches!(
                r.status,
                CommitStatus::Formatted | CommitStatus::RawTranscript
            ),
            "{}: {r:?}",
            c.name
        );
        assert!(!r.final_text.trim().is_empty(), "{}: empty paste", c.name);
        let reason = r.fallback_reason.clone().unwrap_or_default();
        assert!(!reason.starts_with("llm_error"), "{}: {reason}", c.name);
        // run_cleanup's hard timeout + scheduling slack.
        assert!(r.timings.llm_ms <= budget_ms + 150, "{}: {r:?}", c.name);
        if r.status == CommitStatus::Formatted {
            formatted += 1;
            lat.push(r.timings.llm_ms);
            for bad in [
                "here is",
                "here's",
                "cleaned",
                "<li>",
                "<ul>",
                "<ol>",
                "transcript",
                "«",
            ] {
                assert!(
                    !has(&r.final_text, bad),
                    "{}: leaked {bad:?}: {:?}",
                    c.name,
                    r.final_text
                );
            }
        }
        let ok = r.status == CommitStatus::Formatted
            && c.want.iter().all(|w| has(&r.final_text, w))
            && c.not.iter().all(|n| !has(&r.final_text, n));
        good += ok as usize;
        eprintln!(
            "[{}] {:<22} {:?} llm {} ms {} -> {:?}",
            if ok { "ok" } else { "--" },
            c.name,
            r.status,
            r.timings.llm_ms,
            r.fallback_reason.as_deref().unwrap_or(""),
            r.final_text
        );
    }
    lat.sort_unstable();
    let p50 = lat.get(lat.len() / 2).copied().unwrap_or(0);
    let max = lat.last().copied().unwrap_or(0);
    (good, formatted, p50, max)
}

struct SwapAsr(Arc<std::sync::Mutex<String>>);
impl flow_core::asr::AsrEngine for SwapAsr {
    fn transcribe(
        &self,
        _pcm: &[f32],
        _req: &flow_core::asr::AsrRequest,
    ) -> Result<flow_core::asr::AsrOutput, flow_core::asr::AsrError> {
        Ok(flow_core::asr::AsrOutput {
            text: self.0.lock().unwrap().clone(),
            language: Some("en".into()),
            ..Default::default()
        })
    }
}

#[test]
fn load_models_wires_llm_path() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(model) = model_path() else {
        eprintln!("SKIP: LLM model missing");
        return;
    };
    if !plugin_present() && std::env::var_os("FLOW_LLM_LIB").is_none() {
        eprintln!("SKIP: libflow_llm plugin not built");
        return;
    }
    let core = FlowCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Mock {
            text: "um hello there".into(),
        },
        llm_path: Some(model.to_string_lossy().into_owned()),
        ..Default::default()
    })
    .expect("load");
    core.start_session(SessionConfig {
        llm_timeout_ms: 10_000,
        ..Default::default()
    })
    .unwrap();
    for ch in speech(800).chunks(640) {
        core.push_audio(ch);
    }
    let r = core.commit();
    assert!(r.timings.llm_ms > 0, "LLM did not run: {r:?}");
    assert!(
        matches!(
            r.status,
            CommitStatus::Formatted | CommitStatus::RawTranscript
        ),
        "{r:?}"
    );
    // Raw mode still bypasses the loaded model.
    core.start_session(SessionConfig {
        raw_mode: true,
        ..Default::default()
    })
    .unwrap();
    for ch in speech(800).chunks(640) {
        core.push_audio(ch);
    }
    let r = core.commit();
    assert_eq!(r.timings.llm_ms, 0);
    assert_eq!(r.status, CommitStatus::RawTranscript);
    core.unload();
}

#[test]
fn bad_model_path_fails_load() {
    if !plugin_present() && std::env::var_os("FLOW_LLM_LIB").is_none() {
        eprintln!("SKIP: libflow_llm plugin not built");
        return;
    }
    let e = FlowCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Mock { text: "x".into() },
        llm_path: Some("/nonexistent.gguf".into()),
        ..Default::default()
    })
    .err()
    .expect("must fail");
    assert!(e.to_string().contains("not found"), "{e}");
}

fn sim(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let n = a.len().max(b.len());
    if n == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let c = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / n as f64
}

/// The benchmark's 20 cases through the full engine (spoken cleanup → LLM
/// → guard → post-processing). Opt-in (`FLOW_LLM_BENCH=1`; ~2-5 min); a
/// report, not a gate: prints status and similarity per case and the mean.
/// Budget: `FLOW_LLM_QUALITY_BUDGET_MS` (default 30 s).
#[test]
fn bench_cases_through_the_engine() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if std::env::var_os("FLOW_LLM_BENCH").is_none() {
        eprintln!("SKIP: set FLOW_LLM_BENCH=1 (and FLOW_LLM_MODEL) to run the 20-case report");
        return;
    }
    let Some(model) = model_path() else {
        eprintln!("SKIP: LLM model missing");
        return;
    };
    let j: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bench_cleanup_cases.json")).unwrap();
    let text = Arc::new(std::sync::Mutex::new(String::new()));
    let cleaner = flow_core::llama::LlamaCleaner::load(
        model.to_str().unwrap(),
        flow_core::config::default_threads(),
        None,
    )
    .expect("load llm");
    let core = FlowCore::from_engines(Arc::new(SwapAsr(text.clone())), Some(Arc::new(cleaner)));
    let budget = env_u64("FLOW_LLM_QUALITY_BUDGET_MS", 30_000);
    let (mut sum_raw, mut sum, mut n) = (0.0, 0.0, 0.0);
    let mut statuses = std::collections::BTreeMap::<String, usize>::new();
    for c in j["cases"].as_array().unwrap() {
        let input = c["input"].as_str().unwrap();
        let want = c["expected"].as_str().unwrap();
        let cat = match c["category"].as_str().unwrap() {
            "email" => AppCategory::Email,
            "work" => AppCategory::WorkChat,
            "personal" => AppCategory::PersonalChat,
            _ => AppCategory::Other,
        };
        *text.lock().unwrap() = input.to_string();
        core.start_session(SessionConfig {
            llm_timeout_ms: budget,
            ..Default::default()
        })
        .unwrap();
        core.set_context(FieldContext {
            app_id: c["app"].as_str().unwrap().to_string(),
            app_category: cat,
            context_known: false,
            ..Default::default()
        });
        core.push_audio(&speech(1000));
        let r = core.commit();
        assert_ne!(r.status, CommitStatus::Error, "{r:?}");
        let got = r.final_text.trim();
        let s = sim(got, want);
        sum += s;
        sum_raw += sim(input, want);
        n += 1.0;
        let key = format!(
            "{:?} {}",
            r.status,
            r.fallback_reason.clone().unwrap_or_default()
        );
        *statuses.entry(key).or_default() += 1;
        eprintln!(
            "{} {:?} {:?} llm {} ms sim {s:.3} | {got:?}",
            c["id"], r.status, r.fallback_reason, r.timings.llm_ms
        );
    }
    eprintln!(
        "model {} | mean sim {:.3} (raw {:.3}) | {statuses:?}",
        model.display(),
        sum / n,
        sum_raw / n
    );
}

/// Load 3: a language switch on the phone frees the engine and loads the
/// other speech model; the LLM kept from the old engine is reused instead
/// of loaded again (~38 s for the 1.5B on the phone), once.
#[test]
fn a_language_switch_reuses_the_kept_llm() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(model) = model_path() else {
        eprintln!("SKIP: LLM model missing");
        return;
    };
    if !plugin_present() && std::env::var_os("FLOW_LLM_LIB").is_none() {
        eprintln!("SKIP: libflow_llm plugin not built");
        return;
    }
    let cfg = ModelConfig {
        asr_backend: AsrBackend::Mock {
            text: "english words".into(),
        },
        languages: std::collections::BTreeMap::from([(
            "he".to_string(),
            flow_core::config::LanguageModel {
                asr_backend: AsrBackend::Mock {
                    text: "מילים בעברית".into(),
                },
                ..Default::default()
            },
        )]),
        llm_path: Some(model.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let en = FlowCore::load_models_for(&cfg, Some("en")).expect("load en");
    let first = en.load_timings().llm().expect("llm timing").clone();
    assert!(!first.reused && first.ms > 0, "{first:?}");
    let before = Arc::as_ptr(&en.cleaner().unwrap()) as *const () as usize;
    assert!(en.keep_llm());
    drop(en);

    let t = Instant::now();
    let he = FlowCore::load_models_for(&cfg, Some("he")).expect("load he");
    let llm = he.load_timings().llm().unwrap();
    assert!(llm.reused && llm.ms == 0, "{llm:?}");
    assert_eq!(
        Arc::as_ptr(&he.cleaner().unwrap()) as *const () as usize,
        before,
        "the same LLM instance"
    );
    eprintln!(
        "switch: {} ms (LLM load was {} ms)",
        t.elapsed().as_millis(),
        first.ms
    );
    // Kept for one load only: without a new keep_llm() the next load loads it.
    drop(he);
    let en = FlowCore::load_models_for(&cfg, Some("en")).expect("reload en");
    assert!(!en.load_timings().llm().unwrap().reused);
}
