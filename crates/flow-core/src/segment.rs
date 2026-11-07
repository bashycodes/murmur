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

/// Audio after a cut that a boundary "bridge" decode covers (the worker
/// decodes the bridge while recording once this much follows the cut).
pub const BRIDGE_SAMPLES: usize = SAMPLE_RATE * 4;
/// Audio before a cut that a bridge covers. Longer than the right side:
/// with 4 s, Parakeet-unified dropped real sentence ends at cuts
/// ("flowers the pain"); with 6 s it kept them (cliptest 2026-10-23,
/// `tools/probes/prototypes/seg_join_2026_10_23`). The left audio is
/// already there, so this costs decode time only, not waiting.
pub const BRIDGE_LEFT_SAMPLES: usize = SAMPLE_RATE * 6;
/// Words of each piece next to a cut that the bridge may rewrite.
const SPLICE_WORDS: usize = 6;
/// Of those, the words right at the cut that may also change (not just
/// their punctuation and case): the ones a piece decoded without context.
const SPLICE_FREE_WORDS: usize = 2;

/// The bridge window around `cut`, within the two pieces `lo..hi`.
/// `FLOW_BRIDGE_LEFT_MS` overrides the left side (bench knob, like
/// `FLOW_SEGMENT_JOIN`; `tests/segment_bench.rs`).
pub fn bridge_range(cut: usize, lo: usize, hi: usize) -> Range<usize> {
    let left = std::env::var("FLOW_BRIDGE_LEFT_MS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(BRIDGE_LEFT_SAMPLES, |ms| ms * SAMPLE_RATE / 1000);
    cut.saturating_sub(left).max(lo)..(cut + BRIDGE_SAMPLES).min(hi)
}

fn key(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Rewrite the words on both sides of a cut with a decode of the audio
/// around it (`bridge`), so punctuation, case and words near a cut are
/// decided with context on both sides: pieces decoded alone end every
/// pause with a sentence break ("ask not. What your country").
///
/// The last [`SPLICE_WORDS`] of `left` and the first of `right` are
/// aligned with the bridge (normalised words, free end gaps). The span
/// between the first exact match on the left and the last exact match on
/// the right is replaced with the bridge's words, and split back at the
/// bridge's first word aligned to `right`. Needs an exact match on each
/// side of the cut, and the rewrite must keep every word except the two
/// next to the cut on each side (only punctuation and case change
/// there); otherwise (or with nothing to change) `None`. The
/// bridge's first and last words, decoded without context on one side,
/// are never anchors.
pub fn splice<'a>(left: &'a str, right: &'a str, bridge: &'a str) -> Option<(String, String)> {
    let lw: Vec<&str> = left.split_whitespace().collect();
    let rw: Vec<&str> = right.split_whitespace().collect();
    let bw: Vec<&str> = bridge.split_whitespace().collect();
    let t0 = lw.len().saturating_sub(SPLICE_WORDS);
    let s: Vec<&str> = lw[t0..]
        .iter()
        .chain(rw.iter().take(SPLICE_WORDS))
        .copied()
        .collect();
    let nt = lw.len() - t0;
    let (sk, bk): (Vec<String>, Vec<String>) = (
        s.iter().map(|w| key(w)).collect(),
        bw.iter().map(|w| key(w)).collect(),
    );
    let (n, m) = (sk.len(), bk.len());
    if nt == 0 || n == nt || m == 0 {
        return None;
    }
    // Longest common subsequence of the normalised words: the exact
    // matches are the anchors; words between anchors are substitutions.
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if sk[i] == bk[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if sk[i] == bk[j] && dp[i][j] == dp[i + 1][j + 1] + 1 {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    // The bridge's first and last words were decoded without context on
    // one side (and may be cut mid-word): never anchor on them.
    pairs.retain(|p| p.1 >= 1 && p.1 + 2 <= m);
    let &(ta, ba) = pairs.iter().find(|p| p.0 < nt)?;
    let &(hz, bz) = pairs.iter().rev().find(|p| p.0 >= nt)?;
    let &(sl, bl) = pairs.iter().rev().find(|p| p.0 < nt)?;
    let &(sr, br) = pairs.iter().find(|p| p.0 >= nt)?;
    // Bridge words between the last left anchor and the first right one
    // go left as far as there were left words there.
    let bj = (bl + 1 + (nt - 1 - sl)).min(br);
    debug_assert!(sr >= nt && ba <= bl && bl < bj && bj <= br && br <= bz);
    // Accept only a like-for-like rewrite: the same number of words on
    // each side, the same words (normalised) except at most the two next
    // to the cut. Punctuation and case may change anywhere in the span.
    // Anything else means the alignment slipped on a common word (it
    // duplicated "the man who told") or the bridge, with less context,
    // heard a different word: keep the pieces.
    let (old_l, new_l) = (&lw[t0 + ta..], &bw[ba..bj]);
    let (old_r, new_r) = (&rw[..=hz - nt], &bw[bj..=bz]);
    if old_l.len() != new_l.len() || old_r.len() != new_r.len() {
        return None;
    }
    let near = |from_cut: usize| from_cut < SPLICE_FREE_WORDS;
    let n_l = old_l.len();
    let same_l = old_l
        .iter()
        .zip(new_l)
        .enumerate()
        .all(|(i, (a, b))| key(a) == key(b) || near(n_l - 1 - i));
    let same_r = old_r
        .iter()
        .zip(new_r)
        .enumerate()
        .all(|(i, (a, b))| key(a) == key(b) || near(i));
    if !same_l || !same_r {
        return None;
    }
    // Next to its edges the bridge lacks context too ("Capacity, the
    // Englishman" for "capacity. The Englishman"): there the pieces' own
    // spelling stands.
    let pick = |j: usize, old: &'a str| -> &'a str {
        if j < BRIDGE_EDGE_WORDS || j + BRIDGE_EDGE_WORDS >= m {
            old
        } else {
            bw[j]
        }
    };
    let mut words: Vec<String> = lw[..t0 + ta].iter().map(|w| w.to_string()).collect();
    words.extend(
        old_l
            .iter()
            .enumerate()
            .map(|(i, &o)| pick(ba + i, o).to_string()),
    );
    let split = words.len();
    words.extend(
        old_r
            .iter()
            .enumerate()
            .map(|(i, &o)| pick(bj + i, o).to_string()),
    );
    let span_end = words.len();
    words.extend(rw[hz - nt + 1..].iter().map(|w| w.to_string()));
    // Mixed sources can leave "him. the": a rewritten word (or the one
    // after the span) that follows a sentence end starts upper case. And
    // "fine, The" (D70): a bridge word capitalised after the bridge's own
    // sentence break, kept after a piece word with no break, takes the
    // piece's lower case back.
    let olds: Vec<&str> = old_l.iter().chain(old_r).copied().collect();
    for i in (t0 + ta).max(1)..=span_end.min(words.len() - 1) {
        if ends_sentence(&words[i - 1]) {
            if let Some(c) = capitalized(&words[i]) {
                words[i] = c;
            }
        } else if let Some(&old) = olds.get(i - (t0 + ta)) {
            if capitalized(old).as_deref() == Some(words[i].as_str()) {
                words[i] = old.to_string();
            }
        }
    }
    let (nl, nr) = (words[..split].join(" "), words[split..].join(" "));
    (nl != left || nr != right).then_some((nl, nr))
}

/// Words at each end of a bridge whose case and punctuation are not
/// taken (the bridge heard them without context on one side).
const BRIDGE_EDGE_WORDS: usize = 2;

/// The two words at a cut, spelled as the bridge spells them: the
/// fallback when [`splice`] refuses the span (a word away from the cut
/// differs, as "quote-unquote" against "quote unquote"). A piece decoded
/// alone starts with a capital, so without it a pause mid-sentence leaves
/// "familiar with Things like" (D70).
///
/// Needs `left`'s last word and `right`'s first word next to each other,
/// once, in the bridge (normalised words), both clear of the bridge's
/// edge words. Only their case and punctuation can change; `None` when
/// nothing does or the pair is missing.
pub fn respell_cut(left: &str, right: &str, bridge: &str) -> Option<(String, String)> {
    let (l_head, l_last) = match left.trim_end().rsplit_once(char::is_whitespace) {
        Some((h, w)) => (Some(h), w),
        None => (None, left.trim()),
    };
    let r = right.trim_start();
    let (r_first, r_tail) = r
        .split_once(char::is_whitespace)
        .map_or((r, None), |(w, t)| (w, Some(t)));
    let (kl, kr) = (key(l_last), key(r_first));
    if kl.is_empty() || kr.is_empty() {
        return None;
    }
    let bw: Vec<&str> = bridge.split_whitespace().collect();
    let m = bw.len();
    let mut hits = (1..m).filter(|&j| {
        j > BRIDGE_EDGE_WORDS
            && j + BRIDGE_EDGE_WORDS < m
            && key(bw[j - 1]) == kl
            && key(bw[j]) == kr
    });
    let j = hits.next()?;
    if hits.next().is_some() {
        return None; // ambiguous
    }
    let (new_l, new_r) = (bw[j - 1], bw[j]);
    if new_l == l_last && new_r == r_first {
        return None;
    }
    let nl = l_head.map_or(new_l.to_string(), |h| format!("{h} {new_l}"));
    let nr = r_tail.map_or(new_r.to_string(), |t| format!("{new_r} {t}"));
    Some((nl, nr))
}

/// Lower-case words that end with a period without ending a sentence.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "st", "jr", "sr", "prof", "vs", "etc", "e.g", "i.e", "eg", "ie",
    "approx", "a.m", "p.m", "fig",
];

/// Whether `w` ends a sentence: `.`, `?` or `!` (before closing quotes or
/// brackets), not an ellipsis or a known abbreviation.
fn ends_sentence(w: &str) -> bool {
    let w = w.trim_end_matches(['"', '\'', ')', ']', '\u{201D}', '\u{2019}']);
    if w.ends_with("..") || w.ends_with('\u{2026}') {
        return false;
    }
    if w.ends_with(['?', '!']) {
        return true;
    }
    let Some(stem) = w.strip_suffix('.') else {
        return false;
    };
    let stem = stem.trim_start_matches(['"', '\'', '(', '[', '\u{201C}', '\u{2018}']);
    !stem.is_empty() && !ABBREVIATIONS.contains(&stem.to_lowercase().as_str())
}

/// `w` with its first letter upper-cased, when it starts with a lower-case
/// letter and has no other capitals ("iPhone" and "eBay" stay).
fn capitalized(w: &str) -> Option<String> {
    let mut ch = w.chars();
    let c = ch.next()?;
    if !c.is_lowercase() || ch.clone().any(char::is_uppercase) {
        return None;
    }
    Some(c.to_uppercase().chain(ch).collect())
}

/// `right` with its first word capitalised when `left` ends a sentence
/// and `right` starts lower case: a piece decoded alone (or with
/// whisper's `prior_text`) may start "the" after a cut that ends a
/// sentence ("figures. that is"). `None` when nothing changes.
pub fn capitalize_after_stop(left: &str, right: &str) -> Option<String> {
    let last = left.split_whitespace().next_back()?;
    if !ends_sentence(last) {
        return None;
    }
    let r = right.trim_start();
    let first = r.split_whitespace().next()?;
    let c = capitalized(first)?;
    Some(format!("{c}{}", &r[first.len()..]))
}

/// [`capitalize_after_stop`] at every join of `texts` (empty pieces are
/// skipped, as when they are joined).
pub fn capitalize_joins(texts: &mut [String]) {
    let mut prev: Option<usize> = None;
    for k in 0..texts.len() {
        if texts[k].trim().is_empty() {
            continue;
        }
        if let Some(p) = prev {
            if let Some(r) = capitalize_after_stop(&texts[p], &texts[k]) {
                texts[k] = r;
            }
        }
        prev = Some(k);
    }
}

/// Text-only join heuristic (candidate (a), kept for the bench): when
/// `left` ends with a period and `right` starts with a capitalised word
/// other than "I", drop the period and lowercase the word.
#[doc(hidden)]
pub fn smooth_join(left: &str, right: &str) -> Option<(String, String)> {
    let l = left.trim_end().strip_suffix('.')?;
    let first = right.split_whitespace().next()?;
    let mut ch = first.chars();
    let c = ch.next()?;
    if !c.is_uppercase() || first.trim_matches(|c: char| !c.is_alphanumeric()) == "I" {
        return None;
    }
    let lowered: String = c.to_lowercase().chain(ch).collect();
    Some((
        l.to_string(),
        format!("{lowered}{}", &right.trim_start()[first.len()..]),
    ))
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
    fn splice_takes_the_bridge_words_around_the_cut() {
        // Parakeet: a sentence break the whole decode does not have.
        let (l, r) = splice(
            "And so, my fellow Americans, ask not.",
            "What your country can do for you.",
            "fellow Americans, ask not what your country can",
        )
        .unwrap();
        assert_eq!(l, "And so, my fellow Americans, ask not");
        assert_eq!(r, "what your country can do for you.");
        // whisper tiny: a word the left piece got wrong without context.
        let (l, r) = splice(
            "And so my fellow Americans asked not",
            "what your country can do for you",
            "my fellow Americans ask not what your country",
        )
        .unwrap();
        assert_eq!(l, "And so my fellow Americans ask not");
        assert_eq!(r, "what your country can do for you");
        // The bridge agrees there is a sentence break: unchanged.
        assert_eq!(
            splice("We shipped it.", "Then we slept.", "shipped it. Then we"),
            None
        );
        // Mangled bridge edges are never used.
        let (l, r) = splice(
            "one two three four",
            "five six seven",
            "ree two three four, five six se",
        )
        .unwrap();
        assert_eq!(
            (l.as_str(), r.as_str()),
            ("one two three four,", "five six seven")
        );
        assert_eq!(
            splice(
                "alpha beta.",
                "Gamma delta",
                "x y alpha beta gamma delta z w"
            ),
            Some(("alpha beta".into(), "gamma delta".into()))
        );
        // The two words at each bridge edge keep the pieces' case and
        // punctuation: they were all the bridge had no context for.
        assert_eq!(splice("alpha beta.", "Gamma delta", "a beta gamma d"), None);
        assert_eq!(splice("alpha beta.", "Gamma delta", "alpha beta"), None);
        // An alignment that slips on common words is refused (it once
        // duplicated "the man who told").
        assert_eq!(
            splice(
                "because the man who told",
                "the story was so good",
                "x because the man who told the story was so good y",
            ),
            None
        );
        // A word the bridge hears differently outside the anchors is
        // never taken; only the span between anchors is rewritten.
        assert_eq!(
            splice(
                "Mr. Meadowcroft the elder spoke.",
                "Then he",
                "x Mettercroft the elder spoke then he y"
            ),
            Some(("Mr. Meadowcroft the elder spoke".into(), "then he".into()))
        );
        // Inside the span, a different word away from the cut is refused.
        assert_eq!(
            splice(
                "one two three four five.",
                "Six seven",
                "x one tree three four five six seven y"
            ),
            None
        );
        // Edge words are not anchors.
        assert_eq!(splice("alpha beta.", "Gamma delta", "beta gamma"), None);
        assert_eq!(splice("", "x", "x"), None);
    }

    #[test]
    fn splice_keeps_case_next_to_the_bridge_edges() {
        // cliptest 2026-10-23, Parakeet long_028s: the bridge started at
        // "Capacity," and so heard "the Englishman" mid-sentence; the
        // splice copied "the" into the left piece ("capacity. the").
        assert_eq!(
            splice(
                "I believe in the training of people to their highest capacity. \
                 The Englishman here heartily seconded him.",
                "I think that will do, she continued, for the other qualities",
                "Capacity, the Englishman here heartily seconded him. I think that \
                 will do, she continued, for the",
            ),
            None
        );
        // D70: the bridge breaks the sentence at its edge ("fine. The"),
        // where the piece's comma stands; its capital must not stay after
        // that comma ("fine, The next step").
        assert_eq!(
            splice(
                "so it all went fine, the next step is",
                "Ready for review",
                "went fine. The next step is ready for review now ok",
            ),
            Some((
                "so it all went fine, the next step is".into(),
                "ready for review".into()
            ))
        );
        // A rewritten word after a kept sentence end starts upper case.
        assert_eq!(
            splice(
                "a b c d. Then we went",
                "home after that",
                "q d, then we went home after that z",
            ),
            None
        );
    }

    #[test]
    fn respell_cut_takes_the_bridge_case_at_the_cut() {
        // A pause mid-sentence: the right piece, decoded alone, starts
        // with a capital, and splice refuses because a word away from the
        // cut is heard differently ("the" / "a").
        let (l, r) = ("we should look at", "Options for the meeting room");
        let bridge = "x then we should look at options for a meeting room y";
        assert_eq!(splice(l, r, bridge), None);
        assert_eq!(
            respell_cut(l, r, bridge),
            Some((
                "we should look at".into(),
                "options for the meeting room".into()
            ))
        );
        // The bridge hears a sentence break the pieces missed.
        assert_eq!(
            respell_cut(
                "it went fine,",
                "The next part",
                "a b it went fine. The next part c d"
            ),
            Some(("it went fine.".into(), "The next part".into()))
        );
        // A name stays a name when the bridge capitalises it too.
        assert_eq!(
            respell_cut("ask", "Maria first", "a b c ask Maria first d e"),
            None
        );
        // One-word pieces.
        assert_eq!(
            respell_cut("and", "Then", "a b c and then d e"),
            Some(("and".into(), "then".into()))
        );
        // The pair must be clear of the bridge edges, present and unique.
        assert_eq!(
            respell_cut("look at", "Options", "a at options b c d"),
            None
        );
        assert_eq!(
            respell_cut("look at", "Options", "a b c at options d"),
            None
        );
        assert_eq!(
            respell_cut("look at", "Options", "a b c at the options d e"),
            None
        );
        assert_eq!(
            respell_cut("at", "Options", "a b c at options x at options d e"),
            None
        );
        assert_eq!(respell_cut("", "Options", "a b c at options d e"), None);
    }

    #[test]
    fn capitalize_after_a_sentence_end_at_joins() {
        // cliptest 2026-10-23: "figures. that is" (Parakeet ls_1188_M).
        let c = |l: &str, r: &str| capitalize_after_stop(l, r);
        assert_eq!(
            c("ill drawn figures.", "that is merely"),
            Some("That is merely".into())
        );
        assert_eq!(c("Is it?", "yes"), Some("Yes".into()));
        assert_eq!(c("he said \"stop.\"", "then"), Some("Then".into()));
        assert_eq!(c("you said no.", "then"), Some("Then".into()));
        // Not a sentence end, already capitalised, or a mixed-case word.
        assert_eq!(c("capacity,", "the Englishman"), None);
        assert_eq!(c("figures", "that is"), None);
        assert_eq!(c("figures.", "That is"), None);
        assert_eq!(c("figures.", "iPhone sales"), None);
        assert_eq!(c("and so...", "then"), None);
        assert_eq!(c("and so\u{2026}", "then"), None);
        assert_eq!(c("e.g.", "this"), None);
        assert_eq!(c("ask Mr.", "smith"), None);
        assert_eq!(c("", "x"), None);
        let mut t = vec![
            "One.".to_string(),
            String::new(),
            "two".into(),
            "three".into(),
        ];
        capitalize_joins(&mut t);
        assert_eq!(t, ["One.", "", "Two", "three"]);
    }

    #[test]
    fn bridge_range_is_clipped_to_the_pieces() {
        assert_eq!(
            bridge_range(10 * S, 0, 30 * S),
            10 * S - BRIDGE_LEFT_SAMPLES..10 * S + BRIDGE_SAMPLES
        );
        assert_eq!(bridge_range(S, 0, 2 * S), 0..2 * S);
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
