//! flow-cli: dev harness that drives the full flow-core pipeline on a WAV.
//!
//! The audio is fed through `push_audio` in 40 ms chunks exactly like a
//! shell would, so the reported `total_ms` is release-to-text latency.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};

use flow_core::config::{AsrBackend, FieldContext, ModelConfig, Platform, SessionConfig};
use flow_core::dictionary::Dictionary;
use flow_core::{audio, FlowCore};

const USAGE: &str =
    "usage: flow-cli transcribe (--model ggml.bin | --engine parakeet --model DIR | --mock TEXT) [--dict dict.json] \
[--session session.json] [--context ctx.json] [--lang xx]... [--platform windows|android] \
[--threads N] [--parakeet-decoding auto|greedy|beam] [--llm model.gguf [--llm-lib libflow_llm.so]] [--level none|light|medium|high] \
[--realtime] [--json] file.wav
       flow-cli clean --llm model.gguf [same options, no wav/model] [--category email|work_chat|...] TEXT...
         (each TEXT runs as one dictation through LLM cleanup + post-processing; prints llm counters)
       flow-cli meeting [transcribe's model/session/lang options] [--router router.json] [--progress] [--json] file.wav
         (a recording of any length: chunked at pauses, each chunk one dictation on the router's route
          order, streamed from disk; prints `[hh:mm:ss.mmm --> hh:mm:ss.mmm] text` per segment as it is made;
          --router needs the `remote` feature for a server route; without it the route is this machine)";

struct Args {
    clean: Vec<String>,
    /// `meeting` mode: the router config (`None` = this machine only).
    meeting: Option<Option<flow_router::RouterConfig>>,
    progress: bool,
    model: ModelConfig,
    session: SessionConfig,
    context: Option<FieldContext>,
    wav: PathBuf,
    json: bool,
    realtime: bool,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("{path}: {e}"))
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut it = argv.iter();
    let (clean_mode, meeting_mode) = match it.next().map(String::as_str) {
        Some("transcribe") => (false, false),
        Some("clean") => (true, false),
        Some("meeting") => (false, true),
        _ => return Err(USAGE.into()),
    };
    let mut router = None;
    let mut progress = false;
    let mut texts = Vec::new();
    let mut level = None;
    let mut category = None;
    let mut model = ModelConfig::default();
    let mut session = SessionConfig::default();
    let mut dict: Option<Dictionary> = None;
    let mut langs = Vec::new();
    let mut platform = None;
    let mut engine = None;
    let (mut context, mut wav, mut json, mut realtime, mut have_model) =
        (None, None, false, false, false);
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--model" => {
                model.asr_backend = AsrBackend::Whisper;
                model.asr_path = val()?;
                have_model = true;
            }
            "--engine" => {
                engine = Some(match val()?.as_str() {
                    "whisper" => AsrBackend::Whisper,
                    "parakeet" => AsrBackend::Parakeet,
                    "sherpa_online" | "zipformer" => AsrBackend::SherpaOnline,
                    e => {
                        return Err(format!(
                            "unknown engine {e} (whisper|parakeet|sherpa_online)"
                        ))
                    }
                })
            }
            "--mock" => {
                model.asr_backend = AsrBackend::Mock { text: val()? };
                have_model = true;
            }
            "--dict" => dict = Some(read_json(&val()?)?),
            "--session" => session = read_json(&val()?)?,
            "--context" => context = Some(read_json(&val()?)?),
            "--lang" => langs.push(val()?),
            "--platform" => {
                platform = Some(match val()?.as_str() {
                    "windows" => Platform::Windows,
                    "android" => Platform::Android,
                    p => return Err(format!("unknown platform {p}")),
                })
            }
            "--threads" => model.threads = val()?.parse().map_err(|e| format!("--threads: {e}"))?,
            "--parakeet-decoding" => {
                model.parakeet_decoding = serde_json::from_value(serde_json::Value::String(val()?))
                    .map_err(|e| format!("--parakeet-decoding: {e}"))?
            }
            "--llm" => model.llm_path = Some(val()?),
            "--llm-lib" => model.llm_lib = Some(val()?),
            "--level" => {
                level = Some(
                    serde_json::from_value(serde_json::Value::String(val()?))
                        .map_err(|e| format!("--level: {e}"))?,
                )
            }
            "--category" => {
                category = Some(
                    serde_json::from_value(serde_json::Value::String(val()?))
                        .map_err(|e| format!("--category: {e}"))?,
                )
            }
            "--router" if meeting_mode => router = Some(read_json(&val()?)?),
            "--progress" if meeting_mode => progress = true,
            "--json" => json = true,
            "--realtime" => realtime = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s if clean_mode => texts.push(s.to_string()),
            s => wav = Some(PathBuf::from(s)),
        }
    }
    if let Some(e) = engine {
        if matches!(model.asr_backend, AsrBackend::Mock { .. }) {
            return Err(format!("--engine needs --model, not --mock\n{USAGE}"));
        }
        model.asr_backend = e;
    }
    if let Some(l) = level {
        session.cleanup_level = l;
    }
    if let Some(c) = category {
        context
            .get_or_insert_with(FieldContext::default)
            .app_category = c;
    }
    if let Some(d) = dict {
        session.dictionary = d;
    }
    if !langs.is_empty() {
        session.languages = langs;
    }
    if let Some(p) = platform {
        session.platform = p;
    }
    if clean_mode {
        if model.llm_path.is_none() || texts.is_empty() {
            return Err(USAGE.into());
        }
        return Ok(Args {
            clean: texts,
            meeting: None,
            progress,
            model,
            session,
            context,
            wav: PathBuf::new(),
            json,
            realtime,
        });
    }
    let wav = wav.ok_or_else(|| USAGE.to_string())?;
    if !have_model {
        return Err(USAGE.into());
    }
    Ok(Args {
        clean: Vec::new(),
        meeting: meeting_mode.then_some(router),
        progress,
        model,
        session,
        context,
        wav,
        json,
        realtime,
    })
}

/// Mock ASR whose transcript is swapped per dictation (`clean`).
struct TextAsr(Arc<Mutex<String>>);
impl AsrEngine for TextAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: self.0.lock().unwrap().clone(),
            ..Default::default()
        })
    }
}

#[cfg(feature = "llm")]
fn run_clean(a: Args) -> Result<String, String> {
    let t = Instant::now();
    let cl = Arc::new(
        flow_core::llama::LlamaCleaner::load(
            a.model.llm_path.as_deref().unwrap_or_default(),
            a.model.threads,
            a.model.llm_lib.as_deref(),
        )
        .map_err(|e| e.to_string())?,
    );
    eprintln!("load {} ms | {}", t.elapsed().as_millis(), cl.info_json());
    let text = Arc::new(Mutex::new(String::new()));
    let core = FlowCore::from_engines(Arc::new(TextAsr(text.clone())), Some(cl.clone()));
    let mut out = Vec::new();
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    for s in &a.clean {
        *text.lock().unwrap() = s.clone();
        core.start_session(a.session.clone())
            .map_err(|e| e.to_string())?;
        if let Some(c) = &a.context {
            core.set_context(c.clone());
        }
        core.push_audio(&tone);
        let r = core.commit();
        let info: serde_json::Value = serde_json::from_str(&cl.info_json()).unwrap_or_default();
        eprintln!(
            "llm {} ms | {:?} {} | {}",
            r.timings.llm_ms,
            r.status,
            r.fallback_reason.as_deref().unwrap_or(""),
            info["last"]
        );
        out.push(if a.json {
            serde_json::to_string(&serde_json::json!({"result": r, "llm": info["last"]})).unwrap()
        } else {
            r.final_text
        });
    }
    Ok(out.join("\n"))
}

#[cfg(not(feature = "llm"))]
fn run_clean(_a: Args) -> Result<String, String> {
    Err("flow-cli was built without the `llm` feature".into())
}

/// `meeting`: stream the WAV through `MeetingTranscriber` over a router.
fn run_meeting(a: Args, router_cfg: Option<flow_router::RouterConfig>) -> Result<String, String> {
    use flow_router::meeting::{transcribe_stream, MeetingParams, MeetingTranscriber};
    let t = Instant::now();
    let core = FlowCore::load_models(&a.model).map_err(|e| e.to_string())?;
    let load_ms = t.elapsed().as_millis();
    let local: Arc<dyn flow_router::Backend> = Arc::new(core);
    let router = Arc::new(make_router(router_cfg, local)?);
    let blocks = audio::WavBlocks::open(&a.wav, 16_000)?;
    let total = blocks.total_samples();
    let mut job = MeetingTranscriber::new(router, &a.session, MeetingParams::default());
    let mut read_err = None;
    let source = blocks.map_while(|b| b.map_err(|e| read_err = Some(e)).ok());
    let t = Instant::now();
    let json = a.json;
    let mut last_pct = -1i64;
    let tr = transcribe_stream(
        &mut job,
        source,
        Some(total),
        |s| {
            if json {
                println!("{}", serde_json::to_string(s).unwrap());
            } else {
                let text = s
                    .error
                    .as_deref()
                    .map_or(s.text.clone(), |e| format!("[error: {e}]"));
                println!(
                    "[{} --> {}] {text}",
                    flow_router::meeting::clock(s.start_ms),
                    flow_router::meeting::clock(s.end_ms)
                );
            }
        },
        |p| {
            let pct = p.percent().unwrap_or(0.0) as i64;
            if a.progress && pct != last_pct {
                last_pct = pct;
                eprintln!("progress {pct}% ({} segments)", p.segments);
            }
        },
    )
    .map_err(|e| e.to_string())?;
    if let Some(e) = read_err {
        return Err(e);
    }
    let wall = t.elapsed().as_secs_f64();
    let audio_s = tr.duration_ms as f64 / 1000.0;
    eprintln!(
        "load {load_ms} ms | audio {audio_s:.1} s | wall {wall:.1} s | rtf {:.3} | {} segments",
        wall / audio_s.max(0.001),
        tr.segments.len()
    );
    Ok(String::new())
}

#[cfg(feature = "remote")]
fn make_router(
    cfg: Option<flow_router::RouterConfig>,
    local: Arc<dyn flow_router::Backend>,
) -> Result<flow_router::Router, String> {
    let cfg = cfg.unwrap_or_else(local_only);
    cfg.validate().map_err(|e| e.to_string())?;
    Ok(flow_router::Router::new(
        cfg,
        Some(local),
        Some(flow_remote::factory("flow-cli")),
    ))
}

#[cfg(not(feature = "remote"))]
fn make_router(
    cfg: Option<flow_router::RouterConfig>,
    local: Arc<dyn flow_router::Backend>,
) -> Result<flow_router::Router, String> {
    if cfg.as_ref().is_some_and(|c| c.server.is_configured()) {
        return Err("--router with a server needs flow-cli built with the `remote` feature".into());
    }
    let cfg = cfg.unwrap_or_else(local_only);
    cfg.validate().map_err(|e| e.to_string())?;
    Ok(flow_router::Router::new(cfg, Some(local), None))
}

fn local_only() -> flow_router::RouterConfig {
    flow_router::RouterConfig {
        allow_server: false,
        prefer: flow_router::Prefer::Client,
        ..Default::default()
    }
}

fn run(mut a: Args) -> Result<String, String> {
    if !a.clean.is_empty() {
        return run_clean(a);
    }
    if let Some(cfg) = a.meeting.take() {
        return run_meeting(a, cfg);
    }
    let pcm = audio::read_wav_16k_mono(&a.wav)?;
    let t = Instant::now();
    let core = FlowCore::load_models(&a.model).map_err(|e| e.to_string())?;
    let load_ms = t.elapsed().as_millis();
    // The pushes below outrun the worker's drain (no mic pacing), so a ring
    // smaller than the clip overflows and drops the tail: hold it all.
    a.session.fit_ring_to(pcm.len());
    core.start_session(a.session).map_err(|e| e.to_string())?;
    if let Some(c) = a.context {
        core.set_context(c);
    }
    for c in pcm.chunks(640) {
        core.push_audio(c);
        if a.realtime {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    let r = core.commit();
    eprintln!(
        "load {load_ms} ms | audio {} ms | asr {} ms | llm {} ms | post {} ms | commit {} ms | {:?}",
        r.timings.audio_ms, r.timings.asr_ms, r.timings.llm_ms, r.timings.postprocess_ms, r.timings.total_ms, r.status
    );
    if let Some(info) = core.llm_info() {
        let v: serde_json::Value = serde_json::from_str(&info).unwrap_or_default();
        eprintln!("llm last {} | ahead {}", v["last"], v["ahead"]);
    }
    if a.json {
        Ok(serde_json::to_string_pretty(&r).unwrap())
    } else {
        Ok(r.final_text)
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse(&argv).and_then(run) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
