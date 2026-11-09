//! Meeting jobs for the shells (D44, D46): one long recording (a call
//! recording on Android) through [`flow_core::meeting`] on an engine handle
//! the job owns, so a dictation on the shell's own engine never shares a
//! session with it.
//!
//! JSON in and out, like [`crate::json_api`]; the JNI layer
//! (`nativeMeeting*`, JNI_CONTRACT.md "Meeting transcription") is a thin
//! wrapper. `push` / `finish` block while a chunk transcribes and run on
//! the job's worker thread; `progress` and `cancel` never wait for them.

use std::sync::{Mutex, MutexGuard};

use flow_core::config::SessionConfig;
use flow_core::CommitResult;
use flow_router::meeting::{
    CancelHandle, ChunkTranscriber, MeetingError, MeetingParams, MeetingTranscriber, Progress,
    Segment,
};

use crate::FlowHandle;

/// A core handle transcribes on the device only (`FlowCore`); a router
/// handle takes the route order with every fallback (`Router`).
impl ChunkTranscriber for FlowHandle {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        match self {
            FlowHandle::Core(c) => c.transcribe_chunk(pcm, cfg),
            FlowHandle::Router(r) => r.transcribe_chunk(pcm, cfg),
        }
    }

    fn abort(&self) {
        self.cancel()
    }
}

/// One running job: the transcriber (used by the worker thread) and what
/// other threads may touch without waiting for a chunk.
pub struct MeetingJob {
    t: Mutex<MeetingTranscriber<FlowHandle>>,
    cancel: CancelHandle,
    progress: Mutex<Progress>,
}

/// Error kinds: `bad_config` (open), `cancelled`, `failed` (every route
/// failed for `max_consecutive_errors` chunks in a row).
pub type JobError = (&'static str, String);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic inside a chunk (caught at the FFI boundary) must not make the
    // job unusable for cancel / progress / close.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn job_error(e: MeetingError) -> JobError {
    match e {
        MeetingError::Cancelled => ("cancelled", "cancelled".into()),
        MeetingError::Failed(m) => ("failed", format!("transcription failed: {m}")),
    }
}

fn segments_json(s: &[Segment]) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "[]".into())
}

impl MeetingJob {
    /// `session_json` = the dictation `SessionConfig` (languages, dictionary,
    /// cleanup); `params_json` = `MeetingParams` (`{}` or empty = defaults).
    pub fn open(
        engine: FlowHandle,
        session_json: &str,
        params_json: &str,
    ) -> Result<Self, JobError> {
        let cfg: SessionConfig = serde_json::from_str(session_json)
            .map_err(|e| ("bad_config", format!("bad_config: bad session config: {e}")))?;
        let params: MeetingParams = if params_json.trim().is_empty() {
            MeetingParams::default()
        } else {
            serde_json::from_str(params_json)
                .map_err(|e| ("bad_config", format!("bad_config: bad meeting params: {e}")))?
        };
        let t = MeetingTranscriber::new(std::sync::Arc::new(engine), &cfg, params);
        let cancel = t.cancel_handle();
        let progress = Mutex::new(t.progress());
        Ok(Self {
            t: Mutex::new(t),
            cancel,
            progress,
        })
    }

    /// Length of the recording in 16 kHz samples, for `Progress::percent`.
    pub fn set_total_samples(&self, n: usize) {
        let mut t = lock(&self.t);
        t.set_total_samples(n);
        *lock(&self.progress) = t.progress();
    }

    /// Feed 16 kHz mono PCM; the segments of the chunks this closed (JSON array).
    pub fn push(&self, pcm: &[i16]) -> Result<String, JobError> {
        let mut t = lock(&self.t);
        let r = t.push(pcm);
        *lock(&self.progress) = t.progress();
        r.map(|s| segments_json(&s)).map_err(job_error)
    }

    /// End of the recording: the last segments (JSON array).
    pub fn finish(&self) -> Result<String, JobError> {
        let mut t = lock(&self.t);
        let r = t.finish();
        *lock(&self.progress) = t.progress();
        r.map(|s| segments_json(&s)).map_err(job_error)
    }

    /// `Progress` JSON as of the last push (never waits for a chunk).
    pub fn progress(&self) -> String {
        serde_json::to_string(&*lock(&self.progress)).unwrap_or_else(|_| "{}".into())
    }

    /// From any thread: no new chunk starts and the one in flight is aborted;
    /// the running `push` / `finish` then fails with `cancelled`.
    pub fn cancel(&self) {
        self.cancel.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_core::config::ModelConfig;
    use flow_core::FlowCore;

    const S: usize = 16_000;

    fn mock_core(text: &str) -> FlowHandle {
        let cfg: ModelConfig = serde_json::from_value(serde_json::json!({
            "asr_backend": {"mock": {"text": text}}, "asr_path": "", "threads": 1
        }))
        .unwrap();
        FlowHandle::Core(FlowCore::load_models(&cfg).unwrap())
    }

    /// Voiced "speech": a 150 Hz tone with a 4 Hz syllable envelope.
    fn voiced(ms: usize) -> Vec<i16> {
        (0..ms * S / 1000)
            .map(|i| {
                let t = i as f32 / S as f32;
                let env = 0.55 + 0.45 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
                (8000.0 * env * (2.0 * std::f32::consts::PI * 150.0 * t).sin()) as i16
            })
            .collect()
    }

    fn session() -> String {
        serde_json::to_string(&SessionConfig::default()).unwrap()
    }

    #[test]
    fn a_long_recording_becomes_timestamped_segments() {
        let job = MeetingJob::open(mock_core("hello there"), &session(), "{}").unwrap();
        let mut pcm = voiced(12_000);
        pcm.extend(vec![0i16; S * 2]);
        pcm.extend(voiced(12_000));
        job.set_total_samples(pcm.len());
        let mut segs = Vec::new();
        for block in pcm.chunks(S) {
            let s: Vec<Segment> = serde_json::from_str(&job.push(block).unwrap()).unwrap();
            segs.extend(s);
        }
        let s: Vec<Segment> = serde_json::from_str(&job.finish().unwrap()).unwrap();
        segs.extend(s);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!(
            segs.iter().all(|s| s.text.contains("hello there")),
            "{segs:?}"
        );
        assert!(
            segs[0].start_ms < 1000 && segs[1].start_ms >= 13_000,
            "{segs:?}"
        );
        let p: Progress = serde_json::from_str(&job.progress()).unwrap();
        assert_eq!(p.total_ms, Some(26_000));
        assert_eq!(p.done_ms, 26_000);
        assert_eq!(p.segments, 2);
    }

    #[test]
    fn cancel_fails_the_next_push_with_cancelled() {
        let job = MeetingJob::open(mock_core("x"), &session(), "").unwrap();
        job.push(&voiced(1000)).unwrap();
        job.cancel();
        assert_eq!(job.push(&voiced(1000)).unwrap_err().0, "cancelled");
        assert_eq!(job.finish().unwrap_err().0, "cancelled");
    }

    #[test]
    fn bad_json_is_bad_config() {
        let e = MeetingJob::open(mock_core("x"), "{", "{}").err().unwrap();
        assert_eq!(e.0, "bad_config");
        let e = MeetingJob::open(mock_core("x"), &session(), "\"nope\"")
            .err()
            .unwrap();
        assert_eq!(e.0, "bad_config");
    }
}
