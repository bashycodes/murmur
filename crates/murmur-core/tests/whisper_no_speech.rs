//! D91: Whisper's no-speech probability must separate noise from speech on
//! multilingual models too.
//!
//! whisper.cpp 1.8.3 reads `<|nospeech|>` from a stale logits row unless
//! the prompt is `sot` alone, so on multilingual models (prompt `sot`,
//! language, `transcribe`) and on any model given a dictionary prompt it
//! reported about 0 on every decode. `WhisperEngine` now reads it at `sot`
//! itself (`docs/findings/2026-11-13-whisper-no-speech-multilingual.md`).
//!
//! Needs `models/ggml-tiny.bin`, `models/ggml-tiny.en.bin` and
//! `models/jfk.wav` (`tools/fetch-models.sh`); skips when one is missing.
//! Audio: the public JFK clip plus synthetic noise only.
//!
//! The ignored `no_speech_before_after` prints whisper.cpp's own value next
//! to the engine's for each model in `MURMUR_BENCH_MODELS`:
//!   cargo test --release -p murmur-core --features whisper,wav \
//!     --test whisper_no_speech -- --ignored --nocapture

use std::path::PathBuf;

use murmur_core::asr::{AsrEngine, AsrRequest};
use murmur_core::audio;
use murmur_core::hallucination::{whisper_says_no_speech, NO_SPEECH_THOLD};
use murmur_core::whisper::{audio_ctx_for, full_params, pad, WhisperEngine};

fn models_dir() -> PathBuf {
    std::env::var_os("MURMUR_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn fixture(model: &str) -> Option<(PathBuf, Vec<f32>)> {
    let d = models_dir();
    let (m, wav) = (d.join(model), d.join("jfk.wav"));
    if !m.exists() || !wav.exists() {
        eprintln!("SKIP: {} or {} missing", m.display(), wav.display());
        return None;
    }
    let jfk = audio::read_wav_16k_mono(&wav)
        .unwrap()
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect();
    Some((m, jfk))
}

/// xorshift white noise, peak `amp`.
fn white(n: usize, amp: f32, seed: u32) -> Vec<f32> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

/// Leaky-integrated white noise (rumble), same RMS as `white(n, amp, _)`.
fn brown(n: usize, amp: f32, seed: u32) -> Vec<f32> {
    let mut y = 0.0f32;
    let v: Vec<f32> = white(n, 1.0, seed)
        .iter()
        .map(|s| {
            y = 0.98 * y + s;
            y
        })
        .collect();
    let gain = rms(&white(n, amp, seed)) / rms(&v).max(1e-9);
    v.iter().map(|s| s * gain).collect()
}

fn rms(p: &[f32]) -> f32 {
    (p.iter().map(|s| s * s).sum::<f32>() / p.len().max(1) as f32).sqrt()
}

/// Noise windows above the engine's silence gate: white and brown, three
/// levels, 1.5 / 3 / 6 s, one seed (the tails bench of D88, halved).
fn noise_windows() -> Vec<(String, Vec<f32>)> {
    let mut v = Vec::new();
    for (kind, gen) in [
        ("white", white as fn(usize, f32, u32) -> Vec<f32>),
        ("brown", brown),
    ] {
        for amp in [0.006f32, 0.012, 0.024] {
            for secs in [1.5f32, 3.0, 6.0] {
                v.push((
                    format!("{kind}-{amp}-{secs}s"),
                    gen((secs * 16_000.0) as usize, amp, 1),
                ));
            }
        }
    }
    v
}

fn en(prompt: &str) -> AsrRequest {
    AsrRequest {
        language: Some("en".into()),
        initial_prompt: prompt.into(),
        ..Default::default()
    }
}

/// A made-up dictionary prompt, as `compose_prompt` builds one.
const PROMPT: &str = "Glossary: Murmur, Kotlin, Tauri, whisper.cpp.";

/// The engine's `(no_speech_prob, avg_logprob)` on the noise windows that
/// produced text (no text = no value), and on JFK.
fn probe(eng: &WhisperEngine, prompt: &str, jfk: &[f32]) -> (Vec<(f32, f32)>, (f32, f32)) {
    let pair = |pcm: &[f32]| {
        let o = eng.transcribe(pcm, &en(prompt)).unwrap();
        Some((
            o.no_speech_prob?,
            o.avg_logprob.expect("paired with no_speech_prob"),
        ))
    };
    let noise = noise_windows()
        .iter()
        .filter_map(|(_, pcm)| pair(pcm))
        .collect();
    (noise, pair(jfk).expect("JFK decodes to text"))
}

fn check(model: &str, prompt: &str) {
    let Some((m, jfk)) = fixture(model) else {
        return;
    };
    let eng = WhisperEngine::load(m.to_str().unwrap(), 4)
        .unwrap()
        .with_fit_ctx(true);
    let (noise, (speech, speech_lp)) = probe(&eng, prompt, &jfk);
    let caught = noise
        .iter()
        .filter(|&&(p, l)| whisper_says_no_speech(Some(p), Some(l)))
        .count();
    eprintln!(
        "{model} prompt={:?}: jfk ({speech:.3}, {speech_lp:.2}), noise {noise:.2?}, \
         no speech by OpenAI's rule on {caught}/{}",
        !prompt.is_empty(),
        noise.len()
    );
    assert!(!noise.is_empty(), "{model}: no noise window gave text");
    assert!(speech < 0.15, "{model}: speech read as no speech: {speech}");
    assert!(!whisper_says_no_speech(Some(speech), Some(speech_lp)));
    // Before D91 every noise value here was below 0.003 (tiny: 0.000).
    // After: tiny 0.33-0.76, tiny.en with a prompt 0.87-0.96.
    let min = noise.iter().map(|w| w.0).fold(1.0f32, f32::min);
    let max = noise.iter().map(|w| w.0).fold(0.0f32, f32::max);
    assert!(min > 0.25, "{model}: noise read as speech: {noise:?}");
    assert!(
        max > NO_SPEECH_THOLD,
        "{model}: no noise window passes {NO_SPEECH_THOLD}: {noise:?}"
    );
}

#[test]
fn multilingual_model_flags_noise_as_no_speech() {
    check("ggml-tiny.bin", "");
}

#[test]
fn a_dictionary_prompt_keeps_the_no_speech_signal() {
    check("ggml-tiny.en.bin", PROMPT);
}

/// Median of `n` timings of `f`, in ms.
fn median_ms(n: usize, mut f: impl FnMut()) -> f64 {
    let mut v: Vec<f64> = (0..n)
        .map(|_| {
            let t = std::time::Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    v.sort_by(f64::total_cmp);
    v[n / 2]
}

/// What the D91 probe costs: one `sot` decode per window that gave text,
/// timed alone right after a decode (median of 21), next to a whole
/// whisper.cpp decode without the probe and a whole engine decode with it
/// (median of 5 each).
#[test]
#[ignore]
fn no_speech_probe_latency() {
    let models = std::env::var("MURMUR_BENCH_MODELS")
        .unwrap_or_else(|_| "ggml-tiny.bin,ggml-base.en.bin".into());
    for model in models.split(',') {
        let Some((m, jfk)) = fixture(model) else {
            continue;
        };
        let path = m.to_str().unwrap();
        let ctx = whisper_rs::WhisperContext::new_with_params(path, Default::default()).unwrap();
        let mut st = ctx.create_state().unwrap();
        let eng = WhisperEngine::load(path, 4).unwrap().with_fit_ctx(true);
        let sot = ctx.token_sot();
        for (id, pcm) in [("jfk-11s", jfk.clone()), ("jfk-3s", jfk[..48_000].to_vec())] {
            let without = median_ms(5, || {
                whisper_cpp_value(&mut st, &pcm, "");
            });
            let probe = median_ms(21, || st.decode(&[sot], 0, 4).unwrap());
            let with = median_ms(5, || {
                eng.transcribe(&pcm, &en("")).unwrap();
            });
            println!(
                "LATENCY {model} {id}: sot decode {probe:.1} ms; \
                 whole decode without probe {without:.0} ms, engine with probe {with:.0} ms"
            );
        }
    }
}

/// whisper.cpp's own per-segment value (the pre-D91 read), with the
/// engine's decode settings.
fn whisper_cpp_value(st: &mut whisper_rs::WhisperState, pcm: &[f32], prompt: &str) -> Option<f32> {
    let audio = pad(pcm);
    let p = full_params(4, "en", audio_ctx_for(audio.len()), prompt);
    st.full(p, &audio).unwrap();
    st.as_iter()
        .map(|s| s.no_speech_probability())
        .reduce(f32::max)
}

/// Risk check for D91: a real, short, quiet "Thank you." or "you" with the
/// VAD off. A no-speech probability ≥ `NO_SPEECH_THOLD` here would make
/// the stock-phrase guard drop real speech. Synthetic speech (espeak-ng,
/// en-us, 16 kHz mono, 0.3 s of silence each side) at 0, -20 and -30 dB
/// in `MURMUR_NOSP_WAVS`, named `thankyou0.wav`, `you-20.wav`, ….
/// Prints the engine's text, value and guard verdict, plus the probe
/// value read directly (also for clips the engine's silence gate skips).
#[test]
#[ignore]
fn quiet_stock_phrase_risk() {
    use murmur_core::hallucination::{is_hallucination, Evidence};
    let dir = PathBuf::from(std::env::var("MURMUR_NOSP_WAVS").expect("MURMUR_NOSP_WAVS"));
    let models = std::env::var("MURMUR_BENCH_MODELS")
        .unwrap_or_else(|_| "ggml-tiny.bin,ggml-base.en.bin".into());
    for model in models.split(',') {
        let Some((m, _)) = fixture(model) else {
            continue;
        };
        let path = m.to_str().unwrap();
        let ctx = whisper_rs::WhisperContext::new_with_params(path, Default::default()).unwrap();
        let mut st = ctx.create_state().unwrap();
        let eng = WhisperEngine::load(path, 4).unwrap().with_fit_ctx(true);
        for word in ["thankyou", "you"] {
            for db in ["0", "-20", "-30"] {
                let pcm: Vec<f32> = audio::read_wav_16k_mono(&dir.join(format!("{word}{db}.wav")))
                    .unwrap()
                    .iter()
                    .map(|&s| s as f32 / 32768.0)
                    .collect();
                let o = eng.transcribe(&pcm, &en("")).unwrap();
                let dropped = is_hallucination(
                    &o.text,
                    Evidence {
                        speech_ms: None,
                        no_speech_prob: o.no_speech_prob,
                        avg_logprob: o.avg_logprob,
                    },
                );
                let audio = pad(&pcm);
                st.full(full_params(4, "en", audio_ctx_for(audio.len()), ""), &audio)
                    .unwrap();
                st.decode(&[ctx.token_sot()], 0, 4).unwrap();
                let l = st.get_logits().unwrap();
                let max = l.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let sum: f32 = l.iter().map(|x| (x - max).exp()).sum();
                let direct = (l[ctx.token_nosp() as usize] - max).exp() / sum;
                println!(
                    "RISK {model} {word} {db} dB: text {:?} no_speech {:?} avg_logprob {:?} \
                     dropped {dropped}; probe direct {direct:.3}",
                    o.text, o.no_speech_prob, o.avg_logprob
                );
            }
        }
    }
}

#[test]
#[ignore]
fn no_speech_before_after() {
    let models = std::env::var("MURMUR_BENCH_MODELS")
        .unwrap_or_else(|_| "ggml-tiny.en.bin,ggml-tiny.bin".into());
    for model in models.split(',') {
        let Some((m, jfk)) = fixture(model) else {
            continue;
        };
        let path = m.to_str().unwrap();
        let ctx = whisper_rs::WhisperContext::new_with_params(path, Default::default()).unwrap();
        let mut st = ctx.create_state().unwrap();
        let eng = WhisperEngine::load(path, 4).unwrap().with_fit_ctx(true);
        for prompt in ["", PROMPT] {
            let mut rows = noise_windows();
            rows.push(("jfk".into(), jfk.clone()));
            let (mut before, mut after, mut caught) = (Vec::new(), Vec::new(), 0);
            for (id, pcm) in &rows {
                let b = whisper_cpp_value(&mut st, pcm, prompt);
                let o = eng.transcribe(pcm, &en(prompt)).unwrap();
                let (a, lp) = (o.no_speech_prob, o.avg_logprob);
                println!(
                    "{model}\tprompt={}\t{id}\tbefore={b:?}\tafter={a:?}\tavg_logprob={lp:?}\ttext={:?}",
                    !prompt.is_empty(),
                    o.text
                );
                if id != "jfk" {
                    before.extend(b);
                    after.extend(a);
                    caught += usize::from(whisper_says_no_speech(a, lp));
                }
            }
            let stats = |v: &[f32]| {
                let n = v.iter().filter(|&&p| p >= NO_SPEECH_THOLD).count();
                let max = v.iter().copied().fold(0.0f32, f32::max);
                format!("max {max:.3}, ≥{NO_SPEECH_THOLD} on {n}/{}", v.len())
            };
            println!(
                "SUMMARY {model} prompt={}: noise before [{}] after [{}]; \
                 after, no speech by OpenAI's rule on {caught}/{}",
                !prompt.is_empty(),
                stats(&before),
                stats(&after),
                after.len()
            );
        }
    }
}
