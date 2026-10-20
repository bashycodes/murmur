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
    "usage: flow-cli transcribe (--model ggml.bin | --mock TEXT) [--dict dict.json] \
[--session session.json] [--context ctx.json] [--lang xx]... [--platform windows|android] \
[--threads N] [--llm model.gguf [--llm-lib libflow_llm.so]] [--level none|light|medium|high] \
[--realtime] [--json] file.wav
       flow-cli clean --llm model.gguf [same options, no wav/model] [--category email|work_chat|...] TEXT...
         (each TEXT runs as one dictation through LLM cleanup + post-processing; prints llm counters)";

struct Args {
    clean: Vec<String>,
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
    let clean_mode = match it.next().map(String::as_str) {
        Some("transcribe") => false,
        Some("clean") => true,
        _ => return Err(USAGE.into()),
    };
    let mut texts = Vec::new();
    let mut level = None;
    let mut category = None;
    let mut model = ModelConfig::default();
    let mut session = SessionConfig::default();
    let mut dict: Option<Dictionary> = None;
    let mut langs = Vec::new();
    let mut platform = None;
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
            "--json" => json = true,
            "--realtime" => realtime = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s if clean_mode => texts.push(s.to_string()),
            s => wav = Some(PathBuf::from(s)),
        }
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

fn run(a: Args) -> Result<String, String> {
    if !a.clean.is_empty() {
        return run_clean(a);
    }
    let pcm = audio::read_wav_16k_mono(&a.wav)?;
    let t = Instant::now();
    let core = FlowCore::load_models(&a.model).map_err(|e| e.to_string())?;
    let load_ms = t.elapsed().as_millis();
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
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
