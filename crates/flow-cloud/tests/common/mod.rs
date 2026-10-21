//! Shared helpers for the flow-cloud contract tests.
#![allow(dead_code)]

use std::time::Duration;

use flow_cloud::testing::{MockServer, Recorded, Reply};
use flow_cloud::{CloudAsr, CloudError};
use flow_core::asr::{AsrEngine, AsrRequest};
use flow_router::{BackendError, CloudProviderConfig, Failure, SecretString};

pub const KEY: &str = "sk-LEAKCHECK-contract-0123456789";

pub fn fixture(path: &str) -> serde_json::Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(path);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

/// A preset pointed at the mock server (`base` = what follows the host in
/// the real base URL, e.g. `/api/v1`).
pub fn provider(preset: &str, srv: &MockServer, base: &str) -> CloudProviderConfig {
    let mut p = flow_cloud::presets()
        .into_iter()
        .find(|p| p.preset == preset)
        .unwrap();
    p.id = format!("{preset}-1");
    p.base_url = srv.url(base);
    p.consent_host = "127.0.0.1".into();
    p.consent_at_ms = 1;
    if p.model.is_empty() {
        p.model = "whisper-1".into();
    }
    assert_eq!(p.validate(), Ok(()));
    p
}

pub fn asr(p: CloudProviderConfig) -> CloudAsr {
    CloudAsr::new(p, SecretString::new(KEY))
}

/// `ms` of a loud-ish tone (speech stand-in).
pub fn speech(ms: usize) -> Vec<f32> {
    (0..ms * 16)
        .map(|i| (i as f32 * 0.05).sin() * 0.25)
        .collect()
}

pub fn req(dict: &[&str], screen: &[&str], lang: Option<&str>) -> AsrRequest {
    AsrRequest {
        language: lang.map(str::to_string),
        dictionary_terms: dict.iter().map(|s| s.to_string()).collect(),
        screen_terms: screen.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

/// Transcribe 1 s through `asr`; the router-facing error if it failed.
pub fn run(asr: &CloudAsr, r: &AsrRequest) -> Result<String, BackendError> {
    match asr.transcribe(&speech(1000), r) {
        Ok(o) => Ok(o.text),
        Err(_) => Err(last_error(asr)),
    }
}

/// The typed error of the last failed call (what `CloudBackend::commit`
/// hands the router).
pub fn last_error(asr: &CloudAsr) -> BackendError {
    let info = asr.last_call().expect("a call was made");
    let kind: Failure =
        serde_json::from_value(serde_json::Value::String(info.error.clone().unwrap())).unwrap();
    BackendError::new(kind, info.message.unwrap())
}

/// The §3.2 status table, checked for one adapter. `ok_body` answers the
/// non-failing requests of multi-request adapters (AssemblyAI).
pub fn check_error_mapping(preset: &str, base: &str, fail_path: &'static str) {
    let cases: Vec<(Reply, Failure, usize)> = vec![
        (
            Reply::json(401, serde_json::json!({})),
            Failure::AuthFailed,
            1,
        ),
        (
            Reply::json(403, serde_json::json!({})),
            Failure::AuthFailed,
            1,
        ),
        (
            Reply::json(402, serde_json::json!({})),
            Failure::QuotaExceeded,
            1,
        ),
        (
            Reply::json(
                429,
                serde_json::json!({"error":{"code":"insufficient_quota"}}),
            ),
            Failure::QuotaExceeded,
            1,
        ),
        (
            Reply::json(413, serde_json::json!({})),
            Failure::ProviderError,
            1,
        ),
        (
            Reply::json(422, serde_json::json!({})),
            Failure::ProviderError,
            1,
        ),
        (
            Reply::json(400, serde_json::json!({})),
            Failure::ProviderError,
            1,
        ),
        (
            Reply::json(500, serde_json::json!({})),
            Failure::ProviderError,
            1,
        ),
        // 502/503/504: one retry.
        (
            Reply::json(503, serde_json::json!({})),
            Failure::ProviderError,
            2,
        ),
        // 429 without Retry-After / with a long one: no retry.
        (Reply::json(429, serde_json::json!({})), Failure::Busy, 1),
        (
            Reply::json(429, serde_json::json!({})).header("Retry-After", "30"),
            Failure::Busy,
            1,
        ),
        // 429 with a short Retry-After: one retry.
        (
            Reply::json(429, serde_json::json!({})).header("Retry-After", "0.1"),
            Failure::Busy,
            2,
        ),
        // 2xx with garbage.
        (
            Reply::raw(200, "<html>oops</html>"),
            Failure::ProviderError,
            1,
        ),
    ];
    for (reply, want, tries) in cases {
        let status = reply.status;
        let srv = fail_on(fail_path, reply);
        let a = asr(provider(preset, &srv, base));
        let e = run(&a, &req(&[], &[], None)).unwrap_err();
        assert_eq!(e.kind, want, "{preset}: HTTP {status}: {e}");
        let n = srv
            .requests()
            .iter()
            .filter(|r| r.path().ends_with(fail_path))
            .count();
        assert_eq!(n, tries, "{preset}: HTTP {status}: attempts");
        assert!(!e.message.contains(KEY) && !e.message.contains("oops"));
    }
}

/// Mock that fails on paths ending in `fail_path` and answers the rest
/// like a healthy AssemblyAI (upload / create) so multi-step adapters reach
/// the failing step.
pub fn fail_on(fail_path: &'static str, reply: Reply) -> MockServer {
    MockServer::start(move |r: &Recorded, _| {
        if r.path().ends_with(fail_path) {
            return reply.clone();
        }
        if r.path().ends_with("/v2/upload") {
            return Reply::json(200, fixture("assemblyai/upload.json"));
        }
        Reply::json(200, fixture("assemblyai/queued.json"))
    })
}

/// Connection refused → connect_failed; a slow server → timeout.
pub fn check_transport_errors(preset: &str, base: &str) {
    // Nothing listens on the port of a dropped server.
    let srv = MockServer::fixed(Reply::raw(200, ""));
    let mut p = provider(preset, &srv, base);
    drop(srv);
    std::thread::sleep(Duration::from_millis(20));
    let port = p
        .base_url
        .split(':')
        .nth(2)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_string();
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let free = dead.local_addr().unwrap().port();
    drop(dead);
    p.base_url = p.base_url.replace(&port, &free.to_string());
    let e = run(&asr(p), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::ConnectFailed, "{preset}: {e}");

    let slow = MockServer::fixed(Reply::raw(200, "{}").delayed(Duration::from_secs(3)));
    let mut p = provider(preset, &slow, base);
    p.timeout_base_ms = 1000;
    let a = asr(p);
    let t = std::time::Instant::now();
    let e = run(&a, &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::Timeout, "{preset}: {e}");
    assert!(
        t.elapsed() < Duration::from_millis(2900),
        "deadline honoured"
    );
}

pub fn assert_no_key_in(r: &Recorded, allowed_header: &str) {
    for (k, v) in &r.headers {
        if v.contains(KEY) {
            assert_eq!(k, allowed_header, "key only in the auth header");
        }
    }
    assert!(!r.target.contains(KEY), "key in URL");
    assert!(
        !String::from_utf8_lossy(&r.body).contains(KEY),
        "key in body"
    );
}

pub fn err_kind(e: &CloudError) -> Failure {
    e.failure()
}
