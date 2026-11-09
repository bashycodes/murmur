//! Dated probe (2026-11-04): how much silence still reaches the ASR engine.
//!
//! Runs murmur-core's real code on 16 kHz mono WAVs (dictation clips,
//! meeting tracks) and reports, per clip, what the engine would receive:
//!
//! * `whole`: segmentation off: one `vad::detect` trim of the clip
//!   (engine.rs commit path).
//! * `seg`: D22 on (the default on Windows and Android): `segment::Segmenter`
//!   cuts at 800 ms pauses (min 4 s, soft max 20 s), each piece trimmed by
//!   `vad::detect` (engine.rs `decode_piece`).
//! * `--meeting`: the D44 chunker (`meeting::MeetingTranscriber`, default
//!   params) with a recording mock engine; each chunk is then trimmed by
//!   `vad::detect` as `MurmurCore::commit` does.
//!
//! Inner silence = gaps between speech runs (frames flagged by
//! `vad::frame_flags`, runs of >= MIN_RUN) inside what reaches the engine.
//! Clipped-speech check: the loudest 100 ms window in the trimmed lead /
//! tail compared with the median 100 ms level of the speech runs; within
//! 10 dB of it is flagged `CLIP?`. `--envelope` prints the 100 ms RMS (dBFS)
//! from 1 s before to 1 s after each trim point.
//!
//! cargo run --release -- [--meeting] [--envelope] [--pcm16] FILE...
//! (`--pcm16`: headerless 16 kHz mono s16le, e.g. a `history_audio.pcm`
//! blob.) Labels are the argument's index, never the file name, so output
//! can be pasted anywhere. Never commit the audio.

use std::sync::{Arc, Mutex};

use murmur_core::config::{SessionConfig, SAMPLE_RATE};
use murmur_core::meeting::{ChunkTranscriber, MeetingParams, MeetingTranscriber};
use murmur_core::segment::{SegmentParams, Segmenter, DEFAULT_MIN_MS, DEFAULT_SOFT_MAX_MS, RECOMMENDED_PAUSE_MS};
use murmur_core::vad::{self, FRAME};
use murmur_core::CommitResult;

/// Mirrors `vad::MIN_RUN` (pub(crate)): 60 ms of consecutive speech frames.
const MIN_RUN: usize = 3;
const LONG_GAP_MS: u64 = 1000;
const WIN: usize = SAMPLE_RATE / 10; // 100 ms

fn ms(samples: usize) -> u64 {
    (samples * 1000 / SAMPLE_RATE) as u64
}

fn db(r: f32) -> f32 {
    20.0 * r.max(1e-6).log10()
}

fn rms(p: &[f32]) -> f32 {
    if p.is_empty() {
        return 0.0;
    }
    (p.iter().map(|s| s * s).sum::<f32>() / p.len() as f32).sqrt()
}

fn read(path: &str, raw: bool) -> Vec<f32> {
    if raw {
        let b = std::fs::read(path).unwrap();
        return b
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect();
    }
    let mut r = hound::WavReader::open(path).unwrap();
    let spec = r.spec();
    assert_eq!(spec.sample_rate as usize, SAMPLE_RATE, "arg needs 16 kHz");
    let ch = spec.channels as usize;
    let s: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
    };
    s.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect()
}

/// Speech runs (frame ranges) as `vad::detect` counts them.
fn runs(pcm: &[f32]) -> Vec<(usize, usize)> {
    let f = vad::frame_flags(pcm);
    let mut out = Vec::new();
    let mut i = 0;
    while i < f.len() {
        if !f[i] {
            i += 1;
            continue;
        }
        let j = i + f[i..].iter().take_while(|&&x| x).count();
        if j - i >= MIN_RUN {
            out.push((i, j));
        }
        i = j;
    }
    out
}

/// Silence inside `range` (samples) given the clip's speech runs:
/// (all non-speech ms, ms of gaps longer than LONG_GAP_MS, count of those).
fn inner(runs: &[(usize, usize)], range: (usize, usize)) -> (u64, u64, usize) {
    let inside: Vec<(usize, usize)> = runs
        .iter()
        .map(|&(a, b)| ((a * FRAME).max(range.0), (b * FRAME).min(range.1)))
        .filter(|&(a, b)| b > a)
        .collect();
    let speech: usize = inside.iter().map(|&(a, b)| b - a).sum();
    let mut long_ms = 0;
    let mut n = 0;
    for w in inside.windows(2) {
        let g = ms(w[1].0 - w[0].1);
        if g > LONG_GAP_MS {
            long_ms += g;
            n += 1;
        }
    }
    (ms(range.1 - range.0 - speech), long_ms, n)
}

/// What one trimmed piece sends: Some(kept range) or None (skipped).
fn trim(pcm: &[f32], off: usize) -> Option<(usize, usize)> {
    vad::detect(pcm).map(|sp| (off + sp.start, off + sp.end))
}

fn envelope(pcm: &[f32], at: usize, label: &str) {
    let lo = at.saturating_sub(SAMPLE_RATE) / WIN * WIN;
    let hi = (at + SAMPLE_RATE).min(pcm.len());
    let v: Vec<String> = (lo..hi)
        .step_by(WIN)
        .map(|s| {
            let mark = if s <= at && at < s + WIN { "|" } else { "" };
            format!("{mark}{:.0}", db(rms(&pcm[s..(s + WIN).min(pcm.len())])))
        })
        .collect();
    println!("    env {label} @{}ms: {}", ms(at), v.join(" "));
}

/// Loudest 100 ms window of `pcm[a..b]`, dBFS.
fn peak_win(pcm: &[f32], a: usize, b: usize) -> f32 {
    if b <= a {
        return -120.0;
    }
    pcm[a..b].chunks(WIN).map(|c| db(rms(c))).fold(-120.0, f32::max)
}

fn speech_median(pcm: &[f32], runs: &[(usize, usize)]) -> f32 {
    let mut v: Vec<f32> = runs
        .iter()
        .flat_map(|&(a, b)| pcm[a * FRAME..(b * FRAME).min(pcm.len())].chunks(WIN))
        .filter(|c| c.len() == WIN)
        .map(|c| db(rms(c)))
        .collect();
    if v.is_empty() {
        return -120.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Sum of kept ranges and inner silence over several pieces.
fn report(tag: &str, pcm: &[f32], rs: &[(usize, usize)], kept: &[(usize, usize)], sent_calls: usize) {
    let total = pcm.len();
    let sent: usize = kept.iter().map(|&(a, b)| b - a).sum();
    let (mut sil, mut long, mut nlong) = (0, 0, 0);
    for &k in kept {
        let (s, l, n) = inner(rs, k);
        sil += s;
        long += l;
        nlong += n;
    }
    let sent_ms = ms(sent).max(1);
    println!(
        "  {tag:5} calls={sent_calls:3} sent={:7}ms ({:5.1}% of clip) nonspeech_sent={:6}ms ({:4.1}% of sent) gaps>1s_sent={:6}ms n={nlong} ({:4.1}% of sent)",
        ms(sent),
        100.0 * sent as f32 / total.max(1) as f32,
        sil,
        100.0 * sil as f32 / sent_ms as f32,
        long,
        100.0 * long as f32 / sent_ms as f32,
    );
}

fn dictation(pcm: &[f32], env: bool) {
    let total = pcm.len();
    let rs = runs(pcm);
    let speech_ms: u64 = rs.iter().map(|&(a, b)| ms((b - a) * FRAME)).sum();
    println!("  dur={}ms speech_runs={} speech={}ms", ms(total), rs.len(), speech_ms);
    let Some((a, b)) = trim(pcm, 0) else {
        println!("  SKIP no_speech (engine not called)");
        return;
    };
    let med = speech_median(pcm, &rs);
    let lead_pk = peak_win(pcm, 0, a);
    let tail_pk = peak_win(pcm, b, total);
    let flag = |pk: f32| if pk > med - 10.0 { "CLIP?" } else { "ok" };
    println!(
        "  lead_trim={}ms tail_trim={}ms | speech_med={:.0}dB lead_peak={:.0}dB {} tail_peak={:.0}dB {}",
        ms(a),
        ms(total - b),
        med,
        lead_pk,
        flag(lead_pk),
        tail_pk,
        flag(tail_pk)
    );
    report("whole", pcm, &rs, &[(a, b)], 1);

    // D22: Segmenter cuts, each piece trimmed (engine.rs decode_piece).
    let mut seg = Segmenter::new(SegmentParams {
        pause_ms: RECOMMENDED_PAUSE_MS,
        min_ms: DEFAULT_MIN_MS,
        soft_max_ms: DEFAULT_SOFT_MAX_MS,
    });
    seg.push(pcm);
    let mut bounds = vec![0];
    bounds.extend_from_slice(seg.cuts());
    bounds.push(total);
    let kept: Vec<(usize, usize)> = bounds
        .windows(2)
        .filter_map(|w| trim(&pcm[w[0]..w[1]], w[0]))
        .collect();
    report("seg", pcm, &rs, &kept, kept.len());
    if env {
        envelope(pcm, a, "start");
        envelope(pcm, b, "end");
    }
}

/// Mock engine: records every chunk the meeting chunker hands it.
#[derive(Default)]
struct Rec(Mutex<Vec<Vec<i16>>>);

impl ChunkTranscriber for Rec {
    fn transcribe_chunk(&self, pcm: &[i16], _cfg: &SessionConfig) -> CommitResult {
        self.0.lock().unwrap().push(pcm.to_vec());
        serde_json::from_str(
            r#"{"status":"EMPTY","asr_text":"","final_text":"","language":null,"timings":{"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},"press_enter":false,"fallback_reason":null,"truncated":false,"error":null}"#,
        )
        .unwrap()
    }
}

fn meeting(pcm: &[f32]) {
    let rs = runs(pcm);
    let speech_ms: u64 = rs.iter().map(|&(a, b)| ms((b - a) * FRAME)).sum();
    println!("  dur={}ms speech_runs={} speech={}ms", ms(pcm.len()), rs.len(), speech_ms);
    let rec = Arc::new(Rec::default());
    let mut m = MeetingTranscriber::new(rec.clone(), &SessionConfig::default(), MeetingParams::default());
    let i16s: Vec<i16> = pcm
        .iter()
        .map(|s| (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
        .collect();
    for b in i16s.chunks(WIN) {
        m.push(b).unwrap();
    }
    m.finish().unwrap();
    let chunks = std::mem::take(&mut *rec.0.lock().unwrap());
    // Chunks are handed over in order and never overlap: locate each by
    // its samples, searching forward from the previous one's end.
    let mut ranges = Vec::new();
    let mut from = 0;
    for c in &chunks {
        let at = (from..=i16s.len() - c.len())
            .find(|&i| i16s[i..i + c.len()] == c[..])
            .expect("chunk not found in the recording");
        ranges.push((at, at + c.len()));
        from = at + c.len();
    }
    let to_commit: usize = ranges.iter().map(|&(a, b)| b - a).sum();
    println!(
        "  chunker: chunks={} audio_to_commit={}ms dropped_as_silence={}ms",
        ranges.len(),
        ms(to_commit),
        ms(pcm.len() - to_commit)
    );
    let kept: Vec<(usize, usize)> = ranges
        .iter()
        .filter_map(|&(a, b)| trim(&pcm[a..b], a))
        .collect();
    report("meet", pcm, &rs, &kept, kept.len());
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |f: &str| {
        let had = args.iter().any(|a| a == f);
        args.retain(|a| a != f);
        had
    };
    let (is_meeting, env, raw) = (flag("--meeting"), flag("--envelope"), flag("--pcm16"));
    for (i, p) in args.iter().enumerate() {
        let pcm = read(p, raw);
        println!("clip #{}", i + 1);
        if is_meeting {
            meeting(&pcm);
        } else {
            dictation(&pcm, env);
        }
    }
}
