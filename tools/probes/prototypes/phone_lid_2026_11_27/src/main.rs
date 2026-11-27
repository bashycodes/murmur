//! Phone language-ID probe (2026-11-27): what D34's `WhisperLid` costs on an
//! Android phone's CPU.
//!
//! Usage: `phone-lid-probe <lid-model> [--wav f.wav] [--threads N]
//!         [--iters N] [--ballast-mb N] [--read <file>]`
//!
//! Prints, one `key=value` per line:
//! - `rss_*` / `hwm_*`: VmRSS and VmHWM (peak) in MB, before and after the
//!   model load and after the runs;
//! - `load_ms`: `WhisperLid::load` with the shrunk window (D34), warm-up
//!   included, as the core loads it;
//! - `lid_ms`: each `scores` call on the first 3 s (`LID_SAMPLES`);
//! - `--ballast-mb`: allocates and touches N MB before loading, standing in
//!   for a resident ASR model (Parakeet) to see whether both fit;
//! - `--read`: times a sequential read of a file (MB/s), to estimate the
//!   load time of a bigger model from its size.
//!
//! Audio: a WAV (16 kHz mono) when given, else 3 s of synthetic voiced
//! signal (harmonics + noise). LID latency does not depend on content: the
//! encoder always runs on the same shrunk window.

use std::time::Instant;

use murmur_core::lid::{LanguageId, WhisperLid, LID_SAMPLES};

fn status_mb(key: &str) -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(-1.0)
}

fn mem_available_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("MemAvailable:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(-1.0)
}

fn mem(tag: &str) {
    println!(
        "rss_{tag}={:.1} hwm_{tag}={:.1} avail_{tag}={:.0}",
        status_mb("VmRSS:"),
        status_mb("VmHWM:"),
        mem_available_mb()
    );
}

fn synthetic() -> Vec<f32> {
    let mut x: u32 = 12345;
    (0..LID_SAMPLES)
        .map(|i| {
            let t = i as f32 / 16000.0;
            let f0 = 120.0 + 20.0 * (t * 3.0).sin();
            let v: f32 = (1..8).map(|h| (t * f0 * h as f32 * std::f32::consts::TAU).sin() / h as f32).sum();
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            0.1 * v + 0.01 * n
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model = args.get(1).expect("usage: phone-lid-probe <lid-model> [opts]").clone();
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let threads: usize = opt("--threads").map(|v| v.parse().unwrap()).unwrap_or(4);
    let iters: usize = opt("--iters").map(|v| v.parse().unwrap()).unwrap_or(10);
    let ballast_mb: usize = opt("--ballast-mb").map(|v| v.parse().unwrap()).unwrap_or(0);

    println!("threads={threads} iters={iters} ballast_mb={ballast_mb}");
    mem("start");

    if let Some(f) = opt("--read") {
        let t = Instant::now();
        let n = std::fs::read(&f).map(|b| b.len()).unwrap_or(0);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("read_bytes={n} read_ms={ms:.1} read_mb_s={:.0}", n as f64 / 1048576.0 / (ms / 1000.0));
    }

    // Stand-in for a resident ASR model: touched pages count in RSS.
    let mut ballast = vec![0u8; ballast_mb << 20];
    for i in (0..ballast.len()).step_by(4096) {
        ballast[i] = 1;
    }
    mem("ballast");

    let pcm: Vec<f32> = match opt("--wav") {
        Some(p) => murmur_core::audio::read_wav_16k_mono(std::path::Path::new(&p))
            .expect("wav")
            .into_iter()
            .take(LID_SAMPLES)
            .map(|s| s as f32 / 32768.0)
            .collect(),
        None => synthetic(),
    };
    println!("pcm_samples={}", pcm.len());

    let t = Instant::now();
    let lid = WhisperLid::load(&model, threads, true).expect("load");
    println!("load_ms={:.1}", t.elapsed().as_secs_f64() * 1000.0);
    mem("loaded");

    let cands = vec!["en".to_string(), "he".to_string()];
    let mut ms = Vec::new();
    for i in 0..iters {
        let t = Instant::now();
        let sc = lid.scores(&pcm, &cands).expect("scores");
        let e = t.elapsed().as_secs_f64() * 1000.0;
        ms.push(e);
        println!("lid_ms[{i}]={e:.1} scores={sc:?}");
    }
    let first = ms[0];
    let mut rest = ms[1..].to_vec();
    rest.sort_by(|a, b| a.total_cmp(b));
    if !rest.is_empty() {
        let mean = rest.iter().sum::<f64>() / rest.len() as f64;
        let p90 = rest[((rest.len() as f64 * 0.9).ceil() as usize).min(rest.len()) - 1];
        println!("lid_first_ms={first:.1} lid_warm_mean_ms={mean:.1} lid_warm_p90_ms={p90:.1}");
    }
    mem("after");
    std::hint::black_box(&ballast);
}
