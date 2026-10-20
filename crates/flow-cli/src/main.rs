//! flow-cli: dev harness that drives the full flow-core pipeline on a WAV.
//!
//! The audio is fed through `push_audio` in 40 ms chunks exactly like a
//! shell would, so the reported `total_ms` is release-to-text latency.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use flow_core::config::{AsrBackend, FieldContext, ModelConfig, Platform, SessionConfig};
use flow_core::dictionary::Dictionary;
use flow_core::{audio, FlowCore};

const USAGE: &str =
    "usage: flow-cli transcribe (--model ggml.bin | --mock TEXT) [--dict dict.json] \
[--session session.json] [--context ctx.json] [--lang xx]... [--platform windows|android] \
[--threads N] [--realtime] [--json] file.wav";

struct Args {
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
    if it.next().map(String::as_str) != Some("transcribe") {
        return Err(USAGE.into());
    }
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
            "--json" => json = true,
            "--realtime" => realtime = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s => wav = Some(PathBuf::from(s)),
        }
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
    let wav = wav.ok_or_else(|| USAGE.to_string())?;
    if !have_model {
        return Err(USAGE.into());
    }
    Ok(Args {
        model,
        session,
        context,
        wav,
        json,
        realtime,
    })
}

fn run(a: Args) -> Result<String, String> {
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
