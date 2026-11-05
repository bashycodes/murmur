//! flow-core: shared Rust core of Murmur, the local Wispr Flow clone (spec §9 v2).
//!
//! See `docs/core-architecture.md`. Entry point: [`FlowCore`].
pub mod align;
pub mod asr;
pub mod audio;
pub mod cleanup;
pub mod command;
pub mod config;
pub mod dictionary;
pub mod endpoint;
mod engine;
pub mod guard;
pub mod hallucination;
pub mod insights;
pub mod learn;
pub mod lid;
#[cfg(feature = "llm")]
pub mod llama;
pub mod llm_keep;
pub mod load_timings;
pub mod meeting;
#[cfg(feature = "parakeet")]
pub mod parakeet;
pub mod paste;
pub mod postprocess;
pub mod script_lock;
pub mod segment;
#[cfg(feature = "parakeet")]
pub mod sherpa_online;
pub mod summary;
pub mod transform;
pub mod vad;
#[cfg(feature = "whisper")]
pub mod whisper;
#[cfg(feature = "whisper-plugin")]
pub mod whisper_plugin;

pub use endpoint::EndpointState;
pub use engine::{
    refinish, CommitResult, CommitStatus, CoreError, FlowCore, RouteInfo, SessionPoll, Timings,
    Usd, PEAK_DBFS_FLOOR,
};

/// flow-core version (reported by the hybrid router and server).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Revision of the deterministic pipeline (VAD, hallucination guard,
/// spoken cleanup, Wispr post-processing, paste segments). Bump it whenever
/// any of them changes output for the same input, so a hybrid client and
/// its server can detect pipeline skew (D17).
pub const POSTPROCESS_REV: &str = "2026-10-26.1";
