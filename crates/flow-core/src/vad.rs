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
//!
//! Quiet recordings: `ABS_MIN` is lowered for a clip whose noise floor is
//! very low, to `REL_FACTOR` × that floor but never under `QUIET_MIN`. So
//! speech recorded at -60 dBFS over a -90 dBFS floor still counts, while
//! stationary noise at any level never reaches 8 × its own floor. The floor
//! for this rule ignores digital silence (all-zero frames from a muted
//! start), which would otherwise make any noise after it look loud. The
//! bars only move when the floor is under `ABS_MIN / REL_FACTOR` (≈ -66
//! dBFS RMS), so normal-level dictation is classified as before.
//!
//! The engines (sherpa, whisper) also refuse audio under a whole-clip RMS
//! of 0.003. [`asr_level`] raises the gain of quiet speech the VAD passed,
//! so that second gate does not blank it.

use crate::config::SAMPLE_RATE;

pub const FRAME: usize = SAMPLE_RATE / 50; // 20 ms
/// Frames quieter than this (≈ -48 dBFS RMS) are never speech.
pub const ABS_MIN: f32 = 0.004;
/// Frames louder than this (≈ -36 dBFS RMS) are always speech, whatever
/// the floor estimate says (stationary loud signals: fail open).
pub const ABS_CAP: f32 = 0.015;
const FLOOR_FACTOR: f32 = 3.0;
/// Quiet-recording bar: speech must be this far above the floor (18 dB)...
pub const REL_FACTOR: f32 = 8.0;
/// ...and at least this loud (≈ -66 dBFS RMS, 16 in PCM16).
pub const QUIET_MIN: f32 = 0.0005;
/// Frames at or below this RMS (half a PCM16 step) are digital silence.
const DIGITAL_SILENCE: f32 = 1.5e-5;
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

/// 10th-percentile value of a non-empty slice (reorders it).
fn p10(v: &mut [f32]) -> f32 {
    let k = v.len() / 10;
    *v.select_nth_unstable_by(k, f32::total_cmp).1
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
        let floor = p10(&mut rms);
        // The quiet-recording floor, over frames that are not digital silence.
        rms.retain(|&r| r > DIGITAL_SILENCE);
        let min_bar = if rms.is_empty() {
            ABS_MIN
        } else {
            (p10(&mut rms) * REL_FACTOR).clamp(QUIET_MIN, ABS_MIN)
        };
        let voiced = (floor * FLOOR_FACTOR).clamp(min_bar, ABS_CAP);
        Self {
            voiced,
            unvoiced: (voiced * UNVOICED_FACTOR).max(min_bar),
        }
    }

    pub(crate) fn is_speech(&self, (r, z): (f32, f32)) -> bool {
        r >= self.voiced || (r >= self.unvoiced && z >= ZCR_UNVOICED)
    }
}

/// Noise-floor window of the streaming classifier: the last 30 s of frames.
pub(crate) const STREAM_FLOOR_FRAMES: usize = 30 * 50;

/// The commit-time classifier run as a stream (shared by the hands-free
/// endpointer, D19, and the pause segmenter, D22): the same 20 ms frames,
/// speech bars and `MIN_RUN`, with the noise floor taken from the last
/// 30 s of frames seen so far. Fed sample by sample, so its answers depend
/// only on the audio, never on how it was chunked.
#[derive(Debug, Clone)]
pub struct StreamVad {
    /// Samples of the current, incomplete frame.
    pending: Vec<f32>,
    /// (RMS, ZCR) of the last `STREAM_FLOOR_FRAMES` frames.
    window: std::collections::VecDeque<(f32, f32)>,
    frames: u64,
    run: usize,
    speech_frames: u64,
    /// Frame index just after the last speech run (0 = no speech yet).
    speech_end: u64,
}

impl Default for StreamVad {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamVad {
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(FRAME),
            window: std::collections::VecDeque::with_capacity(STREAM_FLOOR_FRAMES),
            frames: 0,
            run: 0,
            speech_frames: 0,
            speech_end: 0,
        }
    }

    /// Feed one normalised sample; true when it completed a frame.
    #[inline]
    pub fn sample(&mut self, s: f32) -> bool {
        self.pending.push(s);
        if self.pending.len() < FRAME {
            return false;
        }
        let st = frame_stats(&self.pending);
        self.pending.clear();
        if self.window.len() == STREAM_FLOOR_FRAMES {
            self.window.pop_front();
        }
        self.window.push_back(st);
        self.frames += 1;
        if Thresholds::from_stats(self.window.make_contiguous()).is_speech(st) {
            self.run += 1;
            if self.run == MIN_RUN {
                self.speech_frames += MIN_RUN as u64;
            } else if self.run > MIN_RUN {
                self.speech_frames += 1;
            }
            if self.run >= MIN_RUN {
                self.speech_end = self.frames;
            }
        } else {
            self.run = 0;
        }
        true
    }

    /// Complete frames seen.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Frames in speech runs of at least `MIN_RUN`.
    pub fn speech_frames(&self) -> u64 {
        self.speech_frames
    }

    /// Frame index just after the last speech run (0 = none yet).
    pub fn speech_end(&self) -> u64 {
        self.speech_end
    }

    /// Length of the current speech-frame run (any length).
    pub fn run(&self) -> usize {
        self.run
    }

    /// Frames since the last speech run (since the start without speech).
    pub fn trailing_frames(&self) -> u64 {
        self.frames - self.speech_end
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

/// Speech quieter than this RMS (≈ -44 dBFS) is raised by [`asr_level`];
/// twice the engines' 0.003 silence gate, so anything louder already passes it.
pub const ASR_LEVEL_BELOW: f32 = 0.006;
/// [`asr_level`] scales the peak up to this...
const ASR_TARGET_PEAK: f32 = 0.5;
/// ...with at most this gain (+40 dB).
const ASR_MAX_GAIN: f32 = 100.0;

/// The audio to hand the ASR for a span the VAD found speech in: as is,
/// unless it is quieter than [`ASR_LEVEL_BELOW`], then scaled so its peak
/// reaches [`ASR_TARGET_PEAK`] (gain capped at [`ASR_MAX_GAIN`]). The
/// recognizers' log-mel features hardly depend on level, but their own
/// RMS < 0.003 gate would return "" for quiet speech (`blank_asr`).
pub fn asr_level(pcm: &[f32]) -> std::borrow::Cow<'_, [f32]> {
    with_gain(pcm, asr_gain(pcm))
}

/// The gain [`asr_level`] applies to `pcm` (1.0 = left as is).
pub fn asr_gain(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return 1.0;
    }
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt();
    let peak = pcm.iter().fold(0f32, |m, s| m.max(s.abs()));
    if rms >= ASR_LEVEL_BELOW || peak <= 0.0 {
        return 1.0;
    }
    (ASR_TARGET_PEAK / peak).clamp(1.0, ASR_MAX_GAIN)
}

/// `pcm` scaled by `gain` (borrowed when the gain is not above 1).
pub fn with_gain(pcm: &[f32], gain: f32) -> std::borrow::Cow<'_, [f32]> {
    use std::borrow::Cow;
    if gain <= 1.0 {
        return Cow::Borrowed(pcm);
    }
    Cow::Owned(pcm.iter().map(|s| s * gain).collect())
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

    #[test]
    fn a_soft_lead_in_before_loud_speech_is_kept() {
        // The shape of FLEURS he 10043827527204102024 (2026-10-31): ~1 s of
        // breath at ≈ -33 dBFS RMS, then speech at ≈ -10 dBFS. The quiet
        // part is above ABS_CAP, so nothing before the first word is cut.
        let pcm = concat(&[
            noise(S, 0.035, 7),
            voiced(3 * S, 0.4),
            noise(S / 2, 0.001, 8),
        ]);
        let (rms, _) = frame_stats(&pcm[..FRAME * 10]);
        assert!(rms > ABS_CAP, "{rms}");
        let sp = detect(&pcm).unwrap();
        assert_eq!(sp.start, 0, "{sp:?}");
        assert!(sp.end >= 4 * S, "{sp:?}");
    }

    /// Words of quiet voiced sound (0.4 s on, 0.2 s off), over `floor`.
    fn quiet_speech(words: usize, amp: f32, floor: f32) -> Vec<f32> {
        let mut v = noise(S / 2, floor, 20);
        for w in 0..words {
            let on: Vec<f32> = voiced(S * 2 / 5, amp)
                .iter()
                .zip(noise(S * 2 / 5, floor, 21 + w as u32))
                .map(|(a, b)| a + b)
                .collect();
            v.extend(on);
            v.extend(noise(S / 5, floor, 40 + w as u32));
        }
        v.extend(noise(S / 2, floor, 60));
        v
    }

    #[test]
    fn quiet_speech_over_a_clean_floor_counts() {
        // FLEURS he 2026-10-31 `no_speech` clips: speech at ≈ -60 dBFS RMS
        // (PCM16 ≈ 30) over a floor near -90 dBFS (PCM16 ≈ 1).
        let pcm = quiet_speech(6, 0.0015, 0.00005);
        let (r, _) = frame_stats(&pcm[S / 2 + FRAME..S / 2 + 2 * FRAME]);
        assert!(r < ABS_MIN / 2.0, "{r}");
        let sp = detect(&pcm).expect("quiet speech is speech");
        assert!(sp.speech_ms() >= 2000, "{sp:?}");
        assert!(sp.start <= S / 2 && sp.end >= pcm.len() - S / 2, "{sp:?}");
    }

    #[test]
    fn quiet_noise_alone_has_no_speech() {
        // Hiss at every low level, including just above QUIET_MIN.
        for (k, amp) in [0.0003f32, 0.0009, 0.002, 0.004].into_iter().enumerate() {
            assert_eq!(detect(&noise(3 * S, amp, 70 + k as u32)), None, "{amp}");
        }
    }

    #[test]
    fn noise_after_digital_silence_has_no_speech() {
        // A muted start (exact zeros) must not make the hiss after it look
        // loud: the quiet floor ignores digital silence.
        let pcm = concat(&[vec![0.0; S], noise(2 * S, 0.003, 80)]);
        assert_eq!(detect(&pcm), None);
    }

    #[test]
    fn a_quiet_click_on_a_clean_floor_is_not_speech() {
        let mut pcm = noise(2 * S, 0.00005, 90);
        for s in &mut pcm[S..S + FRAME] {
            *s += 0.002;
        }
        assert_eq!(detect(&pcm), None);
    }

    #[test]
    fn quiet_floor_leaves_normal_bars_alone() {
        // Floor ≥ ABS_MIN / REL_FACTOR: the bars are the same as before.
        let stats: Vec<(f32, f32)> = noise(2 * S, 0.002, 91)
            .chunks(FRAME)
            .map(frame_stats)
            .collect();
        let t = Thresholds::from_stats(&stats);
        assert_eq!(t.voiced, ABS_MIN);
        assert_eq!(t.unvoiced, ABS_MIN);
    }

    #[test]
    fn asr_level_raises_quiet_speech_only() {
        let rms = |p: &[f32]| frame_stats(p).0;
        let quiet = quiet_speech(3, 0.0015, 0.00005);
        let up = asr_level(&quiet);
        assert!(matches!(up, std::borrow::Cow::Owned(_)));
        assert!(rms(&up) > 0.003 * 2.0, "{}", rms(&up));
        assert!(up.iter().all(|s| s.abs() <= 0.5 + 1e-6));
        let loud = voiced(S, 0.2);
        assert!(matches!(asr_level(&loud), std::borrow::Cow::Borrowed(_)));
        let zeros = vec![0.0; S];
        assert!(matches!(asr_level(&zeros), std::borrow::Cow::Borrowed(_)));
        assert!(asr_level(&[]).is_empty());
        // The gain is capped: a -100 dB blip stays tiny.
        let blip = vec![1e-5; S];
        assert!(asr_level(&blip).iter().all(|s| *s <= 1e-3 + 1e-9));
    }
}
