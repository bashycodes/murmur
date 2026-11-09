//! Meeting transcription over the router (D44): each chunk of a long
//! recording is one ordinary dictation on the route order, with the same
//! fallbacks (server → local → cloud …) as a dictation.
//!
//! The router holds one session at a time: a chunk's `start_session`
//! cancels any open dictation and vice versa. A shell that dictates while
//! a meeting runs gives the meeting its own `Router` (and local engine).

pub use murmur_core::meeting::{
    chunk_session, clock, transcribe_pcm, transcribe_stream, CancelHandle, ChunkTranscriber,
    MeetingError, MeetingParams, MeetingTranscriber, Progress, Segment, Transcript,
};

use murmur_core::config::SessionConfig;
use murmur_core::CommitResult;

use crate::Router;

impl ChunkTranscriber for Router {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        if let Err(e) = self.start_session(cfg.clone()) {
            return crate::router::error_result(e.to_string(), None);
        }
        self.push_audio(pcm);
        self.commit()
    }

    fn abort(&self) {
        self.cancel()
    }
}
