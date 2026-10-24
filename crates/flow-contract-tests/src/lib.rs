//! Harness for the hybrid contract suites (docs/hybrid-architecture.md §4):
//! an in-process `flow-server` on 127.0.0.1:0, local `FlowCore` backends,
//! routers wired to the real `flow-remote` client, session fixtures and
//! result normalisation.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use flow_core::config::{AsrBackend, FieldContext, ModelConfig, SessionConfig};
use flow_core::{CommitResult, FlowCore};
use flow_router::{
    Backend, CloudConfig, CloudProviderConfig, Route, Router, RouterConfig, SecretString,
    ServerConfig,
};
use serde::Deserialize;
use serde_json::Value;

pub use flow_server::Config as ServerFileConfig;

pub const TOKEN: &str = "contract-token-0123456789abcdef";
/// 100 ms frames, like the shells.
pub const FRAME: usize = 1600;

/// An in-process flow-server with its own runtime.
pub struct TestServer {
    handle: flow_server::ServerHandle,
    rt: Option<tokio::runtime::Runtime>,
}

impl TestServer {
    pub fn addr(&self) -> SocketAddr {
        self.handle.addr()
    }
    /// Drop every connection and stop listening (a crash / `systemctl stop`).
    pub fn kill(&self) {
        self.handle.kill();
        std::thread::sleep(Duration::from_millis(30));
    }
    /// Commits are never answered.
    pub fn stall_commits(&self, on: bool) {
        self.handle
            .hooks()
            .stall_commits
            .store(on, Ordering::SeqCst);
    }
    /// Every worker looks leased: new sessions are queued.
    pub fn set_busy(&self, on: bool) {
        self.handle.set_busy(on);
    }
    /// TCP connections accepted so far.
    pub fn connections(&self) -> u64 {
        self.handle.connections()
    }
    pub fn cancels(&self) -> u64 {
        self.handle.cancels()
    }
    pub fn sessions(&self) -> u64 {
        self.handle.sessions()
    }
    /// Workers currently leased.
    pub fn busy(&self) -> u32 {
        self.handle.pool().busy()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.kill();
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

pub fn mock_model(text: &str) -> ModelConfig {
    ModelConfig {
        asr_backend: AsrBackend::Mock { text: text.into() },
        ..Default::default()
    }
}

pub fn server_cfg(model: ModelConfig) -> ServerFileConfig {
    ServerFileConfig {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model,
        ..Default::default()
    }
}

pub fn spawn_server(cfg: ServerFileConfig) -> TestServer {
    spawn_server_with(cfg, None)
}

/// A server whose single worker's final decode takes `delay` (MockAsr).
pub fn spawn_slow_mock_server(text: &str, delay: Duration) -> TestServer {
    let cfg = server_cfg(mock_model(text));
    let core = FlowCore::from_engines(
        Arc::new(flow_core::asr::MockAsr::new(text).with_delay(delay)),
        None,
    );
    let pool = flow_server::Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    spawn_server_with(cfg, Some(Arc::new(pool)))
}

/// A one-worker server around a caller-built core (custom test engines).
pub fn spawn_core_server(core: FlowCore) -> TestServer {
    let cfg = server_cfg(mock_model("unused"));
    let pool = flow_server::Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    spawn_server_with(cfg, Some(Arc::new(pool)))
}

fn spawn_server_with(cfg: ServerFileConfig, pool: Option<Arc<flow_server::Pool>>) -> TestServer {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        match pool {
            Some(p) => flow_server::serve_with_pool(cfg, p, vec![l]).await,
            None => flow_server::serve(cfg, vec![l]).await,
        }
        .expect("server starts")
    });
    TestServer {
        handle,
        rt: Some(rt),
    }
}

pub fn spawn_mock_server(text: &str) -> TestServer {
    spawn_server(server_cfg(mock_model(text)))
}

pub fn local_backend(model: &ModelConfig) -> Arc<FlowCore> {
    Arc::new(FlowCore::load_models(model).expect("local model loads"))
}

pub fn server_config(addr: SocketAddr, token: &str) -> ServerConfig {
    ServerConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        token: token.into(),
        ..Default::default()
    }
}

pub fn router_config(addr: SocketAddr, allow_server: bool, allow_client: bool) -> RouterConfig {
    RouterConfig {
        allow_server,
        allow_client,
        server: server_config(addr, TOKEN),
        ..Default::default()
    }
}

/// `Router{allow_server only}` → flow-remote → the server at `addr`.
pub fn remote_router(addr: SocketAddr) -> Router {
    Router::new(
        router_config(addr, true, false),
        None,
        Some(flow_remote::factory("flow-contract-tests")),
    )
}

/// [`remote_router`] for the real-model equivalence tests. They compare
/// results, not latency, so the commit deadline is generous: the product
/// default (`commit_timeout_base_ms` 3 s + 0.3 x audio + LLM budget, ~7.8 s
/// for jfk.wav) is a fallback trigger sized for a dedicated server, and a
/// debug-build whisper/Parakeet run sharing a loaded CPU with the rest of the
/// test suite can exceed it (seen: "no result before the deadline"). The
/// product deadline itself is exercised by `toggle_matrix` (stalled commit).
pub fn remote_router_real_model(addr: SocketAddr) -> Router {
    let mut cfg = router_config(addr, true, false);
    cfg.server.commit_timeout_base_ms = REAL_MODEL_COMMIT_TIMEOUT_MS;
    Router::new(cfg, None, Some(flow_remote::factory("flow-contract-tests")))
}

/// Commit deadline base for [`remote_router_real_model`].
pub const REAL_MODEL_COMMIT_TIMEOUT_MS: u64 = 120_000;

pub fn hybrid_router(cfg: RouterConfig, local: Option<Arc<FlowCore>>) -> Router {
    Router::new(
        cfg,
        local.map(|l| l as Arc<dyn Backend>),
        Some(flow_remote::factory("flow-contract-tests")),
    )
}

// ---- cloud STT (D18) -----------------------------------------------------

/// Sentinel API key: must never appear in any output (C3).
pub const CLOUD_KEY: &str = "sk-LEAKCHECK-7f3a9c0b1e2d4f6a8b5c";

/// The one cloud provider entry id (D28).
pub const CLOUD_ID: &str = "openrouter";

/// A mock OpenRouter. While it lives every flow-cloud `Api` talks to it
/// (`flow_cloud::testing::override_base_url`; serialised process-wide):
/// configs stay production-shaped, the mock is never reachable via config.
pub struct MockCloud {
    pub srv: flow_cloud::testing::MockServer,
    _base: flow_cloud::testing::BaseOverride,
}

impl std::ops::Deref for MockCloud {
    type Target = flow_cloud::testing::MockServer;
    fn deref(&self) -> &Self::Target {
        &self.srv
    }
}

impl MockCloud {
    /// Send flow-cloud's requests elsewhere (e.g. a closed port).
    pub fn point_at(&self, base: &str) {
        self._base.set(base);
    }

    /// The decoded `input_audio` of transcription request `i`.
    pub fn uploaded_audio(&self, i: usize) -> Vec<u8> {
        use base64::Engine;
        let j: Value = serde_json::from_slice(&self.srv.requests()[i].body).unwrap();
        base64::engine::general_purpose::STANDARD
            .decode(j["input_audio"]["data"].as_str().unwrap())
            .unwrap()
    }
}

pub fn mock_cloud(
    handler: impl Fn(&flow_cloud::testing::Recorded, usize) -> flow_cloud::testing::Reply
        + Send
        + Sync
        + 'static,
) -> MockCloud {
    let srv = flow_cloud::testing::MockServer::start(handler);
    let base = flow_cloud::testing::override_base_url(&srv.url("/api/v1"));
    MockCloud { srv, _base: base }
}

pub fn mock_cloud_fixed(reply: flow_cloud::testing::Reply) -> MockCloud {
    mock_cloud(move |_, _| reply.clone())
}

/// A mock OpenRouter transcription endpoint answering `text`; the
/// language echoes the request's, else `en` (what `MockAsr` reports).
pub fn mock_provider(text: &str) -> MockCloud {
    let text = text.to_string();
    mock_cloud(move |r, _| {
        let lang = serde_json::from_slice::<Value>(&r.body)
            .ok()
            .and_then(|j| {
                j.get("language")
                    .and_then(|l| l.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "en".into());
        flow_cloud::testing::Reply::json(200, serde_json::json!({"text": text, "language": lang}))
    })
}

/// The OpenRouter entry, consented for openrouter.ai.
pub fn cloud_provider() -> CloudProviderConfig {
    let mut p = flow_cloud::presets().remove(0);
    p.id = CLOUD_ID.into();
    p.consent_host = flow_cloud::HOST.into();
    p.consent_at_ms = 1_792_540_800_000;
    p
}

/// `cfg` with the cloud enabled for `provider` and routes `order`.
pub fn with_cloud(
    mut cfg: RouterConfig,
    provider: CloudProviderConfig,
    order: &[Route],
) -> RouterConfig {
    cfg.allow_cloud = true;
    cfg.order = order.to_vec();
    cfg.cloud = CloudConfig {
        active: provider.id.clone(),
        providers: vec![provider],
        monthly_budget_usd: None,
    };
    cfg
}

/// Hybrid router (flow-remote) + flow-cloud attached + the sentinel key.
pub fn cloud_router(cfg: RouterConfig, local: Option<Arc<FlowCore>>) -> Router {
    cfg.validate().expect("valid cloud config");
    let id = cfg.cloud.active.clone();
    let r = flow_cloud::attach(hybrid_router(cfg, local), None);
    r.set_cloud_key(&id, Some(SecretString::new(CLOUD_KEY)));
    r
}

/// Everything except `timings` and `route`, as JSON.
pub fn normalize(r: &CommitResult) -> Value {
    let mut v = serde_json::to_value(r).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("timings");
    o.remove("route");
    v
}

// ---- audio ------------------------------------------------------------------

pub fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

pub fn silence(ms: usize) -> Vec<i16> {
    vec![0; ms * 16]
}

/// Low-level white noise (room noise: below the VAD's speech bar).
pub fn noise(ms: usize, amp: i16, seed: u32) -> Vec<i16> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..ms * 16)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            ((x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp as f32) as i16
        })
        .collect()
}

/// Candidate model directories: `FLOW_MODELS_DIR`, the repo's `models/`,
/// and (for git worktrees under `.claude/worktrees/`) the main checkout's.
pub fn models_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("FLOW_MODELS_DIR") {
        v.push(PathBuf::from(d));
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    v.push(here.join("../../models"));
    v.push(here.join("../../../../../models"));
    v
}

pub fn find_model(name: &str) -> Option<PathBuf> {
    models_dirs()
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.exists())
}

/// `"speech:MS" | "silence:MS" | "noise:MS" | "noise+speech:MS" | "<file>.wav"`.
pub fn audio(spec: &str) -> Vec<i16> {
    if let Some((kind, ms)) = spec.split_once(':') {
        let ms: usize = ms.parse().expect("audio ms");
        return match kind {
            "speech" => speech(ms),
            "silence" => silence(ms),
            "noise" => noise(ms, 60, 7),
            "noise+speech" => {
                let mut v = noise(ms / 2, 60, 7);
                v.extend(speech(ms - ms / 2));
                v
            }
            other => panic!("unknown audio kind {other}"),
        };
    }
    let p = find_model(spec).unwrap_or_else(|| panic!("{spec} not found"));
    flow_core::audio::read_wav_16k_mono(&p).unwrap()
}

// ---- session fixtures -----------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CtxAt {
    pub at_ms: u64,
    pub ctx: FieldContext,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Learn {
    pub pasted: String,
    pub edited: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Case {
    pub name: String,
    pub asr: Value,
    pub audio: String,
    pub session_config: SessionConfig,
    #[serde(default)]
    pub contexts: Vec<CtxAt>,
    pub learn: Option<Learn>,
}

impl Case {
    pub fn mock_text(&self) -> Option<&str> {
        self.asr.get("mock").and_then(Value::as_str)
    }
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions")
}

pub fn load_cases() -> Vec<Case> {
    let mut v: Vec<Case> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| load_case(&p))
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn load_case(p: &Path) -> Case {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The session API common to `FlowCore` and `Router`.
pub trait Dictation {
    fn d_start(&self, cfg: SessionConfig);
    fn d_push(&self, pcm: &[i16]);
    fn d_context(&self, ctx: FieldContext);
    fn d_commit(&self) -> CommitResult;
}

impl Dictation for FlowCore {
    fn d_start(&self, cfg: SessionConfig) {
        self.start_session(cfg).unwrap();
    }
    fn d_push(&self, pcm: &[i16]) {
        self.push_audio(pcm);
    }
    fn d_context(&self, ctx: FieldContext) {
        self.set_context(ctx);
    }
    fn d_commit(&self) -> CommitResult {
        self.commit()
    }
}

impl Dictation for Router {
    fn d_start(&self, cfg: SessionConfig) {
        self.start_session(cfg).unwrap();
    }
    fn d_push(&self, pcm: &[i16]) {
        self.push_audio(pcm);
    }
    fn d_context(&self, ctx: FieldContext) {
        self.set_context(ctx);
    }
    fn d_commit(&self) -> CommitResult {
        self.commit()
    }
}

/// Run one case: contexts are set once `at_ms` of audio has been pushed
/// (`at_ms` >= the audio length = after all audio, before commit).
pub fn run_case(d: &dyn Dictation, case: &Case, pcm: &[i16]) -> CommitResult {
    d.d_start(case.session_config.clone());
    let mut ctxs = case.contexts.clone();
    ctxs.sort_by_key(|c| c.at_ms);
    let mut ci = 0;
    let mut pushed_ms = 0u64;
    for chunk in pcm.chunks(FRAME) {
        while ci < ctxs.len() && ctxs[ci].at_ms <= pushed_ms {
            d.d_context(ctxs[ci].ctx.clone());
            ci += 1;
        }
        d.d_push(chunk);
        pushed_ms += (chunk.len() / 16) as u64;
        // Faster than real time, but not a burst: the remote writer queue
        // holds 6.4 s, like a real shell's 100 ms cadence would never fill.
        std::thread::sleep(Duration::from_millis(1));
    }
    for c in &ctxs[ci..] {
        d.d_context(c.ctx.clone());
    }
    d.d_commit()
}
