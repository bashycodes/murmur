//! Shared helpers for the flow-cloud contract tests: a mock OpenRouter
//! (`testing::MockServer` + the base-URL override) and its fixtures.
#![allow(dead_code)]

use std::time::Duration;

use flow_cloud::catalog::ModelInfo;
use flow_cloud::testing::{override_base_url, BaseOverride, MockServer, Recorded, Reply};
use flow_cloud::{CloudAsr, CloudError, SttPath, SttSettings};
use flow_core::asr::{AsrEngine, AsrRequest};
use flow_router::{BackendError, Failure, SecretString};

pub const KEY: &str = "sk-or-LEAKCHECK-contract-0123456789";

pub fn fixture_path(path: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/openrouter")
        .join(path)
}

pub fn fixture_text(path: &str) -> String {
    std::fs::read_to_string(fixture_path(path)).unwrap()
}

pub fn fixture(path: &str) -> serde_json::Value {
    serde_json::from_str(&fixture_text(path)).unwrap()
}

/// A mock OpenRouter; every `Api` made while it lives talks to it.
pub struct Mock {
    pub srv: MockServer,
    _base: BaseOverride,
}

impl std::ops::Deref for Mock {
    type Target = MockServer;
    fn deref(&self) -> &MockServer {
        &self.srv
    }
}

pub fn mock(handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static) -> Mock {
    let srv = MockServer::start(handler);
    let base = override_base_url(&srv.url("/api/v1"));
    Mock { srv, _base: base }
}

pub fn mock_fixed(reply: Reply) -> Mock {
    mock(move |_, _| reply.clone())
}

pub fn mock_seq(replies: Vec<Reply>) -> Mock {
    mock(move |_, i| replies[i.min(replies.len() - 1)].clone())
}

pub fn settings(model: &str) -> SttSettings {
    SttSettings {
        model: model.into(),
        ..Default::default()
    }
}

/// A catalogue entry for a path-A or path-B model.
pub fn info(id: &str, path: SttPath) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        stt_path: Some(path),
        price_unit_ok: true,
        ..Default::default()
    }
}

pub fn asr(s: SttSettings) -> CloudAsr {
    CloudAsr::new(s, SecretString::new(KEY))
}

/// The default model (whisper turbo, path A) on the mock.
pub fn whisper() -> CloudAsr {
    asr(settings(flow_cloud::DEFAULT_STT_MODEL))
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
/// hands the router). `no_private_route` maps to `provider_error` until
/// flow-router has the variant.
pub fn last_error(asr: &CloudAsr) -> BackendError {
    let info = asr.last_call().expect("a call was made");
    let code = info.error.clone().unwrap();
    let kind: Failure = serde_json::from_value(serde_json::Value::String(code.clone()))
        .unwrap_or(Failure::ProviderError);
    BackendError::new(kind, info.message.unwrap())
}

pub fn last_code(asr: &CloudAsr) -> String {
    asr.last_call().unwrap().error.unwrap()
}

/// Connection refused → connect_failed; a slow server → timeout.
pub fn check_transport_errors() {
    {
        // Nothing listens on the port of a dropped server.
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let free = dead.local_addr().unwrap().port();
        drop(dead);
        let _g = override_base_url(&format!("http://127.0.0.1:{free}/api/v1"));
        let e = run(&whisper(), &req(&[], &[], None)).unwrap_err();
        assert_eq!(e.kind, Failure::ConnectFailed, "{e}");
    }
    let _slow = mock_fixed(Reply::raw(200, "{}").delayed(Duration::from_secs(3)));
    let mut s = settings(flow_cloud::DEFAULT_STT_MODEL);
    s.timeout_base_ms = 1000;
    let a = asr(s);
    let t = std::time::Instant::now();
    let e = run(&a, &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::Timeout, "{e}");
    assert!(
        t.elapsed() < Duration::from_millis(2900),
        "deadline honoured"
    );
}

/// The key only in `Authorization`, never in URL/body/other headers, and
/// no attribution headers.
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
    for h in ["http-referer", "referer", "x-openrouter-title", "x-title"] {
        assert!(r.header(h).is_none(), "{h} sent");
    }
}

pub fn err_kind(e: &CloudError) -> Failure {
    e.failure()
}
