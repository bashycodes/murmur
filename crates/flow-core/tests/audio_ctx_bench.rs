//! Whisper `audio_ctx` (encoder window sized to the clip) vs the full 30 s
//! window on the 40-clip LibriSpeech set of the ASR benchmark (ignored by
//! default; minutes per model). Run:
//!   FLOW_BENCH_AUDIO=~/…/.cache/audio FLOW_BENCH_OUT=tools/probes/results/x.jsonl \
//!   cargo test --release -p flow-core --features whisper,wav --test audio_ctx_bench -- --ignored --nocapture
//! Env: FLOW_MODELS_DIR (models), FLOW_BENCH_MODELS="ggml-tiny.en.bin,…".
//! Writes one JSON line per (model, mode, clip) with the hypothesis and
//! the ASR ms; score with `tools/probes/audio_ctx_wer.py`.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use flow_core::asr::{AsrEngine, AsrRequest};
use flow_core::whisper::WhisperEngine;

#[test]
#[ignore]
fn audio_ctx_vs_full_window() {
    let models_dir = PathBuf::from(std::env::var("FLOW_MODELS_DIR").expect("FLOW_MODELS_DIR"));
    let audio_dir = PathBuf::from(std::env::var("FLOW_BENCH_AUDIO").expect("FLOW_BENCH_AUDIO"));
    let out_path = std::env::var("FLOW_BENCH_OUT").expect("FLOW_BENCH_OUT");
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tools/probes/bench/asr_manifest_extended.json"
    ))
    .unwrap();
    let models = std::env::var("FLOW_BENCH_MODELS").unwrap_or_else(|_| "ggml-tiny.en.bin".into());
    let mut out = std::fs::File::create(&out_path).unwrap();
    for m in models.split(',') {
        let path = models_dir.join(m);
        for fit in [false, true] {
            let eng = WhisperEngine::load(path.to_str().unwrap(), 4)
                .unwrap()
                .with_fit_ctx(fit);
            let mut total_ms = 0u128;
            for u in manifest["utterances"].as_array().unwrap() {
                let id = u["id"].as_str().unwrap();
                let pcm: Vec<f32> =
                    flow_core::audio::read_wav_16k_mono(&audio_dir.join(format!("{id}.wav")))
                        .unwrap()
                        .iter()
                        .map(|&s| s as f32 / 32768.0)
                        .collect();
                let req = AsrRequest {
                    language: Some("en".into()),
                    ..Default::default()
                };
                let t = Instant::now();
                let hyp = eng.transcribe(&pcm, &req).unwrap().text;
                let ms = t.elapsed().as_millis();
                total_ms += ms;
                let line = serde_json::json!({"model": m, "fit_ctx": fit, "id": id,
                    "dur_s": u["dur_s"], "ref": u["ref"], "hyp": hyp, "asr_ms": ms});
                writeln!(out, "{line}").unwrap();
            }
            eprintln!(
                "{m} fit_ctx={fit}: total ASR {total_ms} ms, full-window re-decodes {}",
                eng.redecodes()
            );
        }
    }
}
