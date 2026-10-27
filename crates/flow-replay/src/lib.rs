//! Replay harness (beta debug tool): saved dictation recordings re-run
//! through speech models x cleanup models x prompt versions, on the real
//! `FlowCore` pipeline (segmentation, VAD, D30 routing, guards, fallback
//! rules, post-processing). `tools/replay/run.sh` drives the binary;
//! `tools/replay/score.py` scores its rows.
//!
//! Per (speech model, recording) the audio is transcribed once, by a core
//! without a cleaner: that commit is the `none` row (fallback rules only)
//! and its `asr_text` is the transcript. Every other (cleanup, prompt)
//! combination replays the same transcript through a fresh core whose ASR
//! returns it ([`FixedAsr`]), with the same audio pushed, so the cleanup
//! step sees exactly what the phone's would have and paid speech models
//! are called once.
//!
//! Cleanup slots mirror the phone's cleaner: `cloud` (AI cleanup, D30)
//! wrapped in a `RoutedCleaner` with the on-device `local` LLM when there
//! is one, `local` alone (today's pipeline: rules, prompt v2, strict
//! guard), or neither (fallback rules). The product sends prompt v4c to
//! the cloud; other versions are swapped in by [`PromptOverride`], which
//! rebuilds the prompt with `build_prompt_versioned` and changes nothing
//! else.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AbortFlag, AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::cleanup::{
    build_prompt_versioned, AiParts, CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope,
    PromptVersion, RoutedCleaner,
};
use flow_core::config::{AppCategory, FieldContext, SessionConfig};
use flow_core::{CommitResult, FlowCore};
use serde::{Deserialize, Serialize};

/// Cost of the last call of a paid engine (USD), read right after it.
pub type CostFn = Arc<dyn Fn() -> Option<f64> + Send + Sync>;

/// One saved recording plus what the phone made of it (debug log row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// The WAV's file stem (the debug log's `audio` without `.wav`).
    pub id: String,
    pub wav: PathBuf,
    /// The target app (`FieldContext.app_id`; its category follows).
    pub app_id: String,
    pub phone: Option<PhoneResult>,
}

/// The phone's debug-log fields this harness compares against.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneResult {
    pub app_version: Option<String>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub raw: Option<String>,
    pub text: Option<String>,
    pub llm: Option<String>,
    pub llm_model: Option<String>,
    pub llm_cost_usd: Option<f64>,
    pub cleanup_route: Option<String>,
    pub cloud_fallback: Option<String>,
    pub post_ms: Option<u64>,
}

/// A speech model of the matrix.
pub struct AsrSlot {
    pub name: String,
    pub engine: Arc<dyn AsrEngine>,
    /// Set for paid engines (cloud STT): the spend cap applies to them.
    pub call_cost: Option<CostFn>,
}

/// A cleanup model of the matrix (see the crate docs).
pub struct CleanupSlot {
    pub name: String,
    pub cloud: Option<Arc<dyn Cleaner>>,
    pub local: Option<Arc<dyn Cleaner>>,
}

pub struct Matrix {
    pub asr: Vec<AsrSlot>,
    pub cleanup: Vec<CleanupSlot>,
    /// Prompt versions for the cloud slots (the local pipeline is v2).
    pub prompts: Vec<PromptVersion>,
    pub session: SessionConfig,
    /// Stop paid calls once this much (USD) was spent. 0 = no paid calls.
    pub max_spend: f64,
}

/// One output row (`rows.jsonl`). No key, no request bodies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Row {
    pub rec: String,
    pub asr: String,
    pub cleanup: String,
    /// `v4c` / `v2` for cloud slots, `v2` for the local pipeline, `-`
    /// for the fallback rules.
    pub prompt: String,
    pub transcript: String,
    pub output: String,
    pub status: String,
    pub cleanup_route: Option<String>,
    pub cloud_fallback: Option<String>,
    pub fallback_reason: Option<String>,
    pub empty_reason: Option<String>,
    pub llm_engine: Option<String>,
    pub llm_model: Option<String>,
    pub llm_ms: u64,
    pub asr_ms: u64,
    pub total_ms: u64,
    /// Cleanup cost of this row (cloud).
    pub cost_usd: Option<f64>,
    /// Speech-model cost of the transcript (paid ASR; on the `none` row).
    pub asr_cost_usd: Option<f64>,
    pub error: Option<String>,
}

/// What the run spent (USD).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    pub asr: f64,
    pub cleanup: f64,
}

impl Spend {
    pub fn total(&self) -> f64 {
        self.asr + self.cleanup
    }
}

/// The Android shell's session (`DataLogic.toSessionConfig`, v0.6.0):
/// light cleanup, casual styles but formal email, 3 s LLM timeout,
/// decode-while-speaking (800 ms pauses), VAD and hallucination guard on.
/// The user dictionary is not part of a corpus.
pub fn phone_session() -> SessionConfig {
    serde_json::from_value(serde_json::json!({
        "platform": "android",
        "cleanup_level": "light",
        "raw_mode": false,
        "llm_timeout_ms": 3000,
        "segment_pause_ms": 800,
        "spoken_cleanup": true,
        "vad": true,
        "hallucination_guard": true,
        "styles": {"personal": "casual", "work": "casual", "email": "formal", "other": "casual"}
    }))
    .expect("phone session")
}

pub fn parse_prompt(s: &str) -> Result<PromptVersion, String> {
    match s.trim() {
        "v2" => Ok(PromptVersion::V2),
        "v4c" => Ok(PromptVersion::V4c),
        o => Err(format!("unknown prompt version {o:?} (v2, v4c)")),
    }
}

pub fn prompt_name(v: PromptVersion) -> &'static str {
    match v {
        PromptVersion::V2 => "v2",
        PromptVersion::V4c => "v4c",
    }
}

/// A corpus directory: `audio/*.wav` and, when present, the phone's
/// `dictations.jsonl` (rows whose `audio` file exists). Without the log,
/// every WAV in `audio/` (or the directory itself) with no phone result.
pub fn load_corpus(dir: &Path) -> Result<Vec<Recording>, String> {
    let audio = if dir.join("audio").is_dir() { dir.join("audio") } else { dir.to_path_buf() };
    let log = dir.join("dictations.jsonl");
    let mut out = Vec::new();
    if log.is_file() {
        let text = std::fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", log.display(), i + 1))?;
            let Some(file) = v["audio"].as_str() else { continue };
            let wav = audio.join(file);
            if v["kind"].as_str().is_some_and(|k| k != "dictation") || !wav.is_file() {
                continue;
            }
            let mut phone: PhoneResult = serde_json::from_value(v.clone()).unwrap_or_default();
            // The shell appends a space after a paste.
            phone.text = phone.text.map(|t| t.trim_end().to_string());
            out.push(Recording {
                id: file.trim_end_matches(".wav").to_string(),
                wav,
                app_id: v["target"].as_str().unwrap_or("").to_string(),
                phone: Some(phone),
            });
        }
    } else {
        let mut wavs: Vec<PathBuf> = std::fs::read_dir(&audio)
            .map_err(|e| format!("{}: {e}", audio.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "wav"))
            .collect();
        wavs.sort();
        for wav in wavs {
            let id = wav.file_stem().unwrap().to_string_lossy().into_owned();
            out.push(Recording { id, wav, app_id: String::new(), phone: None });
        }
    }
    Ok(out)
}

/// ASR that returns a fixed transcript (the cleanup replays), in one piece.
pub struct FixedAsr(pub String);

impl AsrEngine for FixedAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput { text: self.0.clone(), ..Default::default() })
    }
    fn segmentable(&self) -> bool {
        false
    }
}

/// A cloud cleaner whose prompt is rebuilt as `version`; everything else
/// (engine, scope, timeout, metering, prefill) is the inner cleaner's.
/// It also meters the cloud calls: after a fallback to the local LLM the
/// commit's `llm` usage is the local one, but the cloud call was paid.
pub struct PromptOverride {
    pub inner: Arc<dyn Cleaner>,
    pub version: PromptVersion,
    pub session: SessionConfig,
    pub ctx: FieldContext,
    /// (summed cost, calls in flight).
    meter: Mutex<(f64, usize)>,
}

impl PromptOverride {
    pub fn new(inner: Arc<dyn Cleaner>, version: PromptVersion, session: SessionConfig, ctx: FieldContext) -> Self {
        Self { inner, version, session, ctx, meter: Mutex::new((0.0, 0)) }
    }

    /// The summed cost of its calls, after waiting (up to `wait`) for an
    /// abandoned call (core timeout) to return and report its cost.
    pub fn cost(&self, wait: Duration) -> f64 {
        let t = Instant::now();
        loop {
            let (c, n) = *self.meter.lock().unwrap();
            if n == 0 || t.elapsed() >= wait {
                return c;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn rewrite(&self, p: &CleanPrompt) -> CleanPrompt {
        let cat = flow_core::postprocess::effective_category(self.session.platform, &self.ctx);
        let ctx = FieldContext { app_category: cat, ..self.ctx.clone() };
        build_prompt_versioned(
            &p.transcript,
            self.session.cleanup_level,
            self.session.styles.for_category(cat),
            &ctx,
            &self.session.dictionary,
            self.inner.prompt_scope(),
            self.version,
        )
    }
}

impl Cleaner for PromptOverride {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.inner.clean(&self.rewrite(p), d)
    }
    fn clean_cancellable(&self, p: &CleanPrompt, d: Instant, c: &AbortFlag) -> Result<String, CleanError> {
        self.inner.clean_cancellable(&self.rewrite(p), d, c)
    }
    fn clean_metered(
        &self,
        p: &CleanPrompt,
        d: Instant,
        c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        self.meter.lock().unwrap().1 += 1;
        let (r, u) = self.inner.clean_metered(&self.rewrite(p), d, c);
        let mut m = self.meter.lock().unwrap();
        m.0 += u.cost_usd.map_or(0.0, |c| c.0);
        m.1 -= 1;
        (r, u)
    }
    fn prefill(&self, p: &CleanPrompt) {
        self.inner.prefill(&self.rewrite(p))
    }
    fn cancel_prefill(&self) {
        self.inner.cancel_prefill()
    }
    fn info_json(&self) -> Option<String> {
        self.inner.info_json()
    }
    fn prompt_scope(&self) -> PromptScope {
        self.inner.prompt_scope()
    }
    fn engine(&self) -> &'static str {
        self.inner.engine()
    }
    fn model(&self) -> Option<String> {
        self.inner.model()
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.inner.timeout_ms()
    }
    fn estimate_cost_usd(&self, p: &CleanPrompt) -> Option<f64> {
        self.inner.estimate_cost_usd(&self.rewrite(p))
    }
    fn ai_parts(&self) -> Option<AiParts> {
        None
    }
}

fn commit(
    asr: Arc<dyn AsrEngine>,
    cleaner: Option<Arc<dyn Cleaner>>,
    session: &SessionConfig,
    ctx: &FieldContext,
    pcm: &[i16],
) -> Result<CommitResult, String> {
    let core = FlowCore::from_engines(asr, cleaner);
    let mut s = session.clone();
    s.ring_capacity_samples = s.ring_capacity_samples.max(pcm.len() + 16_000);
    core.start_session(s).map_err(|e| e.to_string())?;
    core.set_context(ctx.clone());
    // 100 ms frames, like the shells (unpaced: nothing here is live).
    for f in pcm.chunks(1600) {
        core.push_audio(f);
    }
    Ok(core.commit())
}

fn row_from(r: &CommitResult) -> Row {
    let llm = r.llm.as_ref();
    Row {
        transcript: r.asr_text.clone(),
        output: r.final_text.clone(),
        status: format!("{:?}", r.status),
        cleanup_route: r.cleanup_route.clone(),
        cloud_fallback: r.cloud_fallback.clone(),
        fallback_reason: r.fallback_reason.clone(),
        empty_reason: r.empty_reason.clone(),
        llm_engine: llm.and_then(|u| u.engine.clone()),
        llm_model: llm.and_then(|u| u.model.clone()),
        llm_ms: r.timings.llm_ms,
        asr_ms: r.timings.asr_ms + r.timings.pre_asr_ms,
        total_ms: r.timings.total_ms,
        cost_usd: llm.and_then(|u| u.cost_usd).map(|c| c.0),
        error: r.error.clone(),
        ..Default::default()
    }
}

impl Matrix {
    /// Run every combination; `pcm` loads a recording's 16 kHz mono PCM,
    /// `out` gets each row as soon as it exists.
    pub fn run(
        &self,
        recs: &[Recording],
        pcm: &mut dyn FnMut(&Recording) -> Result<Vec<i16>, String>,
        out: &mut dyn FnMut(&Row),
    ) -> Spend {
        let mut spend = Spend::default();
        for rec in recs {
            let audio = match pcm(rec) {
                Ok(a) => a,
                Err(e) => {
                    for a in &self.asr {
                        out(&Row { rec: rec.id.clone(), asr: a.name.clone(), error: Some(e.clone()), ..Default::default() });
                    }
                    continue;
                }
            };
            let ctx = FieldContext { app_id: rec.app_id.clone(), app_category: AppCategory::Other, ..Default::default() };
            for a in &self.asr {
                let base = Row { rec: rec.id.clone(), asr: a.name.clone(), ..Default::default() };
                if a.call_cost.is_some() && spend.total() >= self.max_spend {
                    out(&Row { cleanup: "none".into(), prompt: "-".into(), error: Some("spend_cap".into()), ..base });
                    continue;
                }
                let r = match commit(a.engine.clone(), None, &self.session, &ctx, &audio) {
                    Ok(r) => r,
                    Err(e) => {
                        out(&Row { cleanup: "none".into(), prompt: "-".into(), error: Some(e), ..base });
                        continue;
                    }
                };
                let asr_cost = a.call_cost.as_ref().and_then(|f| f());
                spend.asr += asr_cost.unwrap_or(0.0);
                let first = Row {
                    cleanup: "none".into(),
                    prompt: "-".into(),
                    asr_cost_usd: asr_cost,
                    ..Row { rec: base.rec.clone(), asr: base.asr.clone(), ..row_from(&r) }
                };
                out(&first);
                if r.error.is_some() || r.asr_text.trim().is_empty() || r.empty_reason.is_some() {
                    continue;
                }
                let fixed: Arc<dyn AsrEngine> = Arc::new(FixedAsr(r.asr_text.clone()));
                for c in self.cleanup.iter().filter(|c| c.cloud.is_some() || c.local.is_some()) {
                    let versions: Vec<Option<PromptVersion>> = match c.cloud {
                        Some(_) => self.prompts.iter().copied().map(Some).collect(),
                        None => vec![None],
                    };
                    for v in versions {
                        let head = Row {
                            cleanup: c.name.clone(),
                            prompt: v.map_or("v2", prompt_name).into(),
                            ..base.clone()
                        };
                        let mut meter: Option<Arc<PromptOverride>> = None;
                        let cleaner: Arc<dyn Cleaner> = match (&c.cloud, v) {
                            (Some(cloud), Some(v)) => {
                                if spend.total() >= self.max_spend {
                                    out(&Row { transcript: r.asr_text.clone(), error: Some("spend_cap".into()), ..head });
                                    continue;
                                }
                                let po = Arc::new(PromptOverride::new(cloud.clone(), v, self.session.clone(), ctx.clone()));
                                meter = Some(po.clone());
                                let o: Arc<dyn Cleaner> = po;
                                match &c.local {
                                    Some(l) => Arc::new(RoutedCleaner::new(o, Some(l.clone()))),
                                    None => o,
                                }
                            }
                            _ => c.local.clone().expect("local slot"),
                        };
                        let mut row = match commit(fixed.clone(), Some(cleaner), &self.session, &ctx, &audio) {
                            Ok(res) => row_from(&res),
                            Err(e) => Row { transcript: r.asr_text.clone(), error: Some(e), ..Default::default() },
                        };
                        // Cloud cost from the meter (also when the local LLM
                        // answered after a failed AI cleanup); local is free.
                        row.cost_usd = meter.map(|m| m.cost(Duration::from_secs(5)));
                        spend.cleanup += row.cost_usd.unwrap_or(0.0);
                        out(&Row { rec: head.rec, asr: head.asr, cleanup: head.cleanup, prompt: head.prompt, ..row });
                    }
                }
            }
        }
        spend
    }
}
