//! A recorded meeting → one transcript of "Me" / "Others" turns.
//!
//! The two files are transcribed **separately** with the long-recording
//! engine (D44: chunked at pauses, each chunk one dictation on the route
//! order), one after the other on the same engine (a router holds one
//! session at a time). Separate streams also keep a quiet voice from being
//! dropped next to a loud one in the same chunk (the D44 risk). The
//! segments are then merged by start time.
//!
//! Without headphones the mic also hears the call: a "Me" segment that
//! overlaps an "Others" segment in time and repeats most of its words is
//! that echo, and is dropped ([`drop_echo`]).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use murmur_core::config::SessionConfig;
use murmur_core::CommitResult;
use murmur_router::meeting::{
    transcribe_stream, CancelHandle, ChunkTranscriber, MeetingError, MeetingParams,
    MeetingTranscriber, Segment,
};
use serde::{Deserialize, Serialize};

use super::record::Source;

/// Who spoke: the mic is "Me", the PC's audio "Others".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    Me,
    Others,
}

impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Speaker::Me => "Me",
            Speaker::Others => "Others",
        }
    }

    /// The mic is "Me", the system audio "Others".
    pub fn of(src: Source) -> Speaker {
        match src {
            Source::Mic => Speaker::Me,
            Source::System => Speaker::Others,
        }
    }
}

/// One line of a meeting transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub speaker: Speaker,
    /// ms from the start of the recording.
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Every route failed for this piece (its text is empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn turn(speaker: Speaker, s: &Segment) -> Turn {
    Turn {
        speaker,
        start_ms: s.start_ms,
        end_ms: s.end_ms,
        text: s.text.clone(),
        error: s.error.clone(),
    }
}

/// Both streams' segments as turns in time order (start, then end; "Me"
/// first on a tie), with the mic's echo of the call removed.
pub fn merge(mic: &[Segment], system: &[Segment]) -> Vec<Turn> {
    let others: Vec<Turn> = system.iter().map(|s| turn(Speaker::Others, s)).collect();
    let mut all: Vec<Turn> = mic
        .iter()
        .map(|s| turn(Speaker::Me, s))
        .filter(|t| !drop_echo(t, &others))
        .collect();
    all.extend(others);
    all.sort_by_key(|t| (t.start_ms, t.end_ms, t.speaker == Speaker::Others));
    all
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A "Me" turn that overlaps an "Others" turn in time and whose words are
/// at least 60 % that turn's words (at least 3 of them): the call heard
/// through the speakers, not the user.
pub fn drop_echo(me: &Turn, others: &[Turn]) -> bool {
    let mine = words(&me.text);
    if mine.len() < 3 {
        return false;
    }
    others.iter().any(|o| {
        let overlap = me.start_ms < o.end_ms + 1_000 && o.start_ms < me.end_ms + 1_000;
        if !overlap {
            return false;
        }
        let theirs: HashSet<String> = words(&o.text).into_iter().collect();
        let shared = mine.iter().filter(|w| theirs.contains(*w)).count();
        shared * 10 >= mine.len() * 6
    })
}

/// `h:mm:ss` (or `m:ss` under an hour).
pub fn stamp(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The transcript as plain text, one turn per line:
/// `[0:42] Me: text`.
pub fn to_text(turns: &[Turn]) -> String {
    turns
        .iter()
        .map(|t| {
            let text = match &t.error {
                Some(e) => format!("[not transcribed: {e}]"),
                None => t.text.clone(),
            };
            format!("[{}] {}: {text}", stamp(t.start_ms), t.speaker.label())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Any engine behind a pointer (the shell's meeting router; tests' fakes).
pub struct DynEngine(pub Arc<dyn ChunkTranscriber + Send + Sync>);

impl ChunkTranscriber for DynEngine {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        self.0.transcribe_chunk(pcm, cfg)
    }
    fn abort(&self) {
        self.0.abort()
    }
}

/// Where the running job's cancel handle is kept (so delete / quit can
/// stop it from another thread).
pub type CancelSlot = Arc<Mutex<Option<CancelHandle>>>;

/// Transcribe one file into segments. `on_progress` gets 0-100.
fn transcribe_file(
    engine: &Arc<DynEngine>,
    cfg: &SessionConfig,
    path: &Path,
    params: MeetingParams,
    cancel: &CancelSlot,
    mut on_progress: impl FnMut(f32),
) -> Result<Vec<Segment>, String> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let blocks = murmur_core::audio::WavBlocks::open(path, RATE_BLOCK)?;
    let total = blocks.total_samples();
    let mut job = MeetingTranscriber::new(engine.clone(), cfg, params);
    *cancel.lock().unwrap() = Some(job.cancel_handle());
    let mut read_err = None;
    let source = blocks.map_while(|b| b.map_err(|e| read_err = Some(e)).ok());
    let out = transcribe_stream(
        &mut job,
        source,
        Some(total),
        |_| {},
        |p| on_progress(p.percent().unwrap_or(0.0)),
    );
    *cancel.lock().unwrap() = None;
    if let Some(e) = read_err {
        return Err(e);
    }
    match out {
        Ok(t) => Ok(t.segments),
        Err(MeetingError::Cancelled) => Err(CANCELLED.into()),
        Err(e) => Err(e.to_string()),
    }
}

/// 1 s blocks when reading a recording back.
const RATE_BLOCK: usize = 16_000;

/// The error of a cancelled job.
pub const CANCELLED: &str = "cancelled";

/// Transcribe both files of a meeting (`paths` = mic, system) and merge
/// them. `on_progress` gets 0-100 over the whole job.
pub fn transcribe_meeting(
    engine: Arc<dyn ChunkTranscriber + Send + Sync>,
    cfg: &SessionConfig,
    paths: [&Path; 2],
    params: MeetingParams,
    cancel: &CancelSlot,
    mut on_progress: impl FnMut(f32),
) -> Result<Vec<Turn>, String> {
    let engine = Arc::new(DynEngine(engine));
    let mut out: [Vec<Segment>; 2] = Default::default();
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = transcribe_file(&engine, cfg, paths[i], params, cancel, |p| {
            on_progress((i as f32 * 100.0 + p) / 2.0)
        })?;
    }
    on_progress(100.0);
    Ok(merge(&out[0], &out[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: u64, end: u64, text: &str) -> Segment {
        Segment {
            start_ms: start,
            end_ms: end,
            text: text.into(),
            ..Segment::default()
        }
    }

    #[test]
    fn merge_orders_turns_by_time_and_labels_them() {
        let mic = [
            seg(0, 2_000, "hello all"),
            seg(9_000, 12_000, "sounds good"),
        ];
        let sys = [
            seg(3_000, 8_000, "hi there, can you hear me"),
            seg(9_000, 9_500, "yes"),
        ];
        let t = merge(&mic, &sys);
        let got: Vec<(Speaker, u64, &str)> = t
            .iter()
            .map(|t| (t.speaker, t.start_ms, t.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (Speaker::Me, 0, "hello all"),
                (Speaker::Others, 3_000, "hi there, can you hear me"),
                (Speaker::Others, 9_000, "yes"),
                (Speaker::Me, 9_000, "sounds good"),
            ],
            "tie at 9 s: the shorter first"
        );
        assert_eq!(
            to_text(&t[..2]),
            "[0:00] Me: hello all\n[0:03] Others: hi there, can you hear me"
        );
    }

    #[test]
    fn echo_of_the_call_on_the_mic_is_dropped() {
        let sys = [seg(
            10_000,
            15_000,
            "The quarterly numbers are ready for review",
        )];
        let mic = [
            // The speakers, picked up by the mic, slightly later.
            seg(10_200, 15_100, "the quarterly numbers are ready for"),
            // The user, talking over them about something else.
            seg(12_000, 14_000, "can you share your screen please"),
            // The user repeating a phrase, much later: kept.
            seg(40_000, 42_000, "the quarterly numbers are ready"),
        ];
        let t = merge(&mic, &sys);
        let me: Vec<&str> = t
            .iter()
            .filter(|t| t.speaker == Speaker::Me)
            .map(|t| t.text.as_str())
            .collect();
        assert_eq!(
            me,
            vec![
                "can you share your screen please",
                "the quarterly numbers are ready"
            ]
        );
    }

    #[test]
    fn failed_pieces_stay_in_the_transcript() {
        let mut bad = seg(5_000, 9_000, "");
        bad.error = Some("server unreachable".into());
        let t = merge(&[bad], &[]);
        assert_eq!(t[0].error.as_deref(), Some("server unreachable"));
        assert_eq!(
            to_text(&t),
            "[0:05] Me: [not transcribed: server unreachable]"
        );
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp(0), "0:00");
        assert_eq!(stamp(61_999), "1:01");
        assert_eq!(stamp(3_725_000), "1:02:05");
    }
}
