//! `murmur-cli meeting`: a long WAV in, a timestamped transcript out (D44).
//!
//! The real-model runs are ignored by default; audio and models are never
//! committed, the recordings are built here from public clips:
//!
//! * English, 30+ min: LibriSpeech test-clean clips of
//!   `tools/probes/bench/asr_manifest_extended.json` (`MURMUR_AUDIO_DIR`, as
//!   in the segment bench), looped with 0.9-3 s gaps of room noise.
//!   `MURMUR_WHISPER_BENCH_MODEL` = a whisper ggml (e.g. ggml-base-q5_1.bin).
//! * Hebrew: FLEURS he_il dev clips (`MURMUR_FLEURS_HE_DIR` = 16 kHz PCM16
//!   WAVs, `MURMUR_FLEURS_HE_TSV` = FLEURS `dev.tsv`), `MURMUR_HE_MODEL` = a
//!   Hebrew whisper ggml (ivrit.ai turbo).
//!
//! MURMUR_AUDIO_DIR=… MURMUR_WHISPER_BENCH_MODEL=… cargo test --release \
//!   -p murmur-cli --test meeting -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const S: usize = 16_000;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_murmur-cli"))
}

fn tmp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn write_wav(path: &Path, pcm: &[i16], rate: u32, channels: u16) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &s in pcm {
        for _ in 0..channels {
            w.write_sample(s).unwrap();
        }
    }
    w.finalize().unwrap();
}

/// Quiet room noise (deterministic; RMS about -75 dBFS, under the VAD's
/// quietest speech bar of -66 dBFS, so a gap never counts as speech).
fn noise(samples: usize, seed: u32) -> Vec<i16> {
    let mut x = seed;
    (0..samples)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((x >> 16) % 21) as i16 - 10
        })
        .collect()
}

fn voiced(samples: usize, rate: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let env = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
            (8000.0 * env * (2.0 * std::f32::consts::PI * 150.0 * t).sin()) as i16
        })
        .collect()
}

#[derive(Debug, serde::Deserialize)]
struct Seg {
    start_ms: u64,
    end_ms: u64,
    text: String,
    #[serde(default)]
    error: Option<String>,
}

fn segments(stdout: &[u8]) -> Vec<Seg> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
        .collect()
}

#[test]
fn mock_meeting_prints_timestamped_segments_and_progress() {
    // 48 kHz stereo: the reader downmixes and resamples while streaming.
    let rate = 48_000;
    let mut pcm = Vec::new();
    let mut starts = Vec::new();
    for i in 0..4 {
        pcm.extend(noise(rate * 2, i));
        starts.push(pcm.len() as u64 * 1000 / rate as u64);
        pcm.extend(voiced(rate * 12, rate));
    }
    pcm.extend(noise(rate, 9));
    let wav = tmp("meeting-mock.wav");
    write_wav(&wav, &pcm, rate as u32, 2);
    let out = bin()
        .args(["meeting", "--mock", "status update", "--json", "--progress"])
        .arg(&wav)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let segs = segments(&out.stdout);
    assert_eq!(segs.len(), 4, "{segs:?}");
    for (s, a) in segs.iter().zip(&starts) {
        assert!(s.start_ms.abs_diff(*a) <= 60, "{s:?} vs {a}");
        assert_eq!(s.text.to_lowercase().trim_end_matches('.'), "status update");
    }
    assert!(err.contains("progress 100%"), "{err}");
    assert!(err.contains("4 segments"), "{err}");

    let plain = bin()
        .args(["meeting", "--mock", "status update"])
        .arg(&wav)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&plain.stdout);
    assert!(text.starts_with("[00:00:02.0"), "{text}");
    assert_eq!(text.lines().count(), 4);
}

#[test]
fn meeting_with_a_server_route_needs_the_remote_feature() {
    if cfg!(feature = "remote") {
        return;
    }
    let wav = tmp("meeting-short.wav");
    write_wav(&wav, &voiced(S, S), 16_000, 1);
    let cfg = tmp("router-server.json");
    std::fs::write(
        &cfg,
        r#"{"server":{"host":"100.64.0.10","token":"dev-token-0123456789abcdef"}}"#,
    )
    .unwrap();
    let out = bin()
        .args(["meeting", "--mock", "x", "--router"])
        .arg(&cfg)
        .arg(&wav)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("remote"));
}

// ---- real models (ignored) ------------------------------------------------

fn norm(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// (word errors, reference words).
fn wer_counts(reference: &str, hyp: &str) -> (usize, usize) {
    let (r, h) = (norm(reference), norm(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(rw != hw))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    (prev[h.len()], r.len())
}

struct Clip {
    start_ms: u64,
    end_ms: u64,
    reference: String,
}

/// Join `utts` (pcm, reference), looping, with 0.9-3 s noise gaps until
/// `min_s` is reached; writes the WAV and returns the clip spans.
fn build(utts: &[(Vec<i16>, String)], min_s: usize, path: &Path) -> Vec<Clip> {
    const GAPS_MS: [usize; 5] = [1200, 2000, 900, 3000, 1500];
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    let mut n = 0usize;
    let mut clips = Vec::new();
    let mut i = 0;
    while n < min_s * S {
        let gap = noise(GAPS_MS[i % GAPS_MS.len()] * S / 1000, i as u32);
        let (pcm, r) = &utts[i % utts.len()];
        for &s in gap.iter().chain(pcm) {
            w.write_sample(s).unwrap();
        }
        let start = n + gap.len();
        n = start + pcm.len();
        clips.push(Clip {
            start_ms: (start * 1000 / S) as u64,
            end_ms: (n * 1000 / S) as u64,
            reference: r.clone(),
        });
        i += 1;
    }
    for s in noise(2 * S, 99) {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
    clips
}

struct Run {
    segs: Vec<Seg>,
    wall: Duration,
    /// Peak resident set (VmHWM), MB.
    peak_mb: u64,
    /// Highest resident set (VmRSS) in the first / second half of the run, MB.
    rss_halves_mb: (u64, u64),
}

/// Run murmur-cli, sampling its memory (Linux `/proc`) every 100 ms.
fn run_cli(args: &[&str], wav: &Path) -> Run {
    let t = Instant::now();
    let child = bin()
        .args(args)
        .arg(wav)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let sampler = std::thread::spawn(move || {
        let field = |s: &str, k: &str| {
            s.lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        };
        let (mut peak, mut rss) = (0u64, Vec::new());
        while let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
            if s.lines()
                .any(|l| l.starts_with("State:") && l.contains('Z'))
            {
                break;
            }
            peak = peak.max(field(&s, "VmHWM:").unwrap_or(0));
            rss.push((Instant::now(), field(&s, "VmRSS:").unwrap_or(0)));
            std::thread::sleep(Duration::from_millis(100));
        }
        (peak, rss)
    });
    let out = child.wait_with_output().unwrap();
    let wall = t.elapsed();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    eprintln!("{}", err.lines().last().unwrap_or(""));
    let (peak, rss) = sampler.join().unwrap();
    let mid = t + wall / 2;
    let half = |second: bool| {
        rss.iter()
            .filter(|(at, _)| (*at >= mid) == second)
            .map(|r| r.1)
            .max()
            .unwrap_or(0)
            / 1024
    };
    Run {
        segs: segments(&out.stdout),
        wall,
        peak_mb: peak / 1024,
        rss_halves_mb: (half(false), half(true)),
    }
}

/// Segments line up with the clips they cover and carry those clips'
/// words; returns the pooled WER of the whole transcript.
///
/// * Every segment starts and ends inside a clip (never in a gap), in
///   order, without overlap; every clip is covered.
/// * Most segments start right at a clip (a cut in a gap); the others
///   start at a pause inside an utterance.
/// * At least `min_recall` of each clip's words occur in the segments
///   whose times overlap it: a transcript shifted by even one clip fails
///   this (insertions such as a repeated sentence do not count here; they
///   count in the pooled WER).
fn check(clips: &[Clip], segs: &[Seg], min_recall: f64) -> f64 {
    assert!(!segs.is_empty());
    let inside = |t: u64, lead: u64, trail: u64| {
        clips
            .iter()
            .any(|c| t + lead >= c.start_ms && t <= c.end_ms + trail)
    };
    let mut covered = vec![false; clips.len()];
    let mut at_clip_start = 0;
    let mut prev_end = 0;
    for s in segs {
        assert!(s.error.is_none(), "{s:?}");
        assert!(s.start_ms >= prev_end && s.end_ms > s.start_ms, "{s:?}");
        prev_end = s.end_ms;
        assert!(inside(s.start_ms, 60, 0), "start in a gap: {s:?}");
        assert!(inside(s.end_ms, 0, 60), "end in a gap: {s:?}");
        let over: Vec<usize> = (0..clips.len())
            .filter(|&i| clips[i].start_ms < s.end_ms && s.start_ms < clips[i].end_ms)
            .collect();
        if over.iter().any(|&i| {
            s.start_ms + 60 >= clips[i].start_ms && s.start_ms <= clips[i].start_ms + 1200
        }) {
            at_clip_start += 1;
        }
        for &i in &over {
            covered[i] = true;
        }
    }
    let lost = covered.iter().filter(|c| !**c).count();
    assert_eq!(lost, 0, "{lost} clips have no segment");
    for c in clips {
        let mut bag: std::collections::HashMap<String, usize> = Default::default();
        let over = segs
            .iter()
            .filter(|s| c.start_ms < s.end_ms && s.start_ms < c.end_ms);
        for s in over.clone() {
            for w in norm(&s.text) {
                *bag.entry(w).or_default() += 1;
            }
        }
        let want = norm(&c.reference);
        let hit = want
            .iter()
            .filter(|w| match bag.get_mut(*w) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    true
                }
                _ => false,
            })
            .count();
        assert!(
            hit as f64 >= min_recall * want.len() as f64,
            "{hit}/{} words of clip {}..{} {:?} in {:?}",
            want.len(),
            c.start_ms,
            c.end_ms,
            c.reference,
            over.collect::<Vec<_>>()
        );
    }
    assert!(
        at_clip_start * 10 >= segs.len() * 8,
        "only {at_clip_start}/{} segments start at a clip",
        segs.len()
    );
    let reference: Vec<&str> = clips.iter().map(|c| c.reference.as_str()).collect();
    let hyp: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
    let (e, n) = wer_counts(&reference.join(" "), &hyp.join(" "));
    e as f64 / n.max(1) as f64
}

fn librispeech() -> Option<Vec<(Vec<i16>, String)>> {
    let dir = PathBuf::from(std::env::var_os("MURMUR_AUDIO_DIR")?);
    let man = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/probes/bench/asr_manifest_extended.json");
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(man).ok()?).ok()?;
    let utts: Vec<_> = m["utterances"]
        .as_array()?
        .iter()
        .filter_map(|u| {
            let p = dir.join(format!("{}.wav", u["id"].as_str()?));
            let pcm = murmur_core::audio::read_wav_16k_mono(&p).ok()?;
            Some((pcm, u["ref"].as_str()?.to_string()))
        })
        .collect();
    (utts.len() >= 10).then_some(utts)
}

fn threads() -> String {
    std::env::var("MURMUR_THREADS").unwrap_or_else(|_| "4".into())
}

#[test]
#[ignore]
fn thirty_minute_english_meeting_end_to_end() {
    let (Some(utts), Some(model)) = (
        librispeech(),
        std::env::var_os("MURMUR_WHISPER_BENCH_MODEL"),
    ) else {
        eprintln!("SKIP: set MURMUR_AUDIO_DIR and MURMUR_WHISPER_BENCH_MODEL");
        return;
    };
    let model = model.to_string_lossy().to_string();
    let th = threads();
    let args = [
        "meeting",
        "--json",
        "--lang",
        "en",
        "--threads",
        &th,
        "--model",
        &model,
    ];
    let wav = tmp("meeting-30min.wav");
    let clips = build(&utts, 30 * 60 + 30, &wav);
    let audio_s = clips.last().unwrap().end_ms as f64 / 1000.0 + 2.0;
    let run = run_cli(&args, &wav);
    let wer = check(&clips, &run.segs, 0.6);
    let (h1, h2) = run.rss_halves_mb;
    eprintln!(
        "MEETING en: audio {:.1} min | wall {:.1} s | rtf {:.3} | peak RSS {} MB \
         (max RSS 1st half {h1} MB, 2nd half {h2} MB) | {} clips, {} segments | WER {:.2}%",
        audio_s / 60.0,
        run.wall.as_secs_f64(),
        run.wall.as_secs_f64() / audio_s,
        run.peak_mb,
        clips.len(),
        run.segs.len(),
        wer * 100.0,
    );
    assert!(audio_s >= 30.0 * 60.0);
    assert!(wer < 0.15, "pooled WER {wer}");
    // Bounded: the second 15 min add (almost) nothing. The recording alone
    // would be 58 MB as PCM16 and 117 MB as f32.
    assert!(h2 <= h1 + 16, "RSS grew: {h1} MB -> {h2} MB");
}

/// `pcm` scaled to `dbfs` RMS. FLEURS clips range from -52 to -8 dBFS;
/// next to each other in one chunk, Whisper drops the quiet one (a
/// -52 dBFS sentence before a -8 dBFS one: only the loud one comes back,
/// in a dictation as well), which is a recording level problem, not a
/// chunking one.
fn level(pcm: &[i16], dbfs: f64) -> Vec<i16> {
    let rms = (pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64)
        .sqrt()
        .max(1.0);
    let gain = 32768.0 * 10f64.powf(dbfs / 20.0) / rms;
    pcm.iter()
        .map(|&s| (s as f64 * gain).clamp(-32768.0, 32767.0) as i16)
        .collect()
}

#[test]
#[ignore]
fn hebrew_meeting_end_to_end() {
    let (Some(dir), Some(tsv), Some(model)) = (
        std::env::var_os("MURMUR_FLEURS_HE_DIR"),
        std::env::var_os("MURMUR_FLEURS_HE_TSV"),
        std::env::var_os("MURMUR_HE_MODEL"),
    ) else {
        eprintln!("SKIP: set MURMUR_FLEURS_HE_DIR, MURMUR_FLEURS_HE_TSV and MURMUR_HE_MODEL");
        return;
    };
    let refs: std::collections::HashMap<String, String> = std::fs::read_to_string(&tsv)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some((f.get(1)?.to_string(), f.get(3)?.to_string()))
        })
        .collect();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| refs.contains_key(n))
        .collect();
    names.sort();
    let utts: Vec<_> = names
        .iter()
        .map(|n| {
            let pcm = murmur_core::audio::read_wav_16k_mono(&Path::new(&dir).join(n)).unwrap();
            (level(&pcm, -20.0), refs[n].clone())
        })
        .collect();
    assert!(utts.len() >= 10, "{} FLEURS clips", utts.len());
    let total: usize = utts.iter().map(|u| u.0.len()).sum();
    let wav = tmp("meeting-he.wav");
    let clips = build(&utts, total / S, &wav);
    let audio_s = clips.last().unwrap().end_ms as f64 / 1000.0 + 2.0;
    let model = model.to_string_lossy().to_string();
    let th = threads();
    let run = run_cli(
        &[
            "meeting",
            "--json",
            "--lang",
            "he",
            "--threads",
            &th,
            "--model",
            &model,
        ],
        &wav,
    );
    let segs = &run.segs;
    let wer = check(&clips, segs, 0.5);
    eprintln!(
        "MEETING he: audio {:.1} min | wall {:.1} s | rtf {:.3} | peak RSS {} MB | \
         {} clips, {} segments | WER {:.2}%",
        audio_s / 60.0,
        run.wall.as_secs_f64(),
        run.wall.as_secs_f64() / audio_s,
        run.peak_mb,
        clips.len(),
        segs.len(),
        wer * 100.0
    );
    assert!(segs
        .iter()
        .all(|s| s.text.chars().any(|c| ('\u{5d0}'..='\u{5ea}').contains(&c))));
    assert!(wer < 0.4, "pooled WER {wer}");
}
