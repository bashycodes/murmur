//! Meeting transcription (D44): a recording of any length (a meeting, a
//! phone call: minutes to hours) turned into timestamped segments.
//!
//! A dictation session is capped by `SessionConfig::max_samples`; a
//! meeting is not one session. [`MeetingTranscriber`] streams the audio
//! through the pause [`Segmenter`] (D22), cuts it into chunks of at most
//! [`MeetingParams::max_ms`] at pauses, and runs each chunk as **one
//! ordinary dictation** through a [`ChunkTranscriber`]: `FlowCore` here,
//! `flow_router::Router` (route order + fallbacks) in flow-router. Only
//! the open chunk is buffered, so memory stays bounded however long the
//! recording is; each finished chunk becomes a [`Segment`] (start, end,
//! text) and is dropped.
//!
//! Chunking rules, all a pure function of the audio (push sizes never
//! move a cut):
//!
//! * a segmenter cut (pause of `pause_ms` once the chunk is `min_ms`
//!   long, or the soft max in the longest recent gap) closes the chunk;
//! * a chunk reaching `max_ms` without a cut is closed at its longest
//!   pause (else its quietest frame) by [`split_long`], never mid-word
//!   when there is any pause;
//! * audio with no speech is never transcribed: once `max_ms` of it is
//!   buffered, all but the last second is dropped.
//!
//! A segment's times are the speech inside its chunk (whole-chunk VAD
//! frame flags), in ms from the start of the recording.
//!
//! Push-based so it serves both a file ([`transcribe_pcm`]) and a live
//! recording (Notetaker, call capture): `push` blocks while a chunk is
//! transcribed, so a live shell feeds it from a worker thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::{SessionConfig, SAMPLE_RATE};
use crate::segment::{split_long, SegmentParams, Segmenter, RECOMMENDED_PAUSE_MS};
use crate::vad::{self, FRAME};
use crate::{CommitResult, CommitStatus, FlowCore};

/// Chunking settings. Defaults: 800 ms pauses, chunks of 8-30 s (Whisper's
/// window is 30 s; longer chunks keep more context than dictation's 4 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeetingParams {
    pub pause_ms: u32,
    pub min_ms: u32,
    pub soft_max_ms: u32,
    /// Hard cap of one chunk (and of the audio ever buffered).
    pub max_ms: u32,
    /// Abort the job after this many chunks in a row failed on every route
    /// (0 = never). A single failed chunk is kept as a [`Segment`] with
    /// `error` set and the job goes on.
    pub max_consecutive_errors: u32,
}

impl Default for MeetingParams {
    fn default() -> Self {
        Self {
            pause_ms: RECOMMENDED_PAUSE_MS,
            min_ms: 8_000,
            soft_max_ms: 25_000,
            max_ms: 30_000,
            max_consecutive_errors: 3,
        }
    }
}

impl MeetingParams {
    fn max_samples(&self) -> usize {
        (self.max_ms.max(self.min_ms).max(2_000) as usize) * SAMPLE_RATE / 1000
    }
}

/// One timestamped piece of a meeting transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Segment {
    /// Start / end of the speech, ms from the start of the recording.
    pub start_ms: u64,
    pub end_ms: u64,
    /// The chunk's `final_text` (empty only with `error`).
    pub text: String,
    /// Language the engine reported for this chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `local` / `remote` / `cloud`, when the engine reports a route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Every route failed for this chunk (the commit's error message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How far a job is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    /// Audio handled so far (transcribed, or dropped as silence), ms.
    pub done_ms: u64,
    /// Audio received so far, ms.
    pub received_ms: u64,
    /// Length of the whole recording when known (a file), ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    pub segments: usize,
}

impl Progress {
    /// Percent done (0-100) when the total is known.
    pub fn percent(&self) -> Option<f32> {
        let t = self.total_ms?;
        Some(if t == 0 {
            100.0
        } else {
            (self.done_ms as f32 * 100.0 / t as f32).min(100.0)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MeetingError {
    #[error("cancelled")]
    Cancelled,
    /// `max_consecutive_errors` chunks in a row failed; the last error.
    #[error("transcription failed: {0}")]
    Failed(String),
}

/// What runs one chunk: a whole dictation (start, push, commit) on the
/// normal route. Never fails: errors come back as an `ERROR` result, like
/// `FlowCore::commit` / `Router::commit`.
pub trait ChunkTranscriber {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult;
    /// Abort a `transcribe_chunk` running on another thread (best effort).
    fn abort(&self) {}
}

impl<T: ChunkTranscriber + ?Sized> ChunkTranscriber for Arc<T> {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        (**self).transcribe_chunk(pcm, cfg)
    }
    fn abort(&self) {
        (**self).abort()
    }
}

impl<T: ChunkTranscriber + ?Sized> ChunkTranscriber for &T {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        (**self).transcribe_chunk(pcm, cfg)
    }
    fn abort(&self) {
        (**self).abort()
    }
}

impl ChunkTranscriber for FlowCore {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        if let Err(e) = self.start_session(cfg.clone()) {
            return error_result(e.to_string());
        }
        self.push_audio(pcm);
        self.commit()
    }
    fn abort(&self) {
        self.cancel()
    }
}

fn error_result(msg: String) -> CommitResult {
    let mut r: CommitResult = serde_json::from_str(
        r#"{"status":"ERROR","asr_text":"","final_text":"","language":null,"timings":{"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},"press_enter":false,"fallback_reason":null,"truncated":false,"error":null}"#,
    )
    .expect("static CommitResult JSON");
    r.error = Some(msg);
    r
}

/// The session config every chunk runs with: the caller's dictation
/// settings (languages, dictionary, cleanup) with the per-session limits
/// opened up to the chunk and the dictation-only features off (hands-free
/// endpoint, in-session pause segmentation, partials, "press enter").
pub fn chunk_session(base: &SessionConfig, params: &MeetingParams) -> SessionConfig {
    let max = params.max_samples();
    let mut c = SessionConfig {
        max_samples: base.max_samples.max(max),
        endpoint_silence_ms: None,
        segment_pause_ms: None,
        partial_interval_ms: None,
        press_enter_enabled: false,
        ..base.clone()
    };
    c.fit_ring_to(max);
    c
}

/// Cancels a running job from any thread: no new chunk starts, and the
/// chunk in flight is aborted (`ChunkTranscriber::abort`).
#[derive(Clone)]
pub struct CancelHandle {
    flag: Arc<AtomicBool>,
    abort: Arc<dyn Fn() + Send + Sync>,
}

impl CancelHandle {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        (self.abort)();
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Streaming meeting transcriber (see the module docs).
pub struct MeetingTranscriber<E: ChunkTranscriber> {
    engine: Arc<E>,
    cfg: SessionConfig,
    params: MeetingParams,
    max_samples: usize,
    seg: Segmenter,
    /// Audio of the open chunk, from absolute sample `buf_start`.
    buf: Vec<i16>,
    buf_start: usize,
    /// Segmenter cuts already acted on.
    cuts_seen: usize,
    total: Option<usize>,
    segments: usize,
    errors_in_row: u32,
    cancel: Arc<AtomicBool>,
}

impl<E: ChunkTranscriber + Send + Sync + 'static> MeetingTranscriber<E> {
    /// `cfg` = the user's dictation settings (`languages` decides Hebrew /
    /// English / auto per chunk); see [`chunk_session`].
    pub fn new(engine: Arc<E>, cfg: &SessionConfig, params: MeetingParams) -> Self {
        let max_samples = params.max_samples();
        Self {
            engine,
            cfg: chunk_session(cfg, &params),
            seg: Segmenter::new(SegmentParams {
                pause_ms: params.pause_ms,
                min_ms: params.min_ms,
                soft_max_ms: params.soft_max_ms.min(params.max_ms),
            }),
            params,
            max_samples,
            buf: Vec::with_capacity(max_samples + SAMPLE_RATE),
            buf_start: 0,
            cuts_seen: 0,
            total: None,
            segments: 0,
            errors_in_row: 0,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        let e = self.engine.clone();
        CancelHandle {
            flag: self.cancel.clone(),
            abort: Arc::new(move || e.abort()),
        }
    }
}

impl<E: ChunkTranscriber> MeetingTranscriber<E> {
    /// Length of the whole recording, when known, for [`Progress::percent`].
    pub fn set_total_samples(&mut self, n: usize) {
        self.total = Some(n);
    }

    pub fn progress(&self) -> Progress {
        let ms = |s: usize| (s * 1000 / SAMPLE_RATE) as u64;
        Progress {
            done_ms: ms(self.buf_start),
            received_ms: ms(self.seg.fed()),
            total_ms: self.total.map(ms),
            segments: self.segments,
        }
    }

    /// Samples held right now (the open chunk); never above `max_ms` + 1 s.
    pub fn buffered_samples(&self) -> usize {
        self.buf.len()
    }

    /// Feed the next samples (16 kHz mono, in order). Returns the segments
    /// of the chunks this closed (possibly none).
    pub fn push(&mut self, pcm: &[i16]) -> Result<Vec<Segment>, MeetingError> {
        let mut out = Vec::new();
        // Feed in pieces so a cut is acted on before the buffer outgrows
        // the cap, however large the caller's block is.
        for piece in pcm.chunks(FRAME * 10) {
            self.check_cancel()?;
            let f: Vec<f32> = piece.iter().map(|&s| s as f32 / 32768.0).collect();
            self.seg.push(&f);
            self.buf.extend_from_slice(piece);
            self.drain(&mut out)?;
        }
        Ok(out)
    }

    /// Transcribe what is left (end of the recording). Pushing after this
    /// starts a new chunk at the current position.
    pub fn finish(&mut self) -> Result<Vec<Segment>, MeetingError> {
        let mut out = Vec::new();
        self.check_cancel()?;
        let end = self.buf_start + self.buf.len();
        if self.speech_in_buffer() {
            self.close_at(end, &mut out)?;
        } else {
            self.drop_to(end);
        }
        Ok(out)
    }

    fn check_cancel(&self) -> Result<(), MeetingError> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(MeetingError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn speech_in_buffer(&self) -> bool {
        self.seg.speech_end_sample() > self.buf_start
    }

    fn drain(&mut self, out: &mut Vec<Segment>) -> Result<(), MeetingError> {
        while self.cuts_seen < self.seg.cuts().len() {
            let cut = self.seg.cuts()[self.cuts_seen];
            self.cuts_seen += 1;
            // A cut at or before a forced cut is stale.
            if cut > self.buf_start {
                self.close_at(cut, out)?;
            }
        }
        if self.buf.len() >= self.max_samples {
            if !self.speech_in_buffer() {
                // Silence: drop all but the last second (a speech run
                // starting there is not confirmed yet).
                let keep = SAMPLE_RATE.min(self.buf.len());
                self.drop_to(self.buf_start + self.buf.len() - keep);
            } else {
                let f: Vec<f32> = self.buf.iter().map(|&s| s as f32 / 32768.0).collect();
                let pieces = split_long(&f, self.buf.len() - 1);
                let first = pieces.first().map_or(self.buf.len(), |r| r.end);
                self.close_at(self.buf_start + first, out)?;
            }
        }
        Ok(())
    }

    fn drop_to(&mut self, at: usize) {
        let n = at - self.buf_start;
        self.buf.drain(..n);
        self.buf_start = at;
    }

    /// Transcribe `buf_start..at` as one chunk and drop it.
    fn close_at(&mut self, at: usize, out: &mut Vec<Segment>) -> Result<(), MeetingError> {
        let start = self.buf_start;
        let n = at - start;
        let chunk = &self.buf[..n];
        let f: Vec<f32> = chunk.iter().map(|&s| s as f32 / 32768.0).collect();
        let flags = vad::frame_flags(&f);
        let first = flags.iter().position(|&b| b);
        let Some(first) = first else {
            self.drop_to(at);
            return Ok(());
        };
        let last = flags.iter().rposition(|&b| b).unwrap_or(first);
        let r = self.engine.transcribe_chunk(chunk, &self.cfg);
        self.check_cancel()?;
        let ms = |s: usize| (s * 1000 / SAMPLE_RATE) as u64;
        let seg = Segment {
            start_ms: ms(start + first * FRAME),
            end_ms: ms(start + ((last + 1) * FRAME).min(n)),
            text: r.final_text.trim().to_string(),
            language: r.language.clone(),
            engine: r.route.as_ref().map(|x| x.engine.clone()),
            error: None,
        };
        self.drop_to(at);
        match r.status {
            CommitStatus::Error => {
                let msg = r.error.unwrap_or_else(|| "error".into());
                if msg == "cancelled" && self.cancel.load(Ordering::SeqCst) {
                    return Err(MeetingError::Cancelled);
                }
                self.errors_in_row += 1;
                let cap = self.params.max_consecutive_errors;
                if cap > 0 && self.errors_in_row >= cap {
                    return Err(MeetingError::Failed(msg));
                }
                self.segments += 1;
                out.push(Segment {
                    text: String::new(),
                    error: Some(msg),
                    ..seg
                });
            }
            _ => {
                self.errors_in_row = 0;
                if !seg.text.is_empty() {
                    self.segments += 1;
                    out.push(seg);
                }
            }
        }
        Ok(())
    }
}

/// The whole transcript of a job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<Segment>,
    /// Length of the recording, ms.
    pub duration_ms: u64,
}

impl Transcript {
    /// `[hh:mm:ss.mmm --> hh:mm:ss.mmm] text`, one segment per line.
    pub fn to_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| {
                let t = s
                    .error
                    .as_deref()
                    .map_or(s.text.clone(), |e| format!("[error: {e}]"));
                format!("[{} --> {}] {t}", clock(s.start_ms), clock(s.end_ms))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `hh:mm:ss.mmm`.
pub fn clock(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Run a whole recording from `source` (blocks of 16 kHz mono PCM, e.g. a
/// WAV read block by block): `on_segment` sees each segment as it is
/// made, `on_progress` the progress after every block. `total_samples`
/// (when known) gives `Progress::percent`.
pub fn transcribe_stream<E, I, B>(
    t: &mut MeetingTranscriber<E>,
    source: I,
    total_samples: Option<usize>,
    mut on_segment: impl FnMut(&Segment),
    mut on_progress: impl FnMut(Progress),
) -> Result<Transcript, MeetingError>
where
    E: ChunkTranscriber,
    I: IntoIterator<Item = B>,
    B: AsRef<[i16]>,
{
    if let Some(n) = total_samples {
        t.set_total_samples(n);
    }
    let mut segments = Vec::new();
    for block in source {
        for s in t.push(block.as_ref())? {
            on_segment(&s);
            segments.push(s);
        }
        on_progress(t.progress());
    }
    for s in t.finish()? {
        on_segment(&s);
        segments.push(s);
    }
    let p = t.progress();
    on_progress(p);
    let ms = p.received_ms;
    Ok(Transcript {
        segments,
        duration_ms: ms,
    })
}

/// [`transcribe_stream`] over PCM already in memory (tests, short files).
pub fn transcribe_pcm<E: ChunkTranscriber + Send + Sync + 'static>(
    engine: Arc<E>,
    pcm: &[i16],
    cfg: &SessionConfig,
    params: MeetingParams,
) -> Result<Transcript, MeetingError> {
    let mut t = MeetingTranscriber::new(engine, cfg, params);
    transcribe_stream(
        &mut t,
        pcm.chunks(SAMPLE_RATE),
        Some(pcm.len()),
        |_| {},
        |_| {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const S: usize = SAMPLE_RATE;

    /// Voiced "speech": a 150 Hz tone with a 4 Hz syllable envelope.
    fn voiced(ms: usize) -> Vec<i16> {
        (0..ms * S / 1000)
            .map(|i| {
                let t = i as f32 / S as f32;
                let env = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
                (8000.0 * env * (2.0 * std::f32::consts::PI * 150.0 * t).sin()) as i16
            })
            .collect()
    }

    /// Low room noise (deterministic).
    fn quiet(ms: usize) -> Vec<i16> {
        let mut x: u32 = 12345;
        (0..ms * S / 1000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                ((x >> 16) % 61) as i16 - 30
            })
            .collect()
    }

    /// `(gap_ms, speech_ms)` pairs → speech spans in ms + the recording.
    fn recording(bursts: &[(usize, usize)]) -> (Vec<(u64, u64)>, Vec<i16>) {
        let mut pcm = Vec::new();
        let mut spans = Vec::new();
        for &(gap, len) in bursts {
            pcm.extend(quiet(gap));
            let a = (pcm.len() * 1000 / S) as u64;
            pcm.extend(voiced(len));
            spans.push((a, (pcm.len() * 1000 / S) as u64));
        }
        pcm.extend(quiet(1500));
        (spans, pcm)
    }

    #[derive(Default)]
    struct Mock {
        /// (chunk samples, languages) per call.
        calls: Mutex<Vec<(usize, Vec<String>)>>,
        /// Calls (1-based) that fail.
        fail: Vec<usize>,
        aborts: Mutex<usize>,
    }

    impl ChunkTranscriber for Mock {
        fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
            let mut calls = self.calls.lock().unwrap();
            calls.push((pcm.len(), cfg.languages.clone()));
            let n = calls.len();
            let mut r = error_result("server down".into());
            if !self.fail.contains(&n) {
                r.status = CommitStatus::RawTranscript;
                r.error = None;
                r.final_text = format!("chunk {n}");
                r.language = cfg.languages.first().cloned();
            }
            r
        }
        fn abort(&self) {
            *self.aborts.lock().unwrap() += 1;
        }
    }

    fn run_with(
        m: Arc<Mock>,
        pcm: &[i16],
        block: usize,
        cfg: &SessionConfig,
    ) -> Result<Transcript, MeetingError> {
        let mut t = MeetingTranscriber::new(m, cfg, MeetingParams::default());
        transcribe_stream(&mut t, pcm.chunks(block), Some(pcm.len()), |_| {}, |_| {})
    }

    fn run(pcm: &[i16], block: usize) -> (Arc<Mock>, Transcript) {
        let m = Arc::new(Mock::default());
        let t = run_with(m.clone(), pcm, block, &SessionConfig::default()).unwrap();
        (m, t)
    }

    #[test]
    fn segments_line_up_with_the_speech() {
        // 10-14 s utterances, 1.5-3 s apart: one chunk each (min 8 s).
        let (spans, pcm) = recording(&[
            (2000, 12_000),
            (1500, 10_000),
            (3000, 14_000),
            (2000, 11_000),
        ]);
        let (m, tr) = run(&pcm, 640);
        assert_eq!(tr.segments.len(), spans.len(), "{tr:?}");
        for (s, &(a, b)) in tr.segments.iter().zip(&spans) {
            assert!(s.start_ms.abs_diff(a) <= 60, "start {} vs {a}", s.start_ms);
            assert!(s.end_ms.abs_diff(b) <= 60, "end {} vs {b}", s.end_ms);
        }
        assert_eq!(tr.segments[2].text, "chunk 3");
        assert_eq!(m.calls.lock().unwrap().len(), 4);
        assert_eq!(tr.duration_ms, (pcm.len() * 1000 / S) as u64);
        assert!(tr.to_text().starts_with("[00:00:02.0"), "{}", tr.to_text());
    }

    #[test]
    fn short_utterances_merge_up_to_the_minimum() {
        let (spans, pcm) = recording(&[(1000, 3000), (1000, 3000), (1000, 3000), (1000, 3000)]);
        let (_, tr) = run(&pcm, 4000);
        assert_eq!(tr.segments.len(), 2, "{tr:?}");
        assert!(tr.segments[0].start_ms.abs_diff(spans[0].0) <= 60);
        assert!(tr.segments[1].start_ms.abs_diff(spans[2].0) <= 60);
        assert!(tr.segments[1].end_ms.abs_diff(spans[3].1) <= 60);
    }

    #[test]
    fn cuts_do_not_depend_on_push_sizes() {
        let (_, pcm) = recording(&[(2000, 12_000), (900, 9_000), (3000, 40_000), (2000, 6_000)]);
        let a = run(&pcm, 640).1;
        assert_eq!(a, run(&pcm, 160_000).1);
        assert_eq!(a, run(&pcm, 7).1);
    }

    #[test]
    fn nonstop_speech_is_capped_at_max_ms() {
        let mut pcm = quiet(500);
        pcm.extend(voiced(100_000));
        pcm.extend(quiet(1000));
        let m = Arc::new(Mock::default());
        let mut t = MeetingTranscriber::new(
            m.clone(),
            &SessionConfig::default(),
            MeetingParams::default(),
        );
        let mut peak = 0;
        for b in pcm.chunks(S / 2) {
            t.push(b).unwrap();
            peak = peak.max(t.buffered_samples());
        }
        t.finish().unwrap();
        let calls = m.calls.lock().unwrap();
        assert!(calls.len() >= 4, "{}", calls.len());
        assert!(calls.iter().all(|c| c.0 <= 30 * S), "{calls:?}");
        assert!(peak <= 30 * S, "peak {peak}");
        // No speech is lost or sent twice.
        let sent: usize = calls.iter().map(|c| c.0).sum();
        assert!(sent >= 100 * S && sent <= pcm.len(), "{sent}");
    }

    #[test]
    fn silence_is_dropped_untranscribed_and_memory_stays_bounded() {
        // 10 min of room noise between two utterances.
        let (spans, pcm) = recording(&[(1000, 10_000), (600_000, 10_000)]);
        let m = Arc::new(Mock::default());
        let mut t = MeetingTranscriber::new(
            m.clone(),
            &SessionConfig::default(),
            MeetingParams::default(),
        );
        t.set_total_samples(pcm.len());
        let mut peak = 0;
        let mut segs = Vec::new();
        for b in pcm.chunks(S) {
            segs.extend(t.push(b).unwrap());
            peak = peak.max(t.buffered_samples());
        }
        segs.extend(t.finish().unwrap());
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!(segs[1].start_ms.abs_diff(spans[1].0) <= 60);
        assert_eq!(m.calls.lock().unwrap().len(), 2);
        assert!(peak <= 31 * S, "peak {peak}");
        assert_eq!(t.progress().percent(), Some(100.0));
    }

    #[test]
    fn progress_rises_to_100_percent() {
        let (_, pcm) = recording(&[(1000, 12_000), (1000, 12_000), (1000, 12_000)]);
        let m = Arc::new(Mock::default());
        let mut t = MeetingTranscriber::new(m, &SessionConfig::default(), MeetingParams::default());
        let mut seen = Vec::new();
        transcribe_stream(
            &mut t,
            pcm.chunks(S),
            Some(pcm.len()),
            |_| {},
            |p| seen.push(p.percent().unwrap()),
        )
        .unwrap();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert!(seen[0] < 50.0);
        assert_eq!(*seen.last().unwrap(), 100.0);
        assert_eq!(Progress::default().percent(), None);
    }

    #[test]
    fn cancel_stops_the_job_and_aborts_the_chunk_in_flight() {
        let (_, pcm) = recording(&[(1000, 12_000), (1000, 12_000), (1000, 12_000)]);
        let m = Arc::new(Mock::default());
        let mut t = MeetingTranscriber::new(
            m.clone(),
            &SessionConfig::default(),
            MeetingParams::default(),
        );
        let h = t.cancel_handle();
        let mut segs = 0;
        let r = transcribe_stream(
            &mut t,
            pcm.chunks(S),
            None,
            |_| {
                segs += 1;
                h.cancel();
            },
            |_| {},
        );
        assert_eq!(r, Err(MeetingError::Cancelled));
        assert_eq!(segs, 1);
        assert_eq!(m.calls.lock().unwrap().len(), 1);
        assert_eq!(*m.aborts.lock().unwrap(), 1);
        assert!(t.cancel_handle().is_cancelled());
    }

    #[test]
    fn a_failed_chunk_is_kept_and_the_job_goes_on() {
        let (_, pcm) = recording(&[(1000, 12_000), (1000, 12_000), (1000, 12_000)]);
        let m = Arc::new(Mock {
            fail: vec![2],
            ..Default::default()
        });
        let tr = run_with(m, &pcm, S, &SessionConfig::default()).unwrap();
        assert_eq!(tr.segments.len(), 3);
        assert_eq!(tr.segments[1].error.as_deref(), Some("server down"));
        assert_eq!(tr.segments[1].text, "");
        assert_eq!(tr.segments[2].text, "chunk 3");
        assert!(tr.to_text().contains("[error: server down]"));
    }

    #[test]
    fn three_failed_chunks_in_a_row_end_the_job() {
        let (_, pcm) = recording(&[(1000, 12_000); 5]);
        let m = Arc::new(Mock {
            fail: vec![2, 3, 4],
            ..Default::default()
        });
        let r = run_with(m.clone(), &pcm, S, &SessionConfig::default());
        assert_eq!(r, Err(MeetingError::Failed("server down".into())));
        assert_eq!(m.calls.lock().unwrap().len(), 4);
    }

    #[test]
    fn chunks_use_the_dictation_language_and_open_session_limits() {
        let (_, pcm) = recording(&[(1000, 12_000), (1000, 12_000)]);
        let m = Arc::new(Mock::default());
        let cfg = SessionConfig {
            languages: vec!["he".into()],
            max_samples: 5 * 60 * S,
            endpoint_silence_ms: Some(2000),
            segment_pause_ms: Some(800),
            ..Default::default()
        };
        let tr = run_with(m.clone(), &pcm, S, &cfg).unwrap();
        assert!(m.calls.lock().unwrap().iter().all(|c| c.1 == ["he"]));
        assert_eq!(tr.segments[0].language.as_deref(), Some("he"));
        let c = chunk_session(&cfg, &MeetingParams::default());
        assert_eq!(c.languages, ["he"]);
        assert_eq!(c.endpoint_silence_ms, None);
        assert_eq!(c.segment_pause_ms, None);
        assert!(!c.press_enter_enabled);
        assert!(c.ring_capacity_samples > 30 * S);
        assert_eq!(c.max_samples, 5 * 60 * S);
    }

    #[test]
    fn a_silent_recording_has_no_segments() {
        let pcm = quiet(90_000);
        let (m, tr) = run(&pcm, S);
        assert!(tr.segments.is_empty());
        assert!(m.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn clock_formats_hours() {
        assert_eq!(clock(3_723_004), "01:02:03.004");
    }
}
