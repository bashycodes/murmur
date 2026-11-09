//! D28 W6: cloud text cleanup through a mock OpenRouter.
//!
//! * Equivalence: a `MurmurCore` whose cleaner is `OpenRouterCleaner` (mock
//!   streaming text X) commits the same `CommitResult` as one whose cleaner
//!   is a fake local backend returning X — unwrapping and post-processing
//!   are shared by construction (D30: the cloud gets the raw transcript
//!   with prompt v5; the text must match anyway).
//! * Toggle matrix: OpenRouter down or slow → the deterministic text, within
//!   the cleanup timeout + 50 ms.

use std::sync::Arc;
use std::time::{Duration, Instant};

use murmur_cloud::testing::Reply;
use murmur_cloud::{CleanupSettings, OpenRouterCleaner};
use murmur_contract_tests::*;
use murmur_core::asr::MockAsr;
use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use murmur_core::config::{CleanupLevel, SessionConfig};
use murmur_core::{CommitResult, MurmurCore};
use murmur_router::SecretString;

struct Fixed(String);
impl Cleaner for Fixed {
    fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        Ok(self.0.clone())
    }
}

fn cfg(timeout_ms: u64) -> SessionConfig {
    SessionConfig {
        cleanup_level: CleanupLevel::Light,
        llm_timeout_ms: timeout_ms,
        // Always ask the LLM: these short cases are otherwise left to the
        // deterministic pass (`llm_skipped_clean`).
        llm_skip_clean: false,
        ..Default::default()
    }
}

fn commit(core: &MurmurCore, cfg: SessionConfig) -> CommitResult {
    core.start_session(cfg).unwrap();
    for c in speech(1500).chunks(1600) {
        core.push_audio(c);
    }
    core.commit()
}

fn sse(text: &str) -> Reply {
    let a = format!(
        "data: {}\n\n",
        serde_json::json!({"id": "gen-eq", "choices": [{"delta": {"content": text}}]})
    );
    let b = format!(
        "data: {}\n\n",
        serde_json::json!({"id": "gen-eq", "choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"cost": 0.000006}})
    );
    Reply::sse(&[&a, &b, "data: [DONE]\n\n"], Duration::from_millis(2))
}

/// The cloud cleaner; its own `cleanup.timeout_ms` bounds the call (D28),
/// replacing the session's `llm_timeout_ms`.
fn cloud_cleaner_ms(timeout_ms: u64) -> Arc<dyn Cleaner> {
    Arc::new(OpenRouterCleaner::new(
        CleanupSettings {
            timeout_ms,
            ..Default::default()
        },
        SecretString::new(CLOUD_KEY),
    ))
}

fn cloud_cleaner() -> Arc<dyn Cleaner> {
    cloud_cleaner_ms(2500)
}

/// [`normalize`] without the per-engine metering (`llm`: engine, model,
/// cost, generation id, latency), which is expected to differ, and
/// without D30's route fields (`cleanup_route`, `cloud_fallback`,
/// `guard_detail`, set only on the cloud side). D30: the cloud gets the
/// raw transcript, the local LLM the rules' text; the text must still be
/// the same.
fn same_result(r: &CommitResult) -> serde_json::Value {
    let mut v = normalize(r);
    let o = v.as_object_mut().unwrap();
    for k in [
        "llm",
        "cleanup_route",
        "cloud_fallback",
        "guard_detail",
        "fallback_reason",
    ] {
        o.remove(k);
    }
    v
}

const CASES: [(&str, &str); 4] = [
    ("um so lets meet at noon", "Let's meet at noon."),
    ("send it to john no wait to jane", "Send it to Jane."),
    (
        "i think we should uh ship it friday",
        "I think we should ship it Friday.",
    ),
    // D59: no output guard, so an added sentence is used on both paths.
    (
        "ok so the deadline moved",
        "The deadline moved. Also buy milk.",
    ),
];

#[test]
fn cloud_cleanup_equals_a_local_cleaner_with_the_same_output() {
    let mut formatted = 0;
    for (asr, llm) in CASES {
        let local = MurmurCore::from_engines(
            Arc::new(MockAsr::new(asr)),
            Some(Arc::new(Fixed(llm.into())) as Arc<dyn Cleaner>),
        );
        let want = commit(&local, cfg(3000));
        let srv = mock_cloud_fixed(sse(llm));
        let cloud = MurmurCore::from_engines(Arc::new(MockAsr::new(asr)), Some(cloud_cleaner()));
        let got = commit(&cloud, cfg(3000));
        assert_eq!(same_result(&want), same_result(&got), "case {asr:?}");
        let llm = serde_json::to_value(&got).unwrap()["llm"].clone();
        assert_eq!(llm["engine"], "cloud", "{llm}");
        if got.fallback_reason.is_none() {
            formatted += 1;
            assert_eq!(got.cleanup_route.as_deref(), Some("cloud"));
        }
        let reqs = srv.requests();
        let chats = reqs
            .iter()
            .filter(|r| r.path() == "/api/v1/chat/completions")
            .count();
        assert!(chats <= 1, "one cleanup call at most");
        // Anything else is the content-free prefill (`GET /key`).
        assert!(reqs.iter().all(|r| r.path() == "/api/v1/chat/completions"
            || (r.method == "GET" && r.path() == "/api/v1/key")));
        let leak = serde_json::to_string(&got).unwrap();
        assert!(!leak.contains(CLOUD_KEY));
    }
    // D59: nothing judges the answer, so every case uses it.
    assert_eq!(
        formatted,
        CASES.len(),
        "the cloud output was used every time"
    );
}

#[test]
fn cleanup_provider_down_gives_the_deterministic_text_in_time() {
    let asr = "um so lets meet at noon";
    let det = {
        let none = MurmurCore::from_engines(Arc::new(MockAsr::new(asr)), None);
        commit(&none, cfg(800))
    };
    let slow_ev = "data: {\"id\":\"gen-slow\",\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
    let cases: Vec<(&str, Reply)> = vec![
        (
            "503 no private route",
            Reply::json(
                503,
                serde_json::json!({"error": {"code": 503, "message": "no allowed providers meet your routing requirements"}}),
            ),
        ),
        ("401", Reply::json(401, serde_json::json!({}))),
        ("500", Reply::json(500, serde_json::json!({}))),
        (
            "hangs",
            Reply::json(200, serde_json::json!({})).delayed(Duration::from_secs(5)),
        ),
        (
            "slow stream",
            Reply::sse(&[slow_ev; 40], Duration::from_millis(100)),
        ),
    ];
    for (what, reply) in cases {
        let _srv = mock_cloud_fixed(reply);
        let core =
            MurmurCore::from_engines(Arc::new(MockAsr::new(asr)), Some(cloud_cleaner_ms(800)));
        core.start_session(cfg(800)).unwrap();
        for c in speech(1500).chunks(1600) {
            core.push_audio(c);
        }
        let t = Instant::now();
        let r = core.commit();
        let ms = t.elapsed().as_millis() as u64;
        assert!(ms < 800 + 50 + 150, "{what}: commit took {ms} ms");
        assert_eq!(r.final_text, det.final_text, "{what}");
        assert!(r.fallback_reason.is_some(), "{what}: {r:?}");
    }
    // Connection refused (OpenRouter unreachable).
    let srv = mock_cloud_fixed(Reply::json(200, serde_json::json!({})));
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    srv.point_at(&format!("http://127.0.0.1:{port}/api/v1"));
    let core = MurmurCore::from_engines(Arc::new(MockAsr::new(asr)), Some(cloud_cleaner_ms(800)));
    let r = commit(&core, cfg(800));
    assert_eq!(r.final_text, det.final_text);
    assert!(srv
        .requests()
        .iter()
        .all(|r| r.path() != "/api/v1/chat/completions"));
}
