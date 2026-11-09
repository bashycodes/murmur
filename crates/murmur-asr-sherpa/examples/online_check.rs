//! Decode wav files with the online (streaming zipformer) engine.
//!
//! cargo run --release -p flow-asr-sherpa --example online_check -- \
//!   MODEL_DIR [--threads N] [--greedy] [--serial] [--beam N] [--score F]
//!   [--hotwords a,b] [--tail S] [--lead S] [--skip S] [--norm] WAV...
//! Prints `file \t ms \t RTF \t text` per clip; load time and its phases
//! on stderr (`--serial`: create the beam and greedy sessions one after the
//! other, as before 2026-11-05).

use std::time::Instant;

use flow_asr_sherpa::{OnlineConfig, OnlineEngine};

fn read_wav(p: &str) -> Vec<f32> {
    let mut r = hound::WavReader::open(p).unwrap();
    assert_eq!(r.spec().sample_rate, 16_000);
    match r.spec().sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
    }
}

/// Scale to peak 0.5: some FLEURS he clips are recorded at RMS < 0.001,
/// below the engines' silence gate.
fn peak_normalize(pcm: &mut [f32]) {
    let peak = pcm.iter().fold(0f32, |m, x| m.max(x.abs()));
    if peak > 0.0 {
        pcm.iter_mut().for_each(|x| *x *= 0.5 / peak);
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = OnlineConfig::new(&a[0], 4, Some("he"));
    let mut hotwords: Vec<String> = Vec::new();
    let mut wavs = Vec::new();
    let mut norm = false;
    let mut lead = 0f32;
    let mut skip = 0f32;
    let mut i = 1;
    while i < a.len() {
        match a[i].as_str() {
            "--threads" => {
                i += 1;
                cfg.threads = a[i].parse().unwrap()
            }
            "--greedy" => cfg.hotwords = false,
            "--serial" => cfg.parallel_sessions = false,
            "--tail" => {
                i += 1;
                cfg.tail_padding_s = a[i].parse().unwrap()
            }
            "--score" => {
                i += 1;
                cfg.hotwords_score = a[i].parse().unwrap()
            }
            "--beam" => {
                i += 1;
                cfg.max_active_paths = a[i].parse().unwrap()
            }
            "--lead" => {
                i += 1;
                lead = a[i].parse().unwrap()
            }
            "--skip" => {
                i += 1;
                skip = a[i].parse().unwrap()
            }
            "--norm" => norm = true,
            "--hotwords" => {
                i += 1;
                hotwords = a[i].split(',').map(str::to_string).collect()
            }
            w => wavs.push(w.to_string()),
        }
        i += 1;
    }
    let t = Instant::now();
    let eng = OnlineEngine::load(&cfg).expect("load");
    eprintln!("load: {} ms", t.elapsed().as_millis());
    for p in eng.load_phases() {
        eprintln!("  {}: {} ms", p.name, p.ms);
    }
    for w in wavs {
        let mut pcm = read_wav(&w);
        if norm {
            peak_normalize(&mut pcm);
        }
        // --skip drops the first S seconds, --lead prepends S seconds of silence.
        let n = ((skip * 16_000.0) as usize).min(pcm.len());
        pcm.drain(..n);
        let pcm = [vec![0.0; (lead * 16_000.0) as usize], pcm].concat();
        let t = Instant::now();
        let text = eng.transcribe(&pcm, &hotwords).unwrap();
        let s = t.elapsed().as_secs_f64();
        let name = std::path::Path::new(&w)
            .file_name()
            .unwrap()
            .to_string_lossy();
        println!(
            "{name}\t{:.0}\t{:.3}\t{text}",
            s * 1e3,
            s * 16_000.0 / pcm.len() as f64
        );
    }
}
