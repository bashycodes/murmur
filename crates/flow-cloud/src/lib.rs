//! flow-cloud: opt-in OpenRouter cloud speech-to-text and text cleanup
//! (D18, D28; `docs/cloud-stt.md`, `docs/proposals/openrouter-only-cloud.md`).
//!
//! The only crate in the product that sends audio or text to a third
//! party, and that third party is always OpenRouter: the API root is the
//! constant [`api::BASE_URL`], no other provider adapter exists, and
//! `tools/check-build-matrix.sh` fails if another provider host string is
//! compiled into a cloud build. It is linked only into builds that enable
//! the shell's `cloud` feature (C2).
//!
//! * [`CloudAsr`] is a `flow_core::asr::AsrEngine`: it uploads the
//!   VAD-trimmed utterance at commit (batch) through path A
//!   (`/audio/transcriptions`) or path B (chat `input_audio`), chosen per
//!   model from the [`catalog`].
//! * [`CloudBackend`] wraps it in a local `FlowCore` (VAD, guard, spoken
//!   cleanup, dictionary, LLM, post-processing stay on the device, C5) and
//!   implements `flow_router::Backend` for the router's `cloud` route.
//! * [`cleaner::OpenRouterCleaner`] is a `flow_core::cleanup::Cleaner` for
//!   the separate cleanup opt-in.
//! * [`catalog`], [`keyinfo`], [`generation`], [`budget`]: model list,
//!   `/key` credits, deferred `/generation` cost, estimates.
//!
//! Keys arrive as `SecretString` and go only into the `Authorization`
//! header. Nothing here logs; per-call metadata (never key, headers,
//! bodies or text) is in [`CloudBackend::last_call`] /
//! [`cleaner::OpenRouterCleaner::last_call`].

pub mod adapters;
pub mod api;
pub mod budget;
pub mod catalog;
pub mod cleaner;
pub mod encode;
pub mod generation;
pub mod http;
pub mod keyinfo;
pub mod presets;
pub mod settings;
#[cfg(feature = "mock-server")]
pub mod testing;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrPurpose, AsrRequest};
use flow_core::cleanup::Cleaner;
use flow_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use flow_core::{CommitResult, CommitStatus, FlowCore, RouteInfo};
use flow_router::{
    Backend, BackendError, CleanupFactory, CleanupPlan, CloudFactory, EngineKind, Failure,
    HealthReport, OpenRouterConfig, Router, SecretString, StartInfo, Usd,
};

pub use adapters::openrouter::{OpenRouterStt, TRANSCRIBE_PROMPT};
pub use adapters::{CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
pub use api::{Api, BASE_URL, HOST};
pub use catalog::{Catalog, CatalogStore, ModelInfo};
pub use cleaner::OpenRouterCleaner;
pub use encode::{AudioFormat, EncodedAudio};
pub use generation::{CostSink, DeferredCost};
pub use http::CloudError;
pub use keyinfo::KeyInfo;
pub use presets::presets;
pub use settings::{
    CleanupScope, CleanupSettings, Privacy, SttPath, SttPathChoice, SttSettings,
    DEFAULT_CLEANUP_MODEL, DEFAULT_STT_MODEL,
};

/// Adapters compiled into this crate (`Router::cloud_capabilities`): one.
pub const ADAPTERS: [&str; 1] = ["openrouter"];
/// The provider id of the one OpenRouter entry.
pub const PROVIDER_ID: &str = "openrouter";

/// The Developer page's "Test provider" clip: the first 3 s of the
/// public-domain JFK inaugural address sample ("And so, my fellow
/// Americans"), 16 kHz mono s16le.
pub const TEST_CLIP: &[u8] = include_bytes!("../assets/test_clip_3s.s16le");

/// Plug the cloud route and the cloud cleanup factory into a router:
/// `cleaner` is the shell's local LLM cleaner (`FlowCore::cleaner()`),
/// shared so the model is loaded once.
pub fn attach(router: Router, cleaner: Option<Arc<dyn Cleaner>>) -> Router {
    attach_with(router, move || cleaner.clone())
}

/// `flow_router::CloudFactory` building a [`CloudBackend`].
pub fn factory(cleaner: Option<Arc<dyn Cleaner>>) -> CloudFactory {
    factory_with(move || cleaner.clone())
}

/// Like [`factory`], but asks for the cleaner each time a backend is built,
/// for shells whose LLM loads after the router exists (the router rebuilds
/// the cloud backend after `Router::set_local`).
pub fn factory_with(
    cleaner: impl Fn() -> Option<Arc<dyn Cleaner>> + Send + Sync + 'static,
) -> CloudFactory {
    Box::new(move |o: &OpenRouterConfig, key: SecretString| {
        Arc::new(CloudBackend::new(o.into(), key, cleaner())) as Arc<dyn Backend>
    })
}

/// `flow_router::CleanupFactory` building an [`OpenRouterCleaner`].
pub fn cleanup_factory() -> CleanupFactory {
    Box::new(|o: &OpenRouterConfig, key: SecretString| {
        Arc::new(OpenRouterCleaner::new(o.into(), key)) as Arc<dyn Cleaner>
    })
}

/// [`attach`] with a cleaner looked up per backend build ([`factory_with`]).
pub fn attach_with(
    router: Router,
    cleaner: impl Fn() -> Option<Arc<dyn Cleaner>> + Send + Sync + 'static,
) -> Router {
    router
        .with_cloud(factory_with(cleaner))
        .with_cleanup_factory(cleanup_factory())
        .with_cloud_adapters(ADAPTERS.iter().map(|s| s.to_string()).collect())
}

/// A cleanup backend for the given settings.
pub fn cleaner_for(settings: CleanupSettings, key: SecretString) -> Arc<dyn Cleaner> {
    Arc::new(OpenRouterCleaner::new(settings, key))
}

/// What the last provider call did (Developer page; no secrets, no text).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallInfo {
    pub provider_id: String,
    pub adapter: &'static str,
    pub model: String,
    /// `"transcriptions"` or `"chat_audio"`.
    pub stt_path: &'static str,
    /// HTTP requests that produced the transcript (chunks + retries).
    pub requests: u32,
    pub bytes: usize,
    pub audio_ms: u32,
    pub billed_audio_ms: u32,
    pub cost_usd: Option<f64>,
    pub cost_estimated: bool,
    pub encode_ms: u32,
    pub latency_ms: u32,
    /// Generation id (`X-Generation-Id`) of the last chunk.
    pub request_id: Option<String>,
    /// Failure code (`CloudError::code`) and safe message when it failed.
    pub error: Option<String>,
    pub message: Option<String>,
}

#[derive(Default)]
struct CallSlot {
    deadline: Option<Instant>,
    error: Option<BackendError>,
    info: Option<CallInfo>,
}

/// `AsrEngine` that sends the utterance to OpenRouter.
pub struct CloudAsr {
    settings: SttSettings,
    adapter: OpenRouterStt,
    api: Api,
    key: SecretString,
    info: Option<ModelInfo>,
    slot: Mutex<CallSlot>,
    cost_sink: Option<CostSink>,
    lookup_delay: Duration,
}

impl CloudAsr {
    /// The model's catalogue entry (path, prices) comes from the
    /// registered store, else the baked catalogue.
    pub fn new(settings: SttSettings, key: SecretString) -> Self {
        let info = catalog::resolve(settings.model.trim());
        Self::build(settings, key, Api::new(), info)
    }

    fn build(settings: SttSettings, key: SecretString, api: Api, info: Option<ModelInfo>) -> Self {
        Self {
            adapter: OpenRouterStt::new(api.clone(), &settings, info.clone()),
            settings,
            api,
            key,
            info,
            slot: Mutex::new(CallSlot::default()),
            cost_sink: None,
            lookup_delay: generation::LOOKUP_DELAY,
        }
    }

    /// Replace the catalogue entry (re-picks the path).
    pub fn with_model_info(self, info: Option<ModelInfo>) -> Self {
        let (sink, delay) = (self.cost_sink.clone(), self.lookup_delay);
        let mut a = Self::build(self.settings, self.key, self.api, info);
        a.cost_sink = sink;
        a.lookup_delay = delay;
        a
    }

    /// Delay before a deferred `/generation` lookup (default 30 s).
    pub fn with_cost_lookup_delay(mut self, d: Duration) -> Self {
        self.lookup_delay = d;
        self
    }

    /// Where deferred `/generation` costs go.
    pub fn with_cost_sink(mut self, sink: CostSink) -> Self {
        self.cost_sink = Some(sink);
        self
    }

    pub fn settings(&self) -> &SttSettings {
        &self.settings
    }

    pub fn path(&self) -> SttPath {
        self.adapter.path()
    }

    /// Deadline for the next call (set by `CloudBackend::commit`).
    fn set_deadline(&self, d: Option<Instant>) {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        s.deadline = d;
        s.error = None;
        s.info = None;
    }

    fn take(&self) -> (Option<BackendError>, Option<CallInfo>) {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        (s.error.take(), s.info.clone())
    }

    pub fn last_call(&self) -> Option<CallInfo> {
        self.slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .info
            .clone()
    }

    fn default_deadline(&self, audio_ms: u64) -> Instant {
        Instant::now()
            + Duration::from_millis(self.settings.timeout_base_ms + audio_ms / 2)
                .min(Duration::from_secs(90))
    }

    /// The biasing terms the user allowed, capped for this model's path.
    pub fn terms(&self, req: &AsrRequest) -> Vec<String> {
        if !self.settings.send_dictionary {
            return Vec::new();
        }
        self.adapter.term_budget().cap(&req.dictionary_terms)
    }

    /// Encode, chunk, upload; join the chunk texts with one space. The call
    /// metadata comes back whether or not it worked.
    pub fn run(
        &self,
        pcm: &[f32],
        language: Option<&str>,
        terms: &[String],
        deadline: Instant,
    ) -> (Result<CloudTranscript, CloudError>, CallInfo) {
        let t0 = Instant::now();
        let max_by_ms = self.adapter.max_chunk_ms() as usize * SAMPLE_RATE / 1000;
        // FLAC is never larger than PCM16 + headers; plan with 2 B/sample.
        let max_by_bytes = self.adapter.max_upload_bytes().saturating_sub(4096) / 2;
        let ranges = encode::chunk_ranges(pcm, max_by_ms.min(max_by_bytes));
        let mut info = CallInfo {
            provider_id: PROVIDER_ID.into(),
            adapter: self.adapter.id(),
            model: self.settings.model.trim().to_string(),
            stt_path: self.adapter.path().as_str(),
            audio_ms: (pcm.len() * 1000 / SAMPLE_RATE) as u32,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let mut lang = None;
        let mut billed = 0u32;
        let mut cost = Some(0.0);
        let mut encode_ms = 0u128;
        let mut unpriced_ids = Vec::new();
        for r in ranges {
            let mut retried_format = false;
            let out = loop {
                let te = Instant::now();
                let audio = encode::encode(&pcm[r.clone()], self.adapter.preferred_format());
                encode_ms += te.elapsed().as_millis();
                info.bytes += audio.bytes.len();
                let req = CloudRequest {
                    audio: &audio,
                    language,
                    terms,
                    deadline,
                };
                info.requests += 1;
                match self.adapter.transcribe(&self.key, &req) {
                    Ok(o) => break Ok((o, audio.audio_ms)),
                    Err(CloudError::UnsupportedFormat)
                        if !retried_format && self.adapter.format_rejected(audio.format) =>
                    {
                        retried_format = true;
                    }
                    Err(e) => break Err(e),
                }
            };
            let (out, chunk_ms) = match out {
                Ok(o) => o,
                Err(e) => {
                    let e = scrub(e, &self.key);
                    info.encode_ms = encode_ms as u32;
                    info.latency_ms = t0.elapsed().as_millis() as u32;
                    info.error = Some(e.code().into());
                    info.message = Some(e.to_string());
                    return (Err(e), info);
                }
            };
            billed += out.billed_audio_ms.unwrap_or(chunk_ms);
            if out.cost_usd.is_none() {
                unpriced_ids.extend(out.request_id.clone());
            }
            cost = match (cost, out.cost_usd) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
            if !out.text.trim().is_empty() {
                texts.push(out.text.trim().to_string());
            }
            lang = lang.or(out.language);
            info.request_id = out.request_id.or(info.request_id);
        }
        info.encode_ms = encode_ms as u32;
        info.latency_ms = t0.elapsed().as_millis() as u32;
        info.billed_audio_ms = billed;
        match cost {
            Some(c) => info.cost_usd = Some(c),
            None => {
                info.cost_estimated = true;
                info.cost_usd = self
                    .info
                    .as_ref()
                    .and_then(|m| budget::estimate_stt(m, billed as f64 / 1000.0));
                if let Some(sink) = &self.cost_sink {
                    let n = unpriced_ids.len().max(1) as f64;
                    for id in unpriced_ids {
                        generation::schedule(
                            self.api.clone(),
                            self.key.clone(),
                            generation::CostUse::Stt,
                            id,
                            info.cost_usd.map(|c| c / n),
                            self.lookup_delay,
                            sink.clone(),
                        );
                    }
                }
            }
        }
        (
            Ok(CloudTranscript {
                text: texts.join(" "),
                language: lang,
                billed_audio_ms: Some(billed),
                cost_usd: info.cost_usd,
                request_id: info.request_id.clone(),
            }),
            info,
        )
    }
}

/// Drop a provider error code that contains (part of) the key: a hostile
/// or buggy provider may echo it back (C3).
fn scrub(e: CloudError, key: &SecretString) -> CloudError {
    match e {
        CloudError::BadRequest {
            status,
            code: Some(c),
        } if !key.is_empty()
            && (c.contains(key.expose()) || (c.len() >= 8 && key.expose().contains(&c))) =>
        {
            CloudError::BadRequest { status, code: None }
        }
        e => e,
    }
}

impl AsrEngine for CloudAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        // Batch only: no audio leaves the device for live previews.
        if req.purpose == AsrPurpose::Partial {
            return Err(AsrError::Transcribe("partials unsupported".into()));
        }
        // Cancelled before the upload: no audio leaves the device (C4).
        if req.abort.is_set() {
            return Err(AsrError::Transcribe("cancelled".into()));
        }
        let audio_ms = (pcm.len() * 1000 / SAMPLE_RATE) as u64;
        let deadline = self
            .slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .deadline
            .unwrap_or_else(|| self.default_deadline(audio_ms));
        let language = (!self.settings.language.trim().is_empty())
            .then(|| self.settings.language.trim().to_string())
            .or_else(|| req.language.clone());
        let terms = self.terms(req);
        let (res, info) = self.run(pcm, language.as_deref(), &terms, deadline);
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        s.info = Some(info);
        match res {
            Ok(t) => Ok(AsrOutput {
                text: t.text,
                language: t.language.or(language),
                no_speech_prob: None,
            }),
            Err(e) => {
                s.error = Some(e.to_backend());
                Err(AsrError::Transcribe(format!("{}: {e}", e.code())))
            }
        }
    }
    /// No audio leaves the device before commit, and one billed call per
    /// dictation (C5): the core ignores `segment_pause_ms` (D22).
    fn segmentable(&self) -> bool {
        false
    }
}

/// The router's `cloud` backend: a local `FlowCore` around [`CloudAsr`].
pub struct CloudBackend {
    inner: Option<(FlowCore, Arc<CloudAsr>)>,
    llm_timeout_ms: Mutex<u64>,
}

impl CloudBackend {
    pub fn new(
        settings: SttSettings,
        key: SecretString,
        cleaner: Option<Arc<dyn Cleaner>>,
    ) -> Self {
        Self::from_asr(CloudAsr::new(settings, key), cleaner)
    }

    pub fn from_asr(asr: CloudAsr, cleaner: Option<Arc<dyn Cleaner>>) -> Self {
        let asr = Arc::new(asr);
        Self {
            inner: Some((FlowCore::from_engines(asr.clone(), cleaner), asr)),
            llm_timeout_ms: Mutex::new(0),
        }
    }

    /// A backend for a config that is not an OpenRouter entry: every call
    /// fails with `cloud_not_configured` and nothing is sent.
    pub fn refusing() -> Self {
        Self {
            inner: None,
            llm_timeout_ms: Mutex::new(0),
        }
    }

    fn not_configured() -> BackendError {
        BackendError::new(
            Failure::CloudNotConfigured,
            "cloud speech-to-text uses OpenRouter only",
        )
    }

    pub fn last_call(&self) -> Option<CallInfo> {
        self.inner.as_ref().and_then(|(_, a)| a.last_call())
    }

    fn route(asr: &CloudAsr, info: Option<&CallInfo>) -> RouteInfo {
        RouteInfo {
            engine: EngineKind::Cloud.as_str().into(),
            provider: Some(PROVIDER_ID.into()),
            model: Some(asr.settings().model.trim().to_string()),
            billed_audio_ms: info.map(|i| i.billed_audio_ms),
            cost_usd: info.and_then(|i| i.cost_usd).map(Usd),
            cost_estimated: info.map(|i| i.cost_estimated),
            ..Default::default()
        }
    }
}

impl Backend for CloudBackend {
    fn kind(&self) -> EngineKind {
        EngineKind::Cloud
    }

    fn ready(&self) -> bool {
        self.inner.is_some()
    }

    /// D28: cloud-STT dictations clean with the router's plan too (takes
    /// effect at the next session).
    fn install_cleanup(&self, plan: Option<CleanupPlan>) {
        if let Some((core, _)) = &self.inner {
            core.set_cleaner(plan.and_then(|p| p(core.cleaner())));
        }
    }

    /// No network: key and consent were checked by the router.
    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        let Some((core, asr)) = &self.inner else {
            return Err(Self::not_configured());
        };
        let mut cfg = cfg.clone();
        cfg.partial_interval_ms = None;
        // Belt and braces next to `CloudAsr::segmentable` (D22, C5).
        cfg.segment_pause_ms = None;
        *self
            .llm_timeout_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = cfg.llm_timeout_ms;
        asr.set_deadline(None);
        core.start_session(cfg)
            .map(|_| StartInfo::default())
            .map_err(|e| BackendError::new(Failure::Error, e.to_string()))
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        self.inner.as_ref().is_some_and(|(c, _)| c.push_audio(pcm))
    }

    fn set_context(&self, ctx: &FieldContext) {
        if let Some((c, _)) = &self.inner {
            c.set_context(ctx.clone())
        }
    }

    fn partial(&self) -> Option<String> {
        None
    }

    /// The upload happens here. A transport/provider failure is an `Err`
    /// (the router falls back with the buffered audio); any 2xx answer is
    /// final, even an empty transcript.
    fn commit(&self, deadline: Instant) -> Result<CommitResult, BackendError> {
        let Some((core, asr)) = &self.inner else {
            return Err(Self::not_configured());
        };
        let llm = Duration::from_millis(
            *self
                .llm_timeout_ms
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        let asr_deadline = deadline.checked_sub(llm).unwrap_or(deadline);
        asr.set_deadline(Some(
            asr_deadline.max(Instant::now() + Duration::from_millis(500)),
        ));
        let mut r = core.commit();
        let (err, info) = asr.take();
        if r.status == CommitStatus::Error {
            if let Some(e) = err {
                return Err(e);
            }
        }
        r.route = Some(Self::route(
            asr,
            info.as_ref().filter(|i| i.error.is_none()),
        ));
        Ok(r)
    }

    fn cancel(&self) {
        if let Some((c, _)) = &self.inner {
            c.cancel()
        }
    }

    /// "Test provider": transcribe [`TEST_CLIP`] (no VAD, no terms).
    fn probe(&self) -> HealthReport {
        let Some((_, asr)) = &self.inner else {
            return HealthReport {
                ok: false,
                engine: EngineKind::Cloud,
                error: Some(Failure::CloudNotConfigured.as_str().into()),
                ..Default::default()
            };
        };
        let pcm: Vec<f32> = TEST_CLIP
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let p = asr.settings();
        let lang = (!p.language.trim().is_empty()).then(|| p.language.trim().to_string());
        let deadline = Instant::now() + Duration::from_millis(p.timeout_base_ms + 1500);
        let (res, info) = asr.run(&pcm, lang.as_deref(), &[], deadline);
        let mut h = match res {
            Ok(t) => HealthReport {
                ok: true,
                transcript: Some(t.text),
                latency_ms: Some(info.latency_ms),
                cost_usd: info.cost_usd.map(Usd),
                ..Default::default()
            },
            Err(e) => HealthReport {
                ok: false,
                error: Some(e.code().into()),
                message: Some(e.to_string()),
                latency_ms: Some(info.latency_ms),
                ..Default::default()
            },
        };
        h.engine = EngineKind::Cloud;
        h.provider = Some(PROVIDER_ID.into());
        h.model = Some(p.model.trim().to_string());
        h.checked_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_adapter_and_a_refusing_backend() {
        assert_eq!(ADAPTERS, ["openrouter"]);
        assert_eq!(ADAPTERS, flow_router::cloud::ADAPTERS);
        assert_eq!(BASE_URL, flow_router::cloud::OPENROUTER_BASE_URL);
        assert_eq!(HOST, flow_router::cloud::OPENROUTER_HOST);
        assert_eq!(PROVIDER_ID, flow_router::cloud::OPENROUTER_ID);
        assert!(!CloudBackend::refusing().ready());
    }
}
