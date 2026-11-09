//! H6 / D34: whisper.cpp language ID on real he/en speech. Env-gated:
//! skips (passes with a note) unless `MURMUR_LID_MODEL` (a multilingual ggml
//! model, e.g. `ggml-tiny.bin`), `MURMUR_LID_HE_DIR` and `MURMUR_LID_EN_DIR`
//! (16 kHz mono WAVs: FLEURS he_il dev, LibriSpeech) are set. Audio and
//! models are never committed.
//!
//! `cargo test -p murmur-core --features whisper,wav --release --test lid_e2e -- --nocapture`
//! prints accuracy and latency for the full and the shrunk encoder window.

use std::path::Path;
use std::time::Instant;

use murmur_core::lid::{choose, LanguageId, WhisperLid, LID_SAMPLES, ROUTE_MIN_PROB};
use murmur_core::{audio, vad};

/// White noise at `MURMUR_LID_SNR` dB below the clip's RMS (unset = clean),
/// deterministic (LCG), like the 2026-10-23 phone-noise probe.
fn noisy(mut pcm: Vec<f32>) -> Vec<f32> {
    let Some(snr) = std::env::var("MURMUR_LID_SNR")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    else {
        return pcm;
    };
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
    let amp = rms / 10f32.powf(snr / 20.0) * 3f32.sqrt();
    let mut x: u32 = 12345;
    for s in pcm.iter_mut() {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        *s += amp * ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0);
    }
    pcm
}

fn clips(dir: &str) -> Vec<Vec<f32>> {
    let mut ps: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    ps.sort();
    ps.iter()
        .map(|p| {
            audio::read_wav_16k_mono(Path::new(p))
                .unwrap()
                .iter()
                .map(|&s| s as f32 / 32768.0)
                .collect()
        })
        .map(noisy)
        .collect()
}

/// What the core hands the identifier: speech start + `secs`.
fn head(pcm: &[f32], secs: f32) -> &[f32] {
    let start = vad::detect(pcm).map_or(0, |s| s.start);
    let n = ((secs * 16000.0) as usize).min(LID_SAMPLES);
    &pcm[start..(start + n).min(pcm.len())]
}

#[test]
fn whisper_lid_he_vs_en() {
    let (Ok(model), Ok(he), Ok(en)) = (
        std::env::var("MURMUR_LID_MODEL"),
        std::env::var("MURMUR_LID_HE_DIR"),
        std::env::var("MURMUR_LID_EN_DIR"),
    ) else {
        eprintln!("SKIP: set MURMUR_LID_MODEL, MURMUR_LID_HE_DIR, MURMUR_LID_EN_DIR");
        return;
    };
    let threads = std::env::var("MURMUR_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4);
    let (he, en) = (clips(&he), clips(&en));
    let cands = vec!["en".to_string(), "he".to_string()];
    let fits: Vec<bool> = match std::env::var("MURMUR_LID_FIT").as_deref() {
        Ok("1") => vec![true],
        Ok("0") => vec![false],
        _ => vec![true, false],
    };
    for model in model.split(',') {
        for &fit in &fits {
            let t = Instant::now();
            let lid = WhisperLid::load(model, threads, fit).unwrap();
            let load_ms = t.elapsed().as_millis();
            for secs in [2.0f32, 3.0] {
                let mut ms = Vec::new();
                // p(he) per clip, per set.
                let mut phe = [Vec::new(), Vec::new()];
                let mut routed_ok = 0;
                for (k, set) in [&he, &en].into_iter().enumerate() {
                    for pcm in set.iter() {
                        let t = Instant::now();
                        let sc = lid.scores(head(pcm, secs), &cands).unwrap();
                        ms.push(t.elapsed().as_secs_f64() * 1000.0);
                        phe[k].push(sc.iter().find(|(l, _)| l == "he").unwrap().1);
                        if choose(&sc, "en").is_some() == (k == 0) {
                            routed_ok += 1;
                        }
                    }
                }
                ms.sort_by(f64::total_cmp);
                let mean = ms.iter().sum::<f64>() / ms.len() as f64;
                let name = Path::new(model).file_name().unwrap().to_string_lossy();
                println!(
                    "{name} fit_ctx={fit} head={secs}s load={load_ms}ms | latency mean {mean:.0} ms \
                     p50 {:.0} p90 {:.0} max {:.0}",
                    ms[ms.len() / 2],
                    ms[ms.len() * 9 / 10],
                    ms[ms.len() - 1],
                );
                let max_en = phe[1].iter().copied().fold(0f32, f32::max);
                let mut hs = phe[0].clone();
                hs.sort_by(f32::total_cmp);
                println!(
                    "  max p(he) on English {max_en:.3}; lowest p(he) on Hebrew {:.3?}",
                    &hs[..5]
                );
                for th in [0.5f32, 0.3, 0.2, 0.1, 0.05] {
                    let he_ok = phe[0].iter().filter(|&&p| p >= th).count();
                    let en_ok = phe[1].iter().filter(|&&p| p < th).count();
                    println!(
                        "  route he when p(he) >= {th}: he {he_ok}/{} en {en_ok}/{}",
                        he.len(),
                        en.len()
                    );
                }
                let acc = routed_ok as f32 / (he.len() + en.len()) as f32;
                println!(
                    "  core routing (p >= {ROUTE_MIN_PROB}): {routed_ok}/{} correct",
                    he.len() + en.len()
                );
                if secs == 3.0 {
                    assert!(acc >= 0.75, "{name}: accuracy {acc}");
                }
            }
        }
    }
}

/// The whole path: `ModelConfig.lid_path` + a language map, real speech,
/// mock decoders (the text says which engine decoded).
#[test]
fn flowcore_routes_real_speech_by_language() {
    use std::collections::BTreeMap;

    use murmur_core::config::{AsrBackend, LanguageModel, ModelConfig, SessionConfig};
    use murmur_core::MurmurCore;

    let (Ok(model), Ok(he), Ok(en)) = (
        std::env::var("MURMUR_LID_MODEL"),
        std::env::var("MURMUR_LID_HE_DIR"),
        std::env::var("MURMUR_LID_EN_DIR"),
    ) else {
        eprintln!("SKIP: set MURMUR_LID_MODEL, MURMUR_LID_HE_DIR, MURMUR_LID_EN_DIR");
        return;
    };
    let core = MurmurCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Mock {
            text: "english".into(),
        },
        languages: BTreeMap::from([(
            "he".to_string(),
            LanguageModel {
                asr_backend: AsrBackend::Mock {
                    text: "עברית".into(),
                },
                ..Default::default()
            },
        )]),
        lid_path: Some(model.split(',').next().unwrap().into()),
        ..Default::default()
    })
    .unwrap();
    let mut ok = 0;
    let mut n = 0;
    for (dir, want) in [(&he, "עברית"), (&en, "english")] {
        for pcm in clips(dir).iter().take(8) {
            core.start_session(SessionConfig::default()).unwrap();
            let pcm: Vec<i16> = pcm.iter().map(|&s| (s * 32767.0) as i16).collect();
            for c in pcm.chunks(640) {
                core.push_audio(c);
            }
            let r = core.commit();
            println!(
                "{want}: got {:?} lid_ms {} ({:?} {:?})",
                r.asr_text, r.timings.lid_ms, r.status, r.empty_reason
            );
            // Some FLEURS clips are too quiet for the energy VAD (no_speech,
            // nothing to identify): not a routing outcome.
            if r.empty_reason.as_deref() == Some("no_speech") {
                continue;
            }
            n += 1;
            if r.asr_text.starts_with(want) {
                ok += 1;
            }
        }
    }
    assert!(n >= 8 && ok * 10 >= n * 8, "{ok}/{n}");
}
