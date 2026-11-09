//! The replay matrix on fake engines: one ASR call per (speech model,
//! recording), one row per (cleanup model x prompt), prompt versions reach
//! the cloud cleaner, D30 routing shows in the rows, and the spend cap
//! stops cloud calls. No network, no models.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use murmur_core::asr::{AbortFlag, MockAsr};
use murmur_core::cleanup::{
    build_prompt_versioned, CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope,
    PromptVersion,
};
use murmur_core::config::{CleanupLevel, FieldContext, Style};
use murmur_core::Usd;
use murmur_replay::{phone_session, AsrSlot, CleanupSlot, Matrix, Recording, Row};

/// Answers `out`; engine `cloud` or `local`; records system prompts.
struct Fake {
    out: &'static str,
    engine: &'static str,
    cost: f64,
    calls: AtomicUsize,
    systems: Mutex<Vec<String>>,
}

impl Fake {
    fn new(engine: &'static str, out: &'static str) -> Arc<Self> {
        Arc::new(Self {
            out,
            engine,
            cost: 0.001,
            calls: AtomicUsize::new(0),
            systems: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Cleaner for Fake {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.systems.lock().unwrap().push(p.system.clone());
        Ok(self.out.into())
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
    fn prompt_scope(&self) -> PromptScope {
        if self.engine == "cloud" {
            PromptScope::MINIMAL
        } else {
            PromptScope::FULL
        }
    }
    fn timeout_ms(&self) -> Option<u64> {
        (self.engine == "cloud").then_some(2500)
    }
    fn clean_metered(
        &self,
        p: &CleanPrompt,
        d: Instant,
        _c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        let r = self.clean(p, d);
        let cost = (self.engine == "cloud").then_some(Usd(self.cost));
        (
            r,
            CleanUsage {
                engine: Some(self.engine.into()),
                model: Some(format!("fake-{}", self.engine)),
                cost_usd: cost,
                ..Default::default()
            },
        )
    }
}

const SAID: &str = "so the meeting is at ten tomorrow and please bring the slides";

fn recs() -> Vec<Recording> {
    ["r1", "r2"]
        .iter()
        .map(|id| Recording {
            id: id.to_string(),
            wav: PathBuf::from(format!("{id}.wav")),
            app_id: "md.obsidian".into(),
            phone: None,
        })
        .collect()
}

fn tone() -> Vec<i16> {
    (0..16_000 * 2)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

/// The phone's session without VAD / segmentation (a sine tone is not
/// speech to the VAD).
fn session() -> murmur_core::config::SessionConfig {
    let mut s = phone_session();
    s.vad = false;
    s.segment_pause_ms = None;
    s
}

fn run(m: &Matrix) -> (Vec<Row>, f64) {
    let mut rows = Vec::new();
    let spend = m.run(&recs(), &mut |_| Ok(tone()), &mut |r| rows.push(r.clone()));
    (rows, spend.total())
}

fn slot(name: &str, cloud: Option<Arc<Fake>>, local: Option<Arc<Fake>>) -> CleanupSlot {
    CleanupSlot {
        name: name.into(),
        cloud: cloud.map(|c| c as Arc<dyn Cleaner>),
        local: local.map(|c| c as Arc<dyn Cleaner>),
    }
}

#[test]
fn matrix_shape_transcript_cache_and_prompt_versions() {
    let asr = Arc::new(MockAsr::new(SAID));
    let cloud = Fake::new(
        "cloud",
        "So the meeting is at 10 tomorrow, and please bring the slides.",
    );
    let local = Fake::new(
        "local",
        "So the meeting is at 10 tomorrow and please bring the slides.",
    );
    let m = Matrix {
        asr: vec![AsrSlot {
            name: "mock".into(),
            engine: asr.clone(),
            call_cost: None,
        }],
        cleanup: vec![
            slot("none", None, None),
            slot("local", None, Some(local.clone())),
            slot("or:fake", Some(cloud.clone()), Some(local.clone())),
        ],
        prompts: vec![PromptVersion::V4c, PromptVersion::V2],
        session: session(),
        max_spend: 1.0,
    };
    let (rows, spent) = run(&m);
    // Per recording: none + local + cloud x 2 prompts.
    assert_eq!(rows.len(), 2 * 4, "{rows:#?}");
    assert_eq!(
        asr.calls(),
        2,
        "one transcription per (speech model, recording)"
    );
    assert_eq!(cloud.calls(), 4);
    assert!((spent - 0.004).abs() < 1e-9, "{spent}");
    let r1: Vec<&Row> = rows.iter().filter(|r| r.rec == "r1").collect();
    let combos: Vec<(String, String)> = r1
        .iter()
        .map(|r| (r.cleanup.clone(), r.prompt.clone()))
        .collect();
    assert_eq!(
        combos,
        [
            ("none", "-"),
            ("local", "v2"),
            ("or:fake", "v4c"),
            ("or:fake", "v2")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    assert!(r1.iter().all(|r| r.transcript == SAID));
    // Rules only: no LLM, no AI route.
    assert_eq!(r1[0].cleanup_route, None);
    assert_eq!(r1[0].llm_engine, None);
    // Today's local pipeline (no cloud): no D30 route fields.
    assert_eq!(r1[1].llm_engine.as_deref(), Some("local"));
    assert_eq!(r1[1].cleanup_route, None);
    // AI cleanup: the cloud answer is used.
    assert_eq!(r1[2].cleanup_route.as_deref(), Some("cloud"));
    // (Casual style: post-processing drops the final period.)
    assert_eq!(
        r1[2].output.trim(),
        "So the meeting is at 10 tomorrow, and please bring the slides"
    );
    assert_eq!(r1[2].cost_usd, Some(0.001));
    // The prompt versions reached the cloud cleaner, in order.
    let sys = |v| {
        build_prompt_versioned(
            SAID,
            CleanupLevel::Light,
            Style::Casual,
            &FieldContext::default(),
            &Default::default(),
            PromptScope::MINIMAL,
            v,
        )
        .system
    };
    let seen = cloud.systems.lock().unwrap().clone();
    assert_eq!(seen[0], sys(PromptVersion::V4c));
    assert_eq!(seen[1], sys(PromptVersion::V2));
    assert_ne!(seen[0], seen[1]);
}

#[test]
fn a_failed_cloud_cleanup_runs_the_local_pipeline() {
    // An empty answer (D59: nothing judges a non-empty one).
    let cloud = Fake::new("cloud", "   ");
    let local = Fake::new(
        "local",
        "So the meeting is at 10 tomorrow and please bring the slides.",
    );
    let m = Matrix {
        asr: vec![AsrSlot {
            name: "mock".into(),
            engine: Arc::new(MockAsr::new(SAID)),
            call_cost: None,
        }],
        cleanup: vec![slot("or:fake", Some(cloud.clone()), Some(local.clone()))],
        prompts: vec![PromptVersion::V4c],
        session: session(),
        max_spend: 1.0,
    };
    let (rows, _) = run(&m);
    // The transcription (`none`) row is always there.
    assert_eq!(rows.iter().filter(|r| r.cleanup == "none").count(), 2);
    let rows: Vec<&Row> = rows.iter().filter(|r| r.cleanup == "or:fake").collect();
    assert_eq!(rows.len(), 2);
    for r in rows {
        assert_eq!(r.cleanup_route.as_deref(), Some("local"), "{r:#?}");
        assert!(
            r.cloud_fallback
                .as_deref()
                .is_some_and(|f| f.starts_with("llm_")),
            "{r:#?}"
        );
        assert!(r.output.contains("slides"), "{r:#?}");
    }
    assert_eq!(local.calls(), 2);
}

#[test]
fn spend_cap_stops_cloud_calls_and_is_reported() {
    let cloud = Fake::new(
        "cloud",
        "So the meeting is at 10 tomorrow, and please bring the slides.",
    );
    let m = Matrix {
        asr: vec![AsrSlot {
            name: "mock".into(),
            engine: Arc::new(MockAsr::new(SAID)),
            call_cost: None,
        }],
        cleanup: vec![
            slot("none", None, None),
            slot("or:fake", Some(cloud.clone()), None),
        ],
        prompts: vec![PromptVersion::V4c, PromptVersion::V2],
        session: session(),
        // Room for two calls at $0.001.
        max_spend: 0.0015,
    };
    let (rows, spent) = run(&m);
    assert_eq!(cloud.calls(), 2);
    assert!((spent - 0.002).abs() < 1e-9);
    let capped: Vec<&Row> = rows
        .iter()
        .filter(|r| r.error.as_deref() == Some("spend_cap"))
        .collect();
    assert_eq!(capped.len(), 2, "{rows:#?}");
    // Rules-only rows are free and always present.
    assert_eq!(
        rows.iter()
            .filter(|r| r.cleanup == "none" && r.error.is_none())
            .count(),
        2
    );
}

#[test]
fn paid_asr_cost_counts_and_failed_asr_skips_cleanup() {
    struct Failing;
    impl murmur_core::asr::AsrEngine for Failing {
        fn transcribe(
            &self,
            _: &[f32],
            _: &murmur_core::asr::AsrRequest,
        ) -> Result<murmur_core::asr::AsrOutput, murmur_core::asr::AsrError> {
            Err(murmur_core::asr::AsrError::Transcribe("boom".into()))
        }
    }
    let cloud = Fake::new("cloud", "x");
    let m = Matrix {
        asr: vec![
            AsrSlot {
                name: "paid".into(),
                engine: Arc::new(MockAsr::new(SAID)),
                call_cost: Some(Arc::new(|| Some(0.0002))),
            },
            AsrSlot {
                name: "broken".into(),
                engine: Arc::new(Failing),
                call_cost: None,
            },
        ],
        cleanup: vec![
            slot("none", None, None),
            slot("or:fake", Some(cloud.clone()), None),
        ],
        prompts: vec![PromptVersion::V4c],
        session: session(),
        max_spend: 1.0,
    };
    let mut rows = Vec::new();
    let spend = m.run(&recs(), &mut |_| Ok(tone()), &mut |r| rows.push(r.clone()));
    assert!((spend.asr - 0.0004).abs() < 1e-9);
    let broken: Vec<&Row> = rows.iter().filter(|r| r.asr == "broken").collect();
    // One error row per recording, no cleanup calls for it.
    assert_eq!(broken.len(), 2, "{rows:#?}");
    assert!(broken
        .iter()
        .all(|r| r.error.as_deref().is_some_and(|e| e.contains("boom"))));
    assert_eq!(cloud.calls(), 2);
}
