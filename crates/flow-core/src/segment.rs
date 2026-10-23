//! Pause segmentation (D22): cut a dictation at pauses so that finished
//! pieces can be decoded while the user is still talking, and only the
//! tail is left for `commit`.
//!
//! [`Segmenter`] is a streaming detector over the samples the worker
//! drains. It runs the commit-time classifier as a stream
//! ([`StreamVad`]: D16 bars, 20 ms frames, noise floor over the last
//! 30 s) and emits cut sample indices:
//!
//! * **Pause cut.** Once the current pause reaches `pause_ms`, the cut goes
//!   at `speech_end + pause_ms / 2` (the middle of the first `pause_ms` of
//!   the pause), if the open segment is then at least `min_ms` long and
//!   holds speech. A shorter piece keeps growing (short pieces lose
//!   context: measured +0.3 WER points at 500 ms pauses).
//! * **Soft max.** An open segment of `soft_max_ms` with no qualifying
//!   pause is cut in the middle of the longest gap of at least 200 ms
//!   that started in the last 5 s, if any. Without such a gap it keeps
//!   growing (a word is never cut).
//!
//! The cuts are a pure function of the audio: the detector sees one
//! sample at a time, so push chunking, drain timing and decode speed
//! cannot move them. That is what keeps local, server and router
//! fallback replay byte-identical.
//!
//! [`split_long`] is the offline relative used by the Parakeet
//! empty-result fallback (`docs/proposals/streaming-asr-latency.md` §1.3).

use std::collections::VecDeque;
use std::ops::Range;

use crate::config::{SessionConfig, SAMPLE_RATE};
use crate::vad::{self, StreamVad, FRAME};

/// Recommended `SessionConfig.segment_pause_ms` (proposal §2: WER equal to
/// whole-clip decoding for Parakeet and whisper base on 19-62 s clips).
pub const RECOMMENDED_PAUSE_MS: u32 = 800;
/// Default `segment_min_ms`.
pub const DEFAULT_MIN_MS: u32 = 4000;
/// Default `segment_soft_max_ms`.
pub const DEFAULT_SOFT_MAX_MS: u32 = 20_000;
/// Pauses below this are never configured (raised to it).
pub const MIN_PAUSE_MS: u32 = 200;
/// Shortest gap the soft max may cut in.
const SOFT_GAP_FRAMES: u64 = 10; // 200 ms
/// How far back the soft max looks for a gap.
const SOFT_LOOKBACK_FRAMES: u64 = 250; // 5 s
const FRAME_MS: u32 = (FRAME * 1000 / SAMPLE_RATE) as u32;

/// Segmenter settings, from [`SessionConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentParams {
    pub pause_ms: u32,
    pub min_ms: u32,
    pub soft_max_ms: u32,
}

impl SegmentParams {
    /// `None` when segmentation is off for this session.
    pub fn for_session(cfg: &SessionConfig) -> Option<Self> {
        let pause_ms = cfg.segment_pause_ms?;
        Some(Self {
            pause_ms: pause_ms.max(MIN_PAUSE_MS),
            min_ms: cfg.segment_min_ms.unwrap_or(DEFAULT_MIN_MS),
            soft_max_ms: cfg.segment_soft_max_ms.unwrap_or(DEFAULT_SOFT_MAX_MS),
        })
    }
}

/// Streaming pause segmenter (see the module docs).
#[derive(Debug, Clone)]
pub struct Segmenter {
    vad: StreamVad,
    pause_frames: u64,
    min_samples: usize,
    soft_max_samples: usize,
    /// Samples fed so far.
    fed: usize,
    /// First sample of the open segment (the last cut, or 0).
    open_start: usize,
    /// `vad.speech_end()` before the current frame.
    last_speech_end: u64,
    /// Completed gaps between speech runs, `(start, end)` frames, recent.
    gaps: VecDeque<(u64, u64)>,
    cuts: Vec<usize>,
}

impl Segmenter {
    pub fn new(p: SegmentParams) -> Self {
        let ms = |ms: u32| ms as usize * SAMPLE_RATE / 1000;
        Self {
            vad: StreamVad::new(),
            pause_frames: p.pause_ms.div_ceil(FRAME_MS) as u64,
            min_samples: ms(p.min_ms),
            soft_max_samples: ms(p.soft_max_ms.max(p.min_ms)),
            fed: 0,
            open_start: 0,
            last_speech_end: 0,
            gaps: VecDeque::new(),
            cuts: Vec::new(),
        }
    }

    /// Feed the next samples (normalised f32, in order).
    pub fn push(&mut self, pcm: &[f32]) {
        for &s in pcm {
            self.fed += 1;
            if self.vad.sample(s) {
                self.frame();
            }
        }
    }

    /// All cuts so far: strictly increasing sample indices.
    pub fn cuts(&self) -> &[usize] {
        &self.cuts
    }

    /// Samples fed so far.
    pub fn fed(&self) -> usize {
        self.fed
    }

    /// Sample index just after the last speech run (0 = no speech yet).
    pub fn speech_end_sample(&self) -> usize {
        self.vad.speech_end() as usize * FRAME
    }

    /// Audio since the last speech run, in ms.
    pub fn trailing_silence_ms(&self) -> u64 {
        self.vad.trailing_frames() * FRAME_MS as u64
    }

    fn frame(&mut self) {
        let f = self.vad.frames();
        let se = self.vad.speech_end();
        if se != self.last_speech_end {
            // A new speech run reached MIN_RUN this frame: the gap since
            // the previous run is complete.
            let run_start = f.saturating_sub(self.vad.run() as u64);
            if self.vad.run() == vad::MIN_RUN
                && self.last_speech_end > 0
                && run_start > self.last_speech_end
            {
                self.gaps.push_back((self.last_speech_end, run_start));
            }
            self.last_speech_end = se;
        }
        while self
            .gaps
            .front()
            .is_some_and(|g| g.0 + SOFT_LOOKBACK_FRAMES < f)
        {
            self.gaps.pop_front();
        }
        let speech_end = se as usize * FRAME;
        if speech_end <= self.open_start {
            return; // no speech in the open segment yet
        }
        let pause = f - se;
        if pause == self.pause_frames {
            let cut = speech_end + self.pause_frames as usize * FRAME / 2;
            if cut - self.open_start >= self.min_samples {
                return self.cut(cut);
            }
        }
        if (f as usize * FRAME) - self.open_start >= self.soft_max_samples {
            // The ongoing gap counts once it is long enough.
            let ongoing = (pause >= SOFT_GAP_FRAMES).then_some((se, f));
            let open_frame = self.open_start.div_ceil(FRAME) as u64;
            let best = self
                .gaps
                .iter()
                .copied()
                .chain(ongoing)
                .filter(|&(a, b)| {
                    b - a >= SOFT_GAP_FRAMES && a + SOFT_LOOKBACK_FRAMES >= f && a >= open_frame
                })
                // Longest; the latest on a tie (deterministic).
                .max_by_key(|&(a, b)| (b - a, a));
            if let Some((a, b)) = best {
                self.cut((a + (b - a) / 2) as usize * FRAME);
            }
        }
    }

    fn cut(&mut self, at: usize) {
        debug_assert!(at > self.open_start && at <= self.fed);
        self.cuts.push(at);
        self.open_start = at;
        self.gaps.clear();
    }
}

/// Offline split of `pcm` into pieces of at most `max_len` samples, cut in
/// the middle of the longest pause (whole-clip D16 frame flags), else at
/// the quietest frame, recursively. Pieces never get shorter than 1 s on
/// either side of a cut. Used to re-decode a clip an engine returned
/// nothing for.
pub fn split_long(pcm: &[f32], max_len: usize) -> Vec<Range<usize>> {
    let flags = vad::frame_flags(pcm);
    let rms: Vec<f32> = pcm.chunks(FRAME).map(|f| vad::frame_stats(f).0).collect();
    let mut out = Vec::new();
    split_rec(
        0..pcm.len(),
        max_len.max(2 * SAMPLE_RATE),
        &flags,
        &rms,
        &mut out,
    );
    out
}

fn split_rec(
    r: Range<usize>,
    max_len: usize,
    flags: &[bool],
    rms: &[f32],
    out: &mut Vec<Range<usize>>,
) {
    if r.len() <= max_len {
        out.push(r);
        return;
    }
    // Candidate frames: at least 1 s from either end.
    let lo = (r.start + SAMPLE_RATE).div_ceil(FRAME);
    let hi = (r.end - SAMPLE_RATE) / FRAME;
    let mut best: Option<(usize, usize)> = None; // (run length, middle frame)
    let mut i = lo;
    while i < hi {
        if flags.get(i).copied().unwrap_or(true) {
            i += 1;
            continue;
        }
        let j = (i..hi).find(|&k| flags[k]).unwrap_or(hi);
        let cand = (j - i, i + (j - i) / 2);
        if best.is_none_or(|b| cand.0 > b.0) {
            best = Some(cand);
        }
        i = j;
    }
    let mid = match best {
        Some((_, m)) => m,
        None => (lo..hi)
            .min_by(|&a, &b| rms[a].total_cmp(&rms[b]))
            .unwrap_or((r.start + r.end) / 2 / FRAME),
    };
    let cut = (mid * FRAME).clamp(r.start + 1, r.end - 1);
    split_rec(r.start..cut, max_len, flags, rms, out);
    split_rec(cut..r.end, max_len, flags, rms, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: usize = SAMPLE_RATE;

    fn voiced(ms: usize, amp: f32) -> Vec<f32> {
        (0..ms * S / 1000)
            .map(|i| {
                let t = i as f32 / S as f32;
                amp * (2.0 * std::f32::consts::PI * 150.0 * t).sin()
            })
            .collect()
    }

    fn quiet(ms: usize, seed: u32) -> Vec<f32> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        (0..ms * S / 1000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.001
            })
            .collect()
    }

    /// `[(speech ms, pause ms)]` → audio.
    fn utterance(parts: &[(usize, usize)]) -> Vec<f32> {
        let mut v = quiet(300, 99);
        for (k, &(sp, pa)) in parts.iter().enumerate() {
            v.extend(voiced(sp, 0.2));
            v.extend(quiet(pa, k as u32 + 1));
        }
        v
    }

    fn params(pause: u32) -> SegmentParams {
        SegmentParams {
            pause_ms: pause,
            min_ms: DEFAULT_MIN_MS,
            soft_max_ms: DEFAULT_SOFT_MAX_MS,
        }
    }

    fn cuts_bulk(pcm: &[f32], p: SegmentParams) -> Vec<usize> {
        let mut s = Segmenter::new(p);
        s.push(pcm);
        s.cuts().to_vec()
    }

    #[test]
    fn cuts_in_the_middle_of_long_pauses() {
        // 3 phrases of 5 s, 1 s pauses: two cuts, each 400 ms after the
        // speech end (pause 800 / 2).
        let pcm = utterance(&[(5000, 1000), (5000, 1000), (5000, 300)]);
        let cuts = cuts_bulk(&pcm, params(800));
        assert_eq!(cuts.len(), 2, "{cuts:?}");
        let end1 = (300 + 5000) * S / 1000;
        assert!(
            cuts[0].abs_diff(end1 + 400 * S / 1000) <= 3 * FRAME,
            "{cuts:?}"
        );
        let end2 = (300 + 5000 + 1000 + 5000) * S / 1000;
        assert!(
            cuts[1].abs_diff(end2 + 400 * S / 1000) <= 3 * FRAME,
            "{cuts:?}"
        );
    }

    #[test]
    fn short_pauses_do_not_cut() {
        let pcm = utterance(&[(5000, 600), (5000, 600), (5000, 300)]);
        assert!(cuts_bulk(&pcm, params(800)).is_empty());
    }

    #[test]
    fn no_cut_before_the_minimum_segment() {
        // 1 s phrases with 1 s pauses: a cut only once ≥ 4 s accumulated.
        let pcm = utterance(&[(1000, 1000); 8]);
        let cuts = cuts_bulk(&pcm, params(800));
        assert!(!cuts.is_empty());
        let mut prev = 0;
        for &c in &cuts {
            assert!(c - prev >= DEFAULT_MIN_MS as usize * S / 1000, "{cuts:?}");
            prev = c;
        }
    }

    #[test]
    fn silence_only_has_no_segments() {
        assert!(cuts_bulk(&quiet(30_000, 7), params(800)).is_empty());
        assert!(cuts_bulk(&vec![0.0; 30 * S], params(800)).is_empty());
    }

    #[test]
    fn identical_cuts_under_any_chunking() {
        let pcm = utterance(&[
            (4200, 900),
            (6100, 1200),
            (3000, 850),
            (2500, 2000),
            (7000, 300),
        ]);
        let p = params(800);
        let want = cuts_bulk(&pcm, p);
        assert!(want.len() >= 2, "{want:?}");
        // Fixed sizes (10 ms, 40 ms, 1 s) and pseudo-random ones.
        let mut x: u32 = 12345;
        let mut sizes: Vec<Box<dyn FnMut() -> usize>> = vec![
            Box::new(|| 160),
            Box::new(|| 640),
            Box::new(|| S),
            Box::new(move || {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                1 + (x as usize % 5000)
            }),
        ];
        for next in sizes.iter_mut() {
            let mut s = Segmenter::new(p);
            let mut i = 0;
            while i < pcm.len() {
                let n = next().min(pcm.len() - i);
                s.push(&pcm[i..i + n]);
                i += n;
            }
            assert_eq!(s.cuts(), &want[..]);
        }
    }

    #[test]
    fn soft_max_cuts_only_at_a_gap() {
        // 26 s of speech with 300 ms gaps every 3 s (< the 800 ms pause):
        // the soft max cuts in one of those gaps.
        let mut parts = vec![(2700, 300); 9];
        parts.push((2000, 300));
        let pcm = utterance(&parts);
        let cuts = cuts_bulk(&pcm, params(800));
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        let c = cuts[0];
        assert!((15 * S..=20 * S + FRAME * 2).contains(&c), "{c}");
        // The cut sits inside a quiet gap.
        let rms = vad::frame_stats(&pcm[c - FRAME..c + FRAME]).0;
        assert!(rms < 0.01, "{rms}");
        // Continuous speech: no gap, no cut at all (a gap after the
        // speech would qualify).
        let pcm = utterance(&[(26_000, 0)]);
        assert!(cuts_bulk(&pcm, params(800)).is_empty());
    }

    #[test]
    fn for_session_reads_the_config() {
        let mut c = SessionConfig::default();
        assert_eq!(SegmentParams::for_session(&c), None);
        c.segment_pause_ms = Some(50);
        let p = SegmentParams::for_session(&c).unwrap();
        assert_eq!(p.pause_ms, MIN_PAUSE_MS);
        assert_eq!(
            (p.min_ms, p.soft_max_ms),
            (DEFAULT_MIN_MS, DEFAULT_SOFT_MAX_MS)
        );
    }

    #[test]
    fn split_long_cuts_at_pauses_into_bounded_pieces() {
        let pcm = utterance(&[
            (6000, 700),
            (6000, 400),
            (6000, 700),
            (6000, 700),
            (6000, 0),
        ]);
        let pieces = split_long(&pcm, 20 * S);
        assert!(pieces.len() >= 2);
        assert_eq!(pieces.first().unwrap().start, 0);
        assert_eq!(pieces.last().unwrap().end, pcm.len());
        for w in pieces.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        for r in &pieces {
            assert!(r.len() <= 20 * S, "{pieces:?}");
            // Each cut is in a quiet stretch.
            if r.start > 0 {
                assert!(vad::frame_stats(&pcm[r.start..r.start + FRAME]).0 < 0.01);
            }
        }
        // Short input: one piece.
        assert_eq!(split_long(&pcm[..5 * S], 20 * S), vec![0..5 * S]);
    }
}
