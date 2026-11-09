//! `OpenRouterCleaner` wire contract (D28 §2.5, §5.1): streamed chat
//! completions, privacy + latency routing + `max_price`, SSE parsing,
//! cancel within 50 ms, usage/cost capture, error codes, and the
//! flow-core `run_cleanup` integration.

mod common;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use flow_cloud::testing::Reply;
use flow_cloud::{Catalog, CleanupSettings, OpenRouterCleaner, DEFAULT_CLEANUP_MODEL};
use flow_core::asr::AbortFlag;
use flow_core::cleanup::{
    build_prompt, run_cleanup, CleanOutcome, CleanPrompt, Cleaner, FallbackReason,
};
use flow_core::config::{CleanupLevel, FieldContext, Style};
use flow_core::dictionary::Dictionary;
use flow_router::SecretString;

const ASR: &str = "lets meet at noon";

fn prompt() -> CleanPrompt {
    build_prompt(
        ASR,
        CleanupLevel::Light,
        Style::default(),
        &FieldContext::default(),
        &Dictionary::default(),
    )
}

fn cleaner() -> OpenRouterCleaner {
    OpenRouterCleaner::new(CleanupSettings::default(), SecretString::new(KEY))
}

fn sse_ok() -> Reply {
    let events: Vec<String> = fixture_text("chat_stream_ok.sse")
        .split_inclusive("\n\n")
        .map(str::to_string)
        .collect();
    let refs: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply::sse(&refs, Duration::from_millis(5)).header("X-Generation-Id", "gen-stream-1")
}

#[test]
fn request_shape_and_streamed_result() {
    let srv = mock_fixed(sse_ok());
    let c = cleaner();
    let out = c
        .clean(&prompt(), Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(out, "Let's meet at noon.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/api/v1/chat/completions");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert_no_key_in(r, "authorization");
    let j = r.json();
    assert_eq!(j["model"], DEFAULT_CLEANUP_MODEL);
    assert_eq!(j["stream"], true);
    assert_eq!(j["temperature"], 0);
    assert_eq!(j["seed"], 0);
    assert_eq!(j["max_completion_tokens"], 32 + 2 * 5, "2 × tokens + 32");
    // llama-3.3 lists no reasoning parameter: none sent.
    assert!(j.get("reasoning").is_none());
    let msgs = j["messages"].as_array().unwrap();
    let p0 = prompt();
    let want = p0.messages();
    assert_eq!(msgs.len(), want.len());
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs.last().unwrap()["content"], want.last().unwrap().1);
    let p = &j["provider"];
    assert_eq!(p["zdr"], true);
    assert_eq!(p["data_collection"], "deny");
    assert_eq!(p["sort"], "latency");
    assert_eq!(p["allow_fallbacks"], true);
    let mp = &p["max_price"];
    assert!(mp["prompt"].as_f64().unwrap() > 0.0 && mp["completion"].as_f64().unwrap() > 0.0);
    for k in ["user", "session_id"] {
        assert!(j.get(k).is_none(), "{k} sent");
    }
    let call = c.last_call().unwrap();
    assert_eq!(call.cost_usd, Some(0.0000061));
    assert!(!call.cost_estimated);
    assert_eq!(call.generation_id.as_deref(), Some("gen-stream-1"));
    assert!(call.ttft_ms.is_some() && call.latency_ms.is_some());
    assert_eq!(call.finish_reason.as_deref(), Some("stop"));
    let info: serde_json::Value = serde_json::from_str(&c.info_json().unwrap()).unwrap();
    assert_eq!(info["engine"], "cloud");
    assert_eq!(info["last_generation_id"], "gen-stream-1");
    assert!(!c.info_json().unwrap().contains(KEY));
}

#[test]
fn reasoning_models_get_the_lowest_effort() {
    let srv = mock_fixed(sse_ok());
    let cat = Catalog::baked();
    let s = CleanupSettings {
        model: "openai/gpt-oss-20b".into(),
        ..Default::default()
    };
    let m = cat.get(&s.model).cloned();
    let want = m.as_ref().and_then(|m| m.reasoning_effort());
    let c = OpenRouterCleaner::new(s, SecretString::new(KEY));
    c.clean(&prompt(), Instant::now() + Duration::from_secs(5))
        .unwrap();
    let j = srv.requests()[0].json();
    assert_eq!(j["reasoning"]["effort"].as_str(), want, "{j}");
    assert_ne!(want, None);
}

#[test]
fn settings_change_routing() {
    let srv = mock_fixed(sse_ok());
    let s = CleanupSettings {
        sort: String::new(),
        allow_fallbacks: false,
        privacy: flow_cloud::Privacy {
            zdr: false,
            data_collection: "allow".into(),
        },
        model: "acme/unknown-model".into(),
        ..Default::default()
    };
    let c = OpenRouterCleaner::new(s, SecretString::new(KEY));
    c.clean(&prompt(), Instant::now() + Duration::from_secs(5))
        .unwrap();
    let p = &srv.requests()[0].json()["provider"];
    assert_eq!(
        p,
        &serde_json::json!({"allow_fallbacks": false}),
        "unknown model: no max_price"
    );
}

#[test]
fn midstream_error_is_a_provider_error() {
    let events: Vec<String> = fixture_text("chat_stream_midstream_error.sse")
        .split_inclusive("\n\n")
        .map(str::to_string)
        .collect();
    let refs: Vec<&str> = events.iter().map(String::as_str).collect();
    let _srv = mock_fixed(Reply::sse(&refs, Duration::from_millis(5)));
    let c = cleaner();
    let e = c
        .clean(&prompt(), Instant::now() + Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(e.to_string(), "provider_error");
    assert_eq!(
        c.last_call().unwrap().generation_id.as_deref(),
        Some("gen-stream-2")
    );
}

#[test]
fn cancel_aborts_the_stream_within_50_ms() {
    // A slow stream: one token every 100 ms for 3 s.
    let ev = "data: {\"id\":\"gen-slow\",\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
    let srv = mock_fixed(Reply::sse(&[ev; 30], Duration::from_millis(100)));
    let c = Arc::new(cleaner().with_cost_lookup_delay(Duration::from_millis(10)));
    let flag = Arc::new(AtomicBool::new(false));
    let abort = AbortFlag(Some(flag.clone()));
    let c2 = c.clone();
    let h = std::thread::spawn(move || {
        let r = c2.clean_cancellable(&prompt(), Instant::now() + Duration::from_secs(10), &abort);
        (r, Instant::now())
    });
    std::thread::sleep(Duration::from_millis(350));
    let t = Instant::now();
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    let (r, done) = h.join().unwrap();
    assert!(
        done.duration_since(t) < Duration::from_millis(50),
        "{:?}",
        done.duration_since(t)
    );
    assert_eq!(r.unwrap_err().to_string(), "timeout");
    // The connection is dropped: the server's next write fails.
    for _ in 0..100 {
        if srv.aborted_streams() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(srv.aborted_streams(), 1, "stream aborted (billing stops)");
    let call = c.last_call().unwrap();
    assert!(call.cost_estimated, "no usage chunk: estimate");
    assert_eq!(call.generation_id.as_deref(), Some("gen-slow"));
}

#[test]
fn run_cleanup_timeout_returns_deterministic_text_in_time() {
    let ev = "data: {\"id\":\"gen-slow\",\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
    let srv = mock_fixed(Reply::sse(&[ev; 30], Duration::from_millis(100)));
    let c: Arc<dyn Cleaner> = Arc::new(cleaner());
    let t = Instant::now();
    let out = run_cleanup(c, prompt(), ASR, Duration::from_millis(400));
    assert!(
        t.elapsed() < Duration::from_millis(450),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(out, CleanOutcome::Fallback(FallbackReason::Timeout));
    for _ in 0..100 {
        if srv.aborted_streams() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(srv.aborted_streams(), 1);
}

#[test]
fn run_cleanup_uses_the_streamed_answer() {
    let _srv = mock_fixed(sse_ok());
    let c: Arc<dyn Cleaner> = Arc::new(cleaner());
    let out = run_cleanup(c, prompt(), ASR, Duration::from_secs(3));
    assert_eq!(out, CleanOutcome::Formatted("Let's meet at noon.".into()));
}

#[test]
fn non_streaming_json_answer_is_accepted() {
    let _srv = mock_fixed(Reply::json(
        200,
        serde_json::json!({"id": "gen-j", "choices": [{"finish_reason": "stop", "message": {"content": "Hi."}}], "usage": {"cost": 0.00001}}),
    ));
    let c = cleaner();
    assert_eq!(
        c.clean(&prompt(), Instant::now() + Duration::from_secs(3))
            .unwrap(),
        "Hi."
    );
    assert_eq!(c.last_call().unwrap().cost_usd, Some(0.00001));
}

#[test]
fn error_codes() {
    for (reply, want) in [
        (Reply::json(401, fixture("err_401.json")), "auth_failed"),
        (
            Reply::json(402, fixture("err_402_credits.json")),
            "quota_exceeded",
        ),
        (
            Reply::json(403, fixture("err_403_key_limit.json")),
            "quota_exceeded",
        ),
        (
            Reply::json(503, fixture("err_503_routing.json")),
            "no_private_route",
        ),
        (Reply::json(502, fixture("err_502.json")), "provider_error"),
        (Reply::json(429, fixture("err_429.json")), "busy"),
    ] {
        let _srv = mock_fixed(reply);
        let c = cleaner();
        let e = c
            .clean(&prompt(), Instant::now() + Duration::from_secs(3))
            .unwrap_err();
        assert_eq!(e.to_string(), want);
        assert_eq!(c.last_call().unwrap().error, Some(want));
    }
}

#[test]
fn prefill_only_checks_the_key_at_most_once_a_minute() {
    let srv = mock(|r, _| {
        assert_eq!(r.path(), "/api/v1/key", "prefill sends no content");
        Reply::json(200, fixture("key_ok.json"))
    });
    let c = cleaner();
    c.prefill(&prompt());
    c.prefill(&prompt());
    for _ in 0..100 {
        if c.key_info().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(srv.count(), 1);
    assert_eq!(c.key_info().unwrap().limit_remaining, Some(7.25));
}

#[test]
fn estimate_and_default_scope() {
    let c = cleaner();
    let e = c.estimate_cost_usd(&prompt()).unwrap();
    assert!(e > 0.0 && e < 0.01, "{e}");
    assert_eq!(c.scope(), flow_cloud::CleanupScope::default());
    let unknown = OpenRouterCleaner::new(
        CleanupSettings {
            model: "acme/nope".into(),
            ..Default::default()
        },
        SecretString::new(KEY),
    );
    assert_eq!(unknown.estimate_cost_usd(&prompt()), None);
}

#[test]
fn deferred_cost_after_abandoned_stream() {
    let ev = "data: {\"id\":\"gen-stream-1\",\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
    let _srv = mock(move |r, _| {
        if r.path().ends_with("/generation") {
            return Reply::json(200, fixture("generation_ok.json"));
        }
        Reply::sse(&[ev; 30], Duration::from_millis(50))
    });
    let got = Arc::new(Mutex::new(Vec::new()));
    let g2 = got.clone();
    let c = cleaner()
        .with_cost_sink(Arc::new(move |d| g2.lock().unwrap().push(d)))
        .with_cost_lookup_delay(Duration::from_millis(10));
    let _ = c.clean(&prompt(), Instant::now() + Duration::from_millis(200));
    for _ in 0..200 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let d = got.lock().unwrap()[0].clone();
    assert_eq!(d.actual_usd, Some(0.0000058));
    assert_eq!(d.generation_id, "gen-stream-1");
}

#[test]
fn metered_usage_and_fast_failures_for_the_router() {
    let _srv = mock_fixed(sse_ok());
    let c = cleaner();
    assert_eq!(c.engine(), "cloud");
    assert_eq!(c.model().as_deref(), Some(DEFAULT_CLEANUP_MODEL));
    assert_eq!(c.timeout_ms(), Some(2500));
    assert_eq!(c.prompt_scope(), flow_core::cleanup::PromptScope::default());
    let (r, u) = c.clean_metered(
        &prompt(),
        Instant::now() + Duration::from_secs(3),
        &AbortFlag::default(),
    );
    assert_eq!(r.unwrap(), "Let's meet at noon.");
    assert_eq!(u.engine.as_deref(), Some("cloud"));
    assert_eq!(u.cost_usd, Some(flow_core::Usd(0.0000061)));
    assert_eq!(u.generation_id.as_deref(), Some("gen-stream-1"));
    assert!(u.latency_ms.is_some() && u.ttft_ms.is_some());
    drop(_srv);
    for (reply, fast) in [
        (Reply::json(401, fixture("err_401.json")), true),
        (Reply::json(402, fixture("err_402_credits.json")), true),
        (Reply::json(503, fixture("err_503_routing.json")), true),
        (Reply::json(429, fixture("err_429.json")), true),
        (Reply::json(502, fixture("err_502.json")), false),
        (Reply::raw(200, "garbage"), false),
    ] {
        let _srv = mock_fixed(reply);
        let (r, _) = cleaner().clean_metered(
            &prompt(),
            Instant::now() + Duration::from_secs(3),
            &AbortFlag::default(),
        );
        let e = r.unwrap_err();
        assert_eq!(
            matches!(e, flow_core::cleanup::CleanError::Unavailable(_)),
            fast,
            "{e:?}"
        );
    }
}
