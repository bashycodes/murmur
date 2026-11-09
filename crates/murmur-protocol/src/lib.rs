//! murmur-server wire protocol v1 (our own protocol; D8/D17).
//!
//! Normative spec: `docs/protocol.md`. This crate holds the message types,
//! constants and helpers shared by `murmur-remote` (client) and `murmur-server`.
//! It contains no network code.
//!
//! * Binary WebSocket frame = PCM16 LE mono 16 kHz audio for the current
//!   session ([`encode_pcm`] / [`decode_pcm`]).
//! * Text frame = JSON control message tagged by `"t"` ([`ClientMsg`],
//!   [`ServerMsg`]). Unknown fields are ignored everywhere.
//!
//! `SessionConfig`, `FieldContext` and `CommitResult` are murmur-core's serde
//! types verbatim, so additive core fields need no protocol change.

use serde::{Deserialize, Serialize};

pub mod admin;
pub mod library;

pub use murmur_core::cleanup::CleanRequest;
pub use murmur_core::config::{FieldContext, SessionConfig};
pub use murmur_core::CommitResult;

/// Lowest protocol minor this build speaks.
pub const PROTO_MIN: u32 = 1;
/// Highest protocol minor this build speaks.
pub const PROTO_MAX: u32 = 1;
/// WebSocket subprotocol (`Sec-WebSocket-Protocol`).
pub const SUBPROTOCOL: &str = "murmur.v1";
/// The subprotocol's name before the Murmur rename (D83). Same protocol;
/// servers still accept it, and clients still offer it after
/// [`SUBPROTOCOL`], for one transition period (old apps, old servers).
pub const OLD_SUBPROTOCOL: &str = "flow.v1";
/// What a client offers: the new name first, then the old one.
pub const CLIENT_SUBPROTOCOLS: &str = "murmur.v1, flow.v1";
pub const PATH_STREAM: &str = "/v1/stream";
pub const PATH_HEALTH: &str = "/v1/health";
pub const DEFAULT_PORT: u16 = 7313;
/// Largest binary (audio) frame accepted.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Recommended audio frame: 100 ms.
pub const RECOMMENDED_FRAME_SAMPLES: usize = 1600;
/// WebSocket close codes.
pub const CLOSE_UNAUTHORIZED: u16 = 4001;
pub const CLOSE_PROTO_MISMATCH: u16 = 4002;
/// Shortest bearer token a server accepts.
pub const MIN_TOKEN_LEN: usize = 16;
/// WS-level ping interval and the silence after which a peer is dead.
pub const WS_PING_S: u64 = 15;
pub const WS_DEAD_S: u64 = 45;
/// The client keeps an idle connection this long for the next session.
pub const IDLE_KEEPALIVE_S: u64 = 60;

/// Stable error codes (`ServerMsg::Error.code`).
pub mod code {
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const PROTO_MISMATCH: &str = "proto_mismatch";
    pub const BUSY: &str = "busy";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const STALE_SID: &str = "stale_sid";
    pub const NO_SESSION: &str = "no_session";
    pub const TOO_LARGE: &str = "too_large";
    pub const SERVER_ERROR: &str = "server_error";
}

/// Client hello (first message on a connection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hello {
    pub proto_min: u32,
    pub proto_max: u32,
    /// e.g. `"murmur-android 0.2.0"`.
    pub client: String,
    /// Client's murmur-core version.
    pub core: String,
    pub postprocess_rev: String,
}

impl Default for Hello {
    fn default() -> Self {
        Self {
            proto_min: PROTO_MIN,
            proto_max: PROTO_MAX,
            client: String::new(),
            core: murmur_core::VERSION.into(),
            postprocess_rev: murmur_core::POSTPROCESS_REV.into(),
        }
    }
}

/// Server hello reply; also the body of `GET /v1/health`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HelloOk {
    pub proto: u32,
    /// e.g. `"murmur-server 0.1.0"`.
    pub server: String,
    pub core: String,
    pub postprocess_rev: String,
    pub asr_backend: String,
    pub asr_model: String,
    pub llm: bool,
    pub workers: u32,
    pub busy: u32,
    pub max_samples: u64,
    /// Optional server capabilities (additive, 2026-10-23). `"segments"`:
    /// the server's core honours `SessionConfig.segment_pause_ms` (D22).
    /// An older server omits the field and simply ignores the session
    /// field, which still gives the full text.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// Languages with their own ASR model on this server (additive,
    /// 2026-10-30, D33): a session whose single forced language
    /// (`SessionConfig.languages`) matches one decodes on that model
    /// (e.g. `"he"` -> the Hebrew zipformer); everything else, auto-detect
    /// included, on `asr_backend`/`asr_model`. Omitted when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

/// [`HelloOk::features`] value for pause-segmented decoding (D22).
pub const FEATURE_SEGMENTS: &str = "segments";
/// [`HelloOk::features`] value: the server answers text-only cleanup
/// requests ([`ClientMsg::Clean`], D36). Advertised only when it has an
/// LLM loaded.
pub const FEATURE_CLEAN: &str = "clean";
/// [`HelloOk::features`] value: the server's `clean` also runs the
/// meeting summary tasks (`CleanRequest::task`, D53). Advertised with
/// [`FEATURE_CLEAN`]. An older server ignores `task` and would clean the
/// text instead, so a client sends a meeting task only to a server that
/// lists this.
pub const FEATURE_SUMMARY: &str = "summary";

/// Largest `text` a [`ClientMsg::Clean`] may carry (bytes).
pub const MAX_CLEAN_TEXT_BYTES: usize = 32 * 1024;

/// Answer to a [`ClientMsg::Clean`] (D36): the model's text, or why there
/// is none. The client's core judges the text with its own guard, as for
/// any cleaner, so the server does not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleaned {
    pub id: u64,
    /// The model's answer; `None` with `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why there is no text: `llm_timeout`, `llm_error: <code>`, `busy`,
    /// `no_llm`, `too_large`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// LLM wall time on the server.
    pub llm_ms: u64,
    /// Time the request waited for a free worker.
    pub queue_ms: u64,
    /// The server's cleanup model, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerTimings {
    /// Time the session waited for a free worker.
    pub queue_ms: u64,
    /// Audio the server received for the session.
    pub recv_audio_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ErrorMsg {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<u64>,
    pub code: String,
    pub message: String,
    /// The server closes the connection after sending a fatal error.
    pub fatal: bool,
}

impl Default for ErrorMsg {
    fn default() -> Self {
        Self {
            sid: None,
            code: code::SERVER_ERROR.into(),
            message: String::new(),
            fatal: false,
        }
    }
}

/// Client → server control messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello(Hello),
    Start {
        sid: u64,
        #[serde(default)]
        config: SessionConfig,
    },
    Context {
        sid: u64,
        #[serde(default)]
        ctx: FieldContext,
    },
    Commit {
        sid: u64,
    },
    Cancel {
        sid: u64,
    },
    Ping {
        #[serde(default)]
        n: u64,
    },
    /// Text-only AI cleanup (D36), independent of any session: answered
    /// with [`ServerMsg::Cleaned`] carrying the same `id`. Only sent to a
    /// server whose `hello_ok.features` lists [`FEATURE_CLEAN`].
    Clean {
        id: u64,
        #[serde(default)]
        req: Box<CleanRequest>,
    },
}

/// Server → client control messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    HelloOk(HelloOk),
    Started {
        sid: u64,
        #[serde(default)]
        queued: bool,
    },
    Partial {
        sid: u64,
        text: String,
    },
    Result {
        sid: u64,
        result: Box<CommitResult>,
        #[serde(default)]
        server_timings: ServerTimings,
    },
    Cancelled {
        sid: u64,
    },
    Pong {
        #[serde(default)]
        n: u64,
        #[serde(default)]
        busy: u32,
    },
    Error(ErrorMsg),
    /// Answer to [`ClientMsg::Clean`] (D36).
    Cleaned(Cleaned),
}

impl ClientMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ClientMsg serialises")
    }
}

impl ServerMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ServerMsg serialises")
    }
    pub fn error(sid: Option<u64>, code: &str, message: impl Into<String>, fatal: bool) -> Self {
        ServerMsg::Error(ErrorMsg {
            sid,
            code: code.into(),
            message: message.into(),
            fatal,
        })
    }
}

/// Pick the protocol minor for a client range: the highest version both
/// sides speak, or `None` when the ranges are disjoint (`proto_mismatch`).
pub fn negotiate(
    client_min: u32,
    client_max: u32,
    server_min: u32,
    server_max: u32,
) -> Option<u32> {
    let v = client_max.min(server_max);
    (client_min <= client_max && v >= client_min && v >= server_min).then_some(v)
}

/// PCM16 samples → little-endian bytes (one binary frame).
pub fn encode_pcm(pcm: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Why a binary frame was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Odd byte count (`bad_request`).
    OddLength,
    /// Larger than [`MAX_FRAME_BYTES`] (`too_large`).
    TooLarge,
}

/// Little-endian bytes → PCM16 samples.
pub fn decode_pcm(bytes: &[u8]) -> Result<Vec<i16>, FrameError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    if !bytes.len().is_multiple_of(2) {
        return Err(FrameError::OddLength);
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect())
}

/// Milliseconds of 16 kHz audio in `samples`.
pub fn samples_to_ms(samples: u64) -> u64 {
    samples * 1000 / murmur_core::config::SAMPLE_RATE as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_round_trips_little_endian() {
        let pcm = [0i16, 1, -1, i16::MAX, i16::MIN, 0x1234];
        let b = encode_pcm(&pcm);
        assert_eq!(&b[10..12], &[0x34, 0x12]);
        assert_eq!(decode_pcm(&b).unwrap(), pcm);
    }

    #[test]
    fn bad_frames_are_rejected() {
        assert_eq!(decode_pcm(&[1, 2, 3]), Err(FrameError::OddLength));
        assert_eq!(
            decode_pcm(&vec![0; MAX_FRAME_BYTES + 2]),
            Err(FrameError::TooLarge)
        );
        assert_eq!(
            decode_pcm(&vec![0; MAX_FRAME_BYTES]).unwrap().len(),
            MAX_FRAME_BYTES / 2
        );
    }
}
