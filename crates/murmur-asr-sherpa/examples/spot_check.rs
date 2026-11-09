//! WER/RTF spot check against docs/model-benchmarks.md (one warm engine).
//!
//! cargo run --release -p murmur-asr-sherpa --example spot_check -- \
//!   MODEL_DIR MANIFEST.json AUDIO_DIR [--threads N] [--greedy] [--hotwords a,b,c]
//! Prints one JSON line per clip ({id, ref, hyp, dur_s, infer_ms}) on stdout
//! and load/RTF totals on stderr; score with tools/probes/parakeet_wer.py.

use std::time::Instant;

use murmur_asr_sherpa::{ParakeetConfig, ParakeetEngine};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, manifest, audio) = (&a[0], &a[1], std::path::Path::new(&a[2]));
    let mut cfg = ParakeetConfig::new(dir, 4);
    let mut hotwords: Vec<String> = Vec::new();
    let mut i = 3;
    while i < a.len() {
        match a[i].as_str() {
            "--threads" => {
                i += 1;
                cfg.threads = a[i].parse().unwrap()
            }
            "--greedy" => cfg.hotwords = false,
            "--score" => {
                i += 1;
                cfg.hotwords_score = a[i].parse().unwrap()
            }
            "--beam" => {
                i += 1;
                cfg.max_active_paths = a[i].parse().unwrap()
            }
            "--hotwords" => {
                i += 1;
                hotwords = a[i].split(',').map(str::to_string).collect()
            }
            o => panic!("unknown option {o}"),
        }
        i += 1;
    }
    let t = Instant::now();
    let eng = ParakeetEngine::load(&cfg).expect("load");
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
    let (mut audio_s, mut infer_s) = (0.0, 0.0);
    for u in m["utterances"].as_array().unwrap() {
        let id = u["id"].as_str().unwrap();
        let mut r = hound::WavReader::open(audio.join(format!("{id}.wav"))).unwrap();
        let pcm: Vec<f32> = r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        let dur = pcm.len() as f64 / 16_000.0;
        let t = Instant::now();
        let hyp = eng.transcribe(&pcm, &hotwords).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        audio_s += dur;
        infer_s += ms / 1e3;
        println!(
            "{}",
            serde_json::json!({"id": id, "ref": u["ref"], "hyp": hyp, "dur_s": dur, "infer_ms": ms})
        );
    }
    eprintln!(
        "load_ms={load_ms:.0} audio_s={audio_s:.1} infer_s={infer_s:.2} RTF={:.3} beam={} threads={}",
        infer_s / audio_s,
        eng.supports_hotwords(),
        cfg.threads
    );
}
