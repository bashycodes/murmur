//! D22 pause-segmented incremental decoding through `FlowCore`, against
//! `BurstEchoAsr` (text = one word per sound burst, a pure function of the
//! audio): no model needed. Spec: docs/proposals/streaming-asr-latency.md §5.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrPurpose, AsrRequest, BurstEchoAsr};
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::{CommitStatus, FlowCore};

const S: usize = 16_000;

/// A phrase: sine at amplitude 1000·k (BurstEchoAsr says "w<k>").
fn phrase(k: i16, ms: usize) -> Vec<i16> {
    (0..ms * S / 1000)
        .map(|i| ((i as f32 * 0.05).sin() * 1000.0 * k as f32) as i16)
        .collect()
}

/// `[(k, speech ms, pause ms)]`, after 300 ms of leading silence.
fn dictation(parts: &[(i16, usize, usize)]) -> (Vec<i16>, String) {
    let mut pcm = vec![0i16; 300 * S / 1000];
    let mut words = Vec::new();
    for &(k, sp, pa) in parts {
        pcm.extend(phrase(k, sp));
        pcm.extend(vec![0i16; pa * S / 1000]);
        words.push(format!("w{k}"));
    }
    (pcm, words.join(" "))
}

/// ~`secs` of 4.5 s phrases with 1 s pauses (a cut after each).
fn long_dictation(secs: usize) -> (Vec<i16>, String) {
    let n = secs.div_ceil(6).max(1);
    let parts: Vec<(i16, usize, usize)> =
        (0..n).map(|i| ((i % 30 + 1) as i16, 4500, 1000)).collect();
    dictation(&parts)
}

fn seg_cfg() -> SessionConfig {
    SessionConfig {
        segment_pause_ms: Some(800),
        ring_capacity_samples: S * 130,
        ..Default::default()
    }
}

fn core_with(asr: Arc<dyn AsrEngine>) -> FlowCore {
    FlowCore::from_engines(asr, None)
}

fn push_chunks(core: &FlowCore, pcm: &[i16], chunk: usize) {
    for c in pcm.chunks(chunk) {
        assert!(core.push_audio(c));
    }
}

/// Push at `speed`× real time in 40 ms chunks.
fn push_paced(core: &FlowCore, pcm: &[i16], speed: f32) {
    let t0 = Instant::now();
    for (i, c) in pcm.chunks(640).enumerate() {
        assert!(core.push_audio(c));
        let due = Duration::from_secs_f32(0.04 * (i + 1) as f32 / speed);
        if let Some(w) = due.checked_sub(t0.elapsed()) {
            std::thread::sleep(w);
        }
    }
}

fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn dictate(
    core: &FlowCore,
    cfg: SessionConfig,
    pcm: &[i16],
    chunk: usize,
) -> flow_core::CommitResult {
    core.start_session(cfg).unwrap();
    push_chunks(core, pcm, chunk);
    core.commit()
}

#[test]
fn segmented_text_equals_whole_decode_and_off_is_unchanged() {
    let (pcm, words) = dictation(&[
        (3, 4200, 900),
        (7, 6100, 1200),
        (12, 3000, 850),
        (5, 2500, 2000),
        (9, 7000, 300),
    ]);
    let core = core_with(Arc::new(BurstEchoAsr::new()));
    let off = dictate(
        &core,
        SessionConfig {
            ring_capacity_samples: S * 130,
            ..Default::default()
        },
        &pcm,
        640,
    );
    assert_eq!(off.asr_text, words);
    assert_eq!(off.timings.segments, 0);
    let on = dictate(&core, seg_cfg(), &pcm, 640);
    assert_eq!(on.status, CommitStatus::RawTranscript, "{on:?}");
    assert_eq!(on.asr_text, off.asr_text);
    assert_eq!(on.final_text, off.final_text);
    assert_eq!(on.segments, off.segments);
    assert!(on.timings.segments >= 3, "{:?}", on.timings);
    // Same JSON apart from timings.
    let strip = |r: &flow_core::CommitResult| {
        let mut v = serde_json::to_value(r).unwrap();
        v.as_object_mut().unwrap().remove("timings");
        v
    };
    assert_eq!(strip(&on), strip(&off));
}

#[test]
fn identical_result_under_any_push_chunking() {
    let (pcm, words) = long_dictation(40);
    let core = core_with(Arc::new(BurstEchoAsr::new()));
    let want = dictate(&core, seg_cfg(), &pcm, 640);
    assert_eq!(want.asr_text, words);
    for chunk in [160, 1600, S, 7919, pcm.len()] {
        let r = dictate(&core, seg_cfg(), &pcm, chunk);
        assert_eq!(r.asr_text, want.asr_text, "chunk {chunk}");
        assert_eq!(r.timings.segments, want.timings.segments, "chunk {chunk}");
        assert_eq!(
            r.timings.asr_audio_ms, want.timings.asr_audio_ms,
            "chunk {chunk}"
        );
    }
}

#[test]
fn commit_latency_is_flat_in_dictation_length() {
    // RTF 0.05 mock, audio pushed at 6× real time: a 60 s whole-clip
    // decode costs 3 s; with segmentation only the tail is left. Each
    // 5.5 s piece also costs a bridge of up to 6 s + 4 s, so at 8× the
    // worker (pieces, bridges, previews) needs more than real time and
    // its queue grows; that says nothing about a 1× dictation.
    let asr = Arc::new(BurstEchoAsr::new().with_rtf(0.05));
    let core = core_with(asr.clone());
    let mut seg_ms = Vec::new();
    for secs in [10, 60] {
        let (pcm, words) = long_dictation(secs);
        core.start_session(seg_cfg()).unwrap();
        push_paced(&core, &pcm, 6.0);
        let r = core.commit();
        assert_eq!(r.asr_text, words, "{secs} s");
        eprintln!("{secs} s: total {} ms, {:?}", r.timings.total_ms, r.timings);
        assert!(r.timings.pre_asr_ms > 0, "{secs} s: {:?}", r.timings);
        seg_ms.push(r.timings.total_ms);
    }
    let (pcm, _) = long_dictation(60);
    core.start_session(SessionConfig {
        ring_capacity_samples: S * 130,
        ..Default::default()
    })
    .unwrap();
    push_paced(&core, &pcm, 6.0);
    let whole = core.commit();
    eprintln!("60 s whole: {:?}", whole.timings);
    // Tail ≈ one 5.5 s piece at RTF 0.05 ≈ 0.3 s, plus at most one segment
    // decode in flight; the whole decode is ≈ 3 s.
    assert!(seg_ms[1] < seg_ms[0] + 1500, "{seg_ms:?}");
    assert!(
        seg_ms[1] * 2 < whole.timings.total_ms,
        "{seg_ms:?} vs {:?}",
        whole.timings
    );
}

#[test]
fn cancel_during_an_in_flight_segment_discards_everything() {
    let asr = Arc::new(BurstEchoAsr::new().with_rtf(2.0));
    let core = core_with(asr.clone());
    let (pcm, _) = long_dictation(12);
    core.start_session(seg_cfg()).unwrap();
    push_chunks(&core, &pcm, 640);
    // A segment decode (≈ 10 s of work) is now in flight.
    assert!(wait_until(10, || asr
        .requests()
        .iter()
        .any(|(_, r)| r.purpose == AsrPurpose::Segment)));
    let t = Instant::now();
    core.cancel();
    assert_eq!(core.partial(), None);
    assert_eq!(core.commit().status, CommitStatus::Error);
    // The next session is clean and not stuck behind the aborted decode.
    let (pcm2, words2) = dictation(&[(4, 1000, 300)]);
    core.start_session(seg_cfg()).unwrap();
    push_chunks(&core, &pcm2, 640);
    let r = core.commit();
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(r.asr_text, words2, "{r:?}");
    assert_eq!(r.timings.segments, 1);
}

#[test]
fn cancel_during_a_segmented_commit_is_cancelled() {
    let asr = Arc::new(BurstEchoAsr::new().with_rtf(1.0));
    let core = Arc::new(core_with(asr));
    let (pcm, _) = long_dictation(12);
    core.start_session(seg_cfg()).unwrap();
    push_chunks(&core, &pcm, pcm.len());
    let c2 = core.clone();
    let h = std::thread::spawn(move || c2.commit());
    std::thread::sleep(Duration::from_millis(300));
    core.cancel();
    let r = h.join().unwrap();
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(r.error.as_deref(), Some("cancelled"));
}

#[test]
fn preview_is_done_segments_plus_the_open_one() {
    let core = core_with(Arc::new(BurstEchoAsr::new()));
    let (a, _) = dictation(&[(3, 4500, 1200)]);
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        ..seg_cfg()
    })
    .unwrap();
    push_chunks(&core, &a, 640);
    assert!(
        wait_until(5, || core.partial().as_deref() == Some("w3")),
        "{:?}",
        core.partial()
    );
    push_chunks(&core, &phrase(8, 1500), 640);
    assert!(
        wait_until(5, || core.partial().as_deref() == Some("w3 w8")),
        "{:?}",
        core.partial()
    );
    let r = core.commit();
    assert_eq!(r.asr_text, "w3 w8");
    assert_eq!(core.partial(), None);
}

#[test]
fn late_context_redecodes_the_pieces_it_changes() {
    let asr = Arc::new(BurstEchoAsr::new());
    let core = core_with(asr.clone());
    let (pcm, words) = long_dictation(18);
    core.start_session(seg_cfg()).unwrap();
    push_chunks(&core, &pcm, 640);
    assert!(wait_until(5, || asr.calls() >= 2));
    core.set_context(FieldContext {
        screen_words: vec!["Kubernetes".into()],
        ..Default::default()
    });
    let r = core.commit();
    assert_eq!(r.asr_text, words);
    // Every piece's text came from a request with the final prompt.
    let reqs = asr.requests();
    let finals: Vec<_> = reqs
        .iter()
        .rev()
        .take(r.timings.segments as usize)
        .collect();
    assert!(
        finals
            .iter()
            .all(|(_, q)| q.initial_prompt.contains("Kubernetes")),
        "{reqs:?}"
    );
}

#[test]
fn later_pieces_get_the_previous_words_as_prior_text() {
    let asr = Arc::new(BurstEchoAsr::new());
    let core = core_with(asr.clone());
    let (pcm, _) = dictation(&[(3, 4500, 1000), (5, 4500, 1000), (7, 1000, 300)]);
    let r = dictate(&core, seg_cfg(), &pcm, pcm.len());
    assert_eq!(r.asr_text, "w3 w5 w7");
    let priors: Vec<String> = asr
        .requests()
        .iter()
        .filter(|(_, q)| q.purpose != AsrPurpose::Partial)
        .filter_map(|(_, q)| q.prior_text.clone())
        .collect();
    // (The first piece and the boundary bridges have none.)
    assert_eq!(priors, ["w3", "w3 w5"]);
}

#[test]
fn bridges_decode_the_audio_around_each_cut() {
    let asr = Arc::new(BurstEchoAsr::new());
    let core = core_with(asr.clone());
    let (pcm, words) = dictation(&[(3, 4500, 1000), (5, 4500, 1000), (7, 4500, 300)]);
    core.start_session(seg_cfg()).unwrap();
    push_chunks(&core, &pcm, 640);
    // Both bridges (4 s each side of a cut) are decoded while recording.
    assert!(wait_until(5, || asr.calls() >= 4), "{}", asr.calls());
    let before = asr.calls();
    let r = core.commit();
    assert_eq!(r.asr_text, words);
    assert_eq!(r.timings.segments, 3);
    // Commit decoded the tail only (pieces and bridges reused).
    assert_eq!(asr.calls(), before + 1, "{:?}", asr.requests());
}

#[test]
fn silence_only_with_segmentation_is_no_speech() {
    let core = core_with(Arc::new(BurstEchoAsr::new()));
    let r = dictate(&core, seg_cfg(), &vec![0i16; 20 * S], 640);
    assert_eq!(r.status, CommitStatus::Empty);
    assert_eq!(r.empty_reason.as_deref(), Some("no_speech"));
}

/// Words like BurstEchoAsr, but the burst at amplitude 5 is whisper's
/// "Thank you." with a high no-speech probability, and the one at 6 gets
/// a trailing "Thank you." appended.
struct ThanksAsr;
impl AsrEngine for ThanksAsr {
    fn transcribe(&self, pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let w = BurstEchoAsr::words(pcm);
        let text = match w.as_str() {
            "w5" => "Thank you.".to_string(),
            "w6" => "w6. Thank you.".to_string(),
            _ => w,
        };
        Ok(AsrOutput {
            text,
            language: Some("en".into()),
            no_speech_prob: Some(0.9),
        })
    }
}

#[test]
fn per_segment_hallucination_guard() {
    let core = core_with(Arc::new(ThanksAsr));
    let (pcm, _) = dictation(&[
        (3, 4500, 1000),
        (5, 4500, 1000),
        (6, 4500, 1000),
        (7, 1000, 300),
    ]);
    let r = dictate(&core, seg_cfg(), &pcm, 640);
    // "w7" follows a sentence end at a join: capitalised.
    assert_eq!(r.asr_text, "w3 w6. W7", "{r:?}");
    // Guard off: everything stays.
    let r = dictate(
        &core,
        SessionConfig {
            hallucination_guard: false,
            ..seg_cfg()
        },
        &pcm,
        640,
    );
    assert_eq!(r.asr_text, "w3 Thank you. W6. Thank you. W7");
}

/// An engine that must not see audio before commit (cloud, D18).
struct Unsegmentable(AtomicUsize);
impl AsrEngine for Unsegmentable {
    fn transcribe(&self, pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(AsrOutput {
            text: BurstEchoAsr::words(pcm),
            ..Default::default()
        })
    }
    fn segmentable(&self) -> bool {
        false
    }
}

#[test]
fn unsegmentable_engine_gets_one_decode_at_commit() {
    let asr = Arc::new(Unsegmentable(AtomicUsize::new(0)));
    let core = core_with(asr.clone());
    let (pcm, words) = long_dictation(30);
    core.start_session(seg_cfg()).unwrap();
    push_chunks(&core, &pcm, 640);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(asr.0.load(Ordering::SeqCst), 0, "decoded before commit");
    let r = core.commit();
    assert_eq!(r.asr_text, words);
    assert_eq!(asr.0.load(Ordering::SeqCst), 1);
    assert_eq!(r.timings.segments, 0);
}

/// Words like BurstEchoAsr, written as a sentence-case model writes a
/// decode heard alone: the first word capitalised, no closing stop. The
/// burst at amplitude 3 is "w3 x" in a decode that also hears "w6" (the
/// bridge) and "w3-x" in the first piece, so the bridge disagrees on a
/// word away from the cut and the wide splice is refused.
struct SentenceCaseAsr;
impl AsrEngine for SentenceCaseAsr {
    fn transcribe(&self, pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let words = BurstEchoAsr::words(pcm);
        let three = if words.contains("w6") { "w3 x" } else { "w3-x" };
        let text = words
            .split(' ')
            .map(|w| if w == "w3" { three } else { w })
            .collect::<Vec<_>>()
            .join(" ");
        let mut c = text.chars();
        let text = c
            .next()
            .map_or(String::new(), |f| f.to_uppercase().chain(c).collect());
        Ok(AsrOutput {
            text,
            language: Some("en".into()),
            no_speech_prob: None,
        })
    }
}

#[test]
fn a_pause_mid_sentence_does_not_capitalise_the_next_piece() {
    // D70: a piece decoded alone starts with a capital; a cut at a pause
    // inside a sentence left it mid-sentence ("w5 W6") whenever the
    // splice was refused. The bridge spells the two words at the cut.
    let group = |ks: std::ops::RangeInclusive<i16>, last_pause: usize| {
        let mut v: Vec<(i16, usize, usize)> = ks.map(|k| (k, 500, 400)).collect();
        v.last_mut().unwrap().2 = last_pause;
        v
    };
    let mut parts = group(1..=5, 1000);
    parts.extend(group(6..=10, 300));
    let (pcm, _) = dictation(&parts);
    let core = core_with(Arc::new(SentenceCaseAsr));
    let r = dictate(&core, seg_cfg(), &pcm, 640);
    assert_eq!(r.timings.segments, 2, "{r:?}");
    assert_eq!(r.asr_text, "W1 w2 w3-x w4 w5 w6 w7 w8 w9 w10", "{r:?}");
}
