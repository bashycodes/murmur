//! flow-cloud: opt-in third-party speech-to-text (D18, `docs/cloud-stt.md`).
//!
//! The only crate in the product that sends audio to a third party. It is
//! linked only into builds that enable the shell's `cloud` feature, so
//! client-only builds contain no HTTP stack (C2, enforced by
//! `tools/check-build-matrix.sh`).
//!
//! * [`CloudAsr`] is a `flow_core::asr::AsrEngine`: it uploads the
//!   VAD-trimmed utterance at commit (batch, no streaming) and returns the
//!   provider's raw text.
//! * [`CloudBackend`] wraps it in a local `FlowCore`, so VAD (silence is
//!   never uploaded, C6), the hallucination guard, spoken cleanup, the
//!   dictionary, the optional local LLM and Wispr post-processing run on the
//!   device exactly as for local ASR (C5), and implements
//!   `flow_router::Backend` for the router's `cloud` route.
//! * [`attach`] / [`factory`] plug it into a `flow_router::Router`.
//!
//! Keys arrive as `SecretString` from the router and go only into the
//! provider's auth header. Nothing here logs; per-call metadata (status,
//! bytes, audio, latency, cost — never key, headers, bodies or text) is in
//! [`CloudBackend::last_call`].

pub mod adapters;
pub mod encode;
pub mod http;
pub mod presets;
#[cfg(feature = "mock-server")]
pub mod testing;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrPurpose, AsrRequest};
use flow_core::cleanup::Cleaner;
use flow_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use flow_core::{CommitResult, CommitStatus, FlowCore, RouteInfo};
use flow_router::{
    Backend, BackendError, CloudFactory, CloudProviderConfig, EngineKind, HealthReport, Router,
    SecretString, StartInfo, Usd,
};

pub use adapters::{for_provider, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
pub use encode::{AudioFormat, EncodedAudio};
pub use http::CloudError;
pub use presets::{presets, price_per_hour};

/// Adapters compiled into this crate (`Router::cloud_capabilities`).
pub const ADAPTERS: [&str; 5] = flow_router::cloud::ADAPTERS;

/// The Developer page's "Test provider" clip: the first 3 s of the
/// public-domain JFK inaugural address sample ("And so, my fellow
/// Americans"), 16 kHz mono s16le.
pub const TEST_CLIP: &[u8] = include_bytes!("../assets/test_clip_3s.s16le");

/// Plug the cloud route into a router: `cleaner` is the shell's local LLM
/// cleaner (`FlowCore::cleaner()`), shared so the model is loaded once.
pub fn attach(router: Router, cleaner: Option<Arc<dyn Cleaner>>) -> Router {
    router
        .with_cloud(factory(cleaner))
        .with_cloud_adapters(ADAPTERS.iter().map(|s| s.to_string()).collect())
}

/// `flow_router::CloudFactory` building a [`CloudBackend`].
pub fn factory(cleaner: Option<Arc<dyn Cleaner>>) -> CloudFactory {
    Box::new(move |p: &CloudProviderConfig, key: SecretString| {
        Arc::new(CloudBackend::new(p.clone(), key, cleaner.clone())) as Arc<dyn Backend>
    })
}

/// What the last provider call did (Developer page; no secrets, no text).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallInfo {
    pub provider_id: String,
    pub adapter: &'static str,
    pub model: String,
    /// HTTP requests that produced the transcript (chunks).
    pub requests: u32,
    pub bytes: usize,
    pub audio_ms: u32,
    pub billed_audio_ms: u32,
    pub cost_usd: Option<f64>,
    pub cost_estimated: bool,
    pub encode_ms: u32,
    pub latency_ms: u32,
    pub request_id: Option<String>,
    /// Failure code (`Failure::as_str`) and safe message when it failed.
    pub error: Option<String>,
    pub message: Option<String>,
}

#[derive(Default)]
struct CallSlot {
    deadline: Option<Instant>,
    error: Option<BackendError>,
    info: Option<CallInfo>,
}

/// `AsrEngine` that sends the utterance to a provider.
pub struct CloudAsr {
    provider: CloudProviderConfig,
    adapter: Box<dyn CloudAdapter>,
    http: http::Client,
    key: SecretString,
    slot: Mutex<CallSlot>,
}

impl CloudAsr {
    pub fn new(provider: CloudProviderConfig, key: SecretString) -> Self {
        Self {
            adapter: for_provider(&provider),
            provider,
            http: http::client(),
            key,
            slot: Mutex::new(CallSlot::default()),
        }
    }

    pub fn provider(&self) -> &CloudProviderConfig {
        &self.provider
    }

    /// Deadline for the next call (set by `CloudBackend::commit`).
    fn set_deadline(&self, d: Option<Instant>) {
        let mut s = self.slot.lock().unwrap();
        s.deadline = d;
        s.error = None;
        s.info = None;
    }

    fn take(&self) -> (Option<BackendError>, Option<CallInfo>) {
        let mut s = self.slot.lock().unwrap();
        (s.error.take(), s.info.clone())
    }

    pub fn last_call(&self) -> Option<CallInfo> {
        self.slot.lock().unwrap().info.clone()
    }

    fn default_deadline(&self, audio_ms: u64) -> Instant {
        Instant::now()
            + Duration::from_millis(self.provider.timeout_base_ms + audio_ms / 2)
                .min(Duration::from_secs(90))
    }

    /// The biasing terms the user allowed, capped for this adapter.
    pub fn terms(&self, req: &AsrRequest) -> Vec<String> {
        let mut t = Vec::new();
        if self.provider.send_dictionary {
            t.extend(req.dictionary_terms.iter().cloned());
        }
        if self.provider.send_screen_words {
            t.extend(req.screen_terms.iter().cloned());
        }
        self.adapter.term_budget().cap(&t)
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
        let format = self.adapter.preferred_format();
        let max_by_ms = self.adapter.max_chunk_ms() as usize * SAMPLE_RATE / 1000;
        // FLAC is never larger than PCM16 + headers; plan with 2 B/sample.
        let max_by_bytes = self.adapter.max_upload_bytes().saturating_sub(4096) / 2;
        let ranges = encode::chunk_ranges(pcm, max_by_ms.min(max_by_bytes));
        let mut info = CallInfo {
            provider_id: self.provider.id.clone(),
            adapter: self.adapter.id(),
            model: self.provider.model.clone(),
            audio_ms: (pcm.len() * 1000 / SAMPLE_RATE) as u32,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let mut lang = None;
        let mut billed = 0u32;
        let mut cost = Some(0.0);
        let mut encode_ms = 0u128;
        for r in ranges {
            let te = Instant::now();
            let audio = encode::encode(&pcm[r], format);
            encode_ms += te.elapsed().as_millis();
            info.bytes += audio.bytes.len();
            let req = CloudRequest {
                audio: &audio,
                language,
                terms,
                deadline,
            };
            let out = match self.adapter.transcribe(&self.http, &self.key, &req) {
                Ok(o) => o,
                Err(e) => {
                    info.encode_ms = encode_ms as u32;
                    info.latency_ms = t0.elapsed().as_millis() as u32;
                    info.error = Some(e.failure().as_str().into());
                    info.message = Some(e.to_string());
                    return (Err(e), info);
                }
            };
            info.requests += 1;
            billed += out.billed_audio_ms.unwrap_or(audio.audio_ms);
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
                info.cost_usd =
                    price_per_hour(&self.provider).map(|p| p * billed as f64 / 3_600_000.0);
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

impl AsrEngine for CloudAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        // v1 is batch only: no audio leaves the device for live previews.
        if req.purpose == AsrPurpose::Partial {
            return Err(AsrError::Transcribe("partials unsupported".into()));
        }
        let audio_ms = (pcm.len() * 1000 / SAMPLE_RATE) as u64;
        let deadline = self
            .slot
            .lock()
            .unwrap()
            .deadline
            .unwrap_or_else(|| self.default_deadline(audio_ms));
        let language = (!self.provider.language.trim().is_empty())
            .then(|| self.provider.language.trim().to_string())
            .or_else(|| req.language.clone());
        let terms = self.terms(req);
        let (res, info) = self.run(pcm, language.as_deref(), &terms, deadline);
        let mut s = self.slot.lock().unwrap();
        s.info = Some(info);
        match res {
            Ok(t) => Ok(AsrOutput {
                text: t.text,
                language: t.language.or(language),
                no_speech_prob: None,
            }),
            Err(e) => {
                s.error = Some(e.to_backend());
                Err(AsrError::Transcribe(format!("{}: {e}", e.failure())))
            }
        }
    }
}

/// The router's `cloud` backend: a local `FlowCore` around [`CloudAsr`].
pub struct CloudBackend {
    core: FlowCore,
    asr: Arc<CloudAsr>,
    llm_timeout_ms: Mutex<u64>,
}

impl CloudBackend {
    pub fn new(
        provider: CloudProviderConfig,
        key: SecretString,
        cleaner: Option<Arc<dyn Cleaner>>,
    ) -> Self {
        let asr = Arc::new(CloudAsr::new(provider, key));
        Self {
            core: FlowCore::from_engines(asr.clone(), cleaner),
            asr,
            llm_timeout_ms: Mutex::new(0),
        }
    }

    pub fn last_call(&self) -> Option<CallInfo> {
        self.asr.last_call()
    }

    fn route(&self, info: Option<&CallInfo>) -> RouteInfo {
        let p = self.asr.provider();
        RouteInfo {
            engine: EngineKind::Cloud.as_str().into(),
            provider: Some(p.id.clone()),
            model: Some(p.model.clone()),
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
        true
    }

    /// No network: provider, key and consent were checked by the router.
    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        let mut cfg = cfg.clone();
        cfg.partial_interval_ms = None;
        *self.llm_timeout_ms.lock().unwrap() = cfg.llm_timeout_ms;
        self.asr.set_deadline(None);
        self.core
            .start_session(cfg)
            .map(|_| StartInfo::default())
            .map_err(|e| BackendError::new(flow_router::Failure::Error, e.to_string()))
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        self.core.push_audio(pcm)
    }

    fn set_context(&self, ctx: &FieldContext) {
        self.core.set_context(ctx.clone())
    }

    fn partial(&self) -> Option<String> {
        None
    }

    /// The upload happens here. A transport/provider failure is an `Err`
    /// (the router falls back with the buffered audio); any 2xx answer is
    /// final, even an empty transcript.
    fn commit(&self, deadline: Instant) -> Result<CommitResult, BackendError> {
        let llm = Duration::from_millis(*self.llm_timeout_ms.lock().unwrap());
        let asr_deadline = deadline.checked_sub(llm).unwrap_or(deadline);
        self.asr.set_deadline(Some(
            asr_deadline.max(Instant::now() + Duration::from_millis(500)),
        ));
        let mut r = self.core.commit();
        let (err, info) = self.asr.take();
        if r.status == CommitStatus::Error {
            if let Some(e) = err {
                return Err(e);
            }
        }
        r.route = Some(self.route(info.as_ref().filter(|i| i.error.is_none())));
        Ok(r)
    }

    fn cancel(&self) {
        self.core.cancel()
    }

    /// "Test provider": transcribe [`TEST_CLIP`] (no VAD, no terms).
    fn probe(&self) -> HealthReport {
        let pcm: Vec<f32> = TEST_CLIP
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let p = self.asr.provider();
        let lang = (!p.language.trim().is_empty()).then(|| p.language.trim().to_string());
        let deadline = Instant::now() + Duration::from_millis(p.timeout_base_ms + 1500);
        let (res, info) = self.asr.run(&pcm, lang.as_deref(), &[], deadline);
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
                error: Some(e.failure().as_str().into()),
                message: Some(e.to_string()),
                latency_ms: Some(info.latency_ms),
                ..Default::default()
            },
        };
        h.engine = EngineKind::Cloud;
        h.provider = Some(p.id.clone());
        h.model = Some(p.model.clone());
        h.checked_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        h
    }
}
