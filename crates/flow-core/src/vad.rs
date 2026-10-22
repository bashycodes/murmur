//! Lightweight energy + zero-crossing voice activity detection (no model).
//!
//! Used at commit to skip ASR on recordings without speech (status EMPTY,
//! and no chance for Whisper to hallucinate "Thank you." on room noise) and
//! to trim leading/trailing silence before ASR.
//!
//! It fails open: anything it cannot tell from speech counts as speech, so a
//! loud stationary background disables trimming rather than eating words.
//!
//! Per 20 ms frame it computes RMS and the zero-crossing rate (ZCR). The
//! noise floor is the 10th-percentile frame RMS. A frame is speech when its
//! RMS is above `clamp(3 × floor, ABS_MIN, ABS_CAP)` (voiced sounds), or
//! above a lower bar with a high ZCR (unvoiced fricatives such as "s",
//! "f", which are quiet but noisy). Speech needs a run of `MIN_RUN` frames,
//! so a single click or pop does not count. The kept span is padded by
//! `PAD_HEAD` / `PAD_TAIL`, so soft word onsets and endings survive.

use crate::config::SAMPLE_RATE;

pub const FRAME: usize = SAMPLE_RATE / 50; // 20 ms
/// Frames quieter than this (≈ -48 dBFS RMS) are never speech.
pub const ABS_MIN: f32 = 0.004;
/// Frames louder than this (≈ -36 dBFS RMS) are always speech, whatever
/// the floor estimate says (stationary loud signals: fail open).
pub const ABS_CAP: f32 = 0.015;
const FLOOR_FACTOR: f32 = 3.0;
/// Unvoiced bar: this fraction of the voiced threshold, with ZCR above
/// `ZCR_UNVOICED`.
const UNVOICED_FACTOR: f32 = 0.5;
const ZCR_UNVOICED: f32 = 0.3;
/// 60 ms of consecutive speech frames.
pub(crate) const MIN_RUN: usize = 3;
pub const PAD_HEAD: usize = SAMPLE_RATE * 3 / 10; // 300 ms
pub const PAD_TAIL: usize = SAMPLE_RATE * 2 / 5; // 400 ms

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Speech {
    /// Sample range to keep (padded, clamped to the input).
    pub start: usize,
    pub end: usize,
    /// Number of 20 ms frames classified as speech (in qualifying runs).
    pub speech_frames: usize,
}

impl Speech {
    pub fn speech_ms(&self) -> u64 {
        (self.speech_frames * FRAME * 1000 / SAMPLE_RATE) as u64
    }
}

pub(crate) fn frame_stats(f: &[f32]) -> (f32, f32) {
    let rms = (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt();
    let zc = f
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count();
    (rms, zc as f32 / (f.len().max(2) - 1) as f32)
}

/// Per-frame speech decision (before the `MIN_RUN` filter).
pub fn frame_flags(pcm: &[f32]) -> Vec<bool> {
    let stats: Vec<(f32, f32)> = pcm.chunks(FRAME).map(frame_stats).collect();
    if stats.is_empty() {
        return Vec::new();
    }
    let thr = Thresholds::from_stats(&stats);
    stats.iter().map(|&s| thr.is_speech(s)).collect()
}

/// Speech bars derived from the noise floor of a set of frames.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Thresholds {
    voiced: f32,
    unvoiced: f32,
}

impl Thresholds {
    /// `stats` = per-frame (RMS, ZCR); must be non-empty.
    pub(crate) fn from_stats(stats: &[(f32, f32)]) -> Self {
        let mut rms: Vec<f32> = stats.iter().map(|s| s.0).collect();
        let k = rms.len() / 10;
        let (_, floor, _) = rms.select_nth_unstable_by(k, f32::total_cmp);
        let voiced = (*floor * FLOOR_FACTOR).clamp(ABS_MIN, ABS_CAP);
        Self {
            voiced,
            unvoiced: (voiced * UNVOICED_FACTOR).max(ABS_MIN),
        }
    }

    pub(crate) fn is_speech(&self, (r, z): (f32, f32)) -> bool {
        r >= self.voiced || (r >= self.unvoiced && z >= ZCR_UNVOICED)
    }
}

/// The span of `pcm` that holds speech, or `None` when there is none.
pub fn detect(pcm: &[f32]) -> Option<Speech> {
    let flags = frame_flags(pcm);
    let (mut first, mut last, mut frames) = (None, 0, 0);
    let mut i = 0;
    while i < flags.len() {
        if !flags[i] {
            i += 1;
            continue;
        }
        let j = flags[i..].iter().take_while(|&&f| f).count() + i;
        if j - i >= MIN_RUN {
            first.get_or_insert(i);
            last = j;
            frames += j - i;
        }
        i = j;
    }
    let first = first?;
    Some(Speech {
        start: (first * FRAME).saturating_sub(PAD_HEAD),
        end: (last * FRAME + PAD_TAIL).min(pcm.len()),
        speech_frames: frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-noise in [-amp, amp].
    fn noise(n: usize, amp: f32, seed: u32) -> Vec<f32> {
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

    /// Voiced-like signal: 150 Hz tone with harmonics, RMS ≈ amp/√2.
    fn voiced(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                amp * (0.7 * (2.0 * std::f32::consts::PI * 150.0 * t).sin()
                    + 0.3 * (2.0 * std::f32::consts::PI * 450.0 * t).sin())
            })
            .collect()
    }

    fn concat(parts: &[Vec<f32>]) -> Vec<f32> {
        parts.concat()
    }

    const S: usize = SAMPLE_RATE;

    #[test]
    fn digital_silence_has_no_speech() {
        assert_eq!(detect(&vec![0.0; 3 * S]), None);
        assert_eq!(detect(&[]), None);
    }

    #[test]
    fn room_noise_alone_has_no_speech() {
        // -50 dBFS-ish hiss: above the old 0.003 total-RMS gate, not speech.
        assert_eq!(detect(&noise(3 * S, 0.006, 1)), None);
    }

    #[test]
    fn trims_leading_and_trailing_silence_with_padding() {
        let pcm = concat(&[
            noise(2 * S, 0.002, 2),
            voiced(S, 0.2),
            noise(3 * S, 0.002, 3),
        ]);
        let sp = detect(&pcm).unwrap();
        assert!(sp.start <= 2 * S - PAD_HEAD + FRAME, "{sp:?}");
        assert!(sp.start + PAD_HEAD + FRAME >= 2 * S, "{sp:?}");
        assert!(sp.end >= 3 * S + PAD_TAIL - FRAME, "{sp:?}");
        assert!(sp.end <= 3 * S + PAD_TAIL + FRAME, "{sp:?}");
        assert!((900..=1100).contains(&sp.speech_ms()), "{sp:?}");
    }

    #[test]
    fn a_single_click_is_not_speech() {
        let mut pcm = vec![0.0; 2 * S];
        for s in &mut pcm[S..S + FRAME] {
            *s = 0.5;
        }
        assert_eq!(detect(&pcm), None);
    }

    #[test]
    fn quiet_fricative_counts_via_zero_crossings() {
        // Background RMS ≈ 0.003 puts the voiced bar at ≈ 0.009. An "sss"
        // at RMS ≈ 0.006 is below it, but its ZCR is high.
        let bg = |seed| noise(S, 0.0052, seed);
        let fric = noise(S / 2, 0.0104, 4);
        let flags = frame_flags(&concat(&[bg(5), fric.clone(), bg(6)]));
        let (rms, _) = frame_stats(&fric[..FRAME]);
        assert!(rms < 0.009, "{rms}");
        assert!(flags[50..75].iter().all(|&f| f), "{flags:?}");
        assert!(flags[..50].iter().filter(|&&f| f).count() < 3);
    }

    #[test]
    fn loud_stationary_signal_fails_open() {
        // Constant tone: the floor equals the signal; ABS_CAP keeps it speech.
        let pcm = voiced(2 * S, 0.25);
        let sp = detect(&pcm).unwrap();
        assert_eq!((sp.start, sp.end), (0, pcm.len()));
    }

    #[test]
    fn speech_at_the_edges_is_kept_whole() {
        let pcm = concat(&[voiced(S, 0.2), vec![0.0; S], voiced(S, 0.2)]);
        let sp = detect(&pcm).unwrap();
        assert_eq!((sp.start, sp.end), (0, pcm.len()));
    }
}
