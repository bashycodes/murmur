//! flow-core: shared Rust core of the local Wispr Flow clone (spec §9 v2).
//!
//! See `docs/core-architecture.md`. Entry point: [`FlowCore`].
pub mod align;
pub mod asr;
pub mod audio;
pub mod cleanup;
pub mod config;
pub mod dictionary;
pub mod endpoint;
mod engine;
pub mod guard;
pub mod hallucination;
pub mod learn;
#[cfg(feature = "llm")]
pub mod llama;
#[cfg(feature = "parakeet")]
pub mod parakeet;
pub mod paste;
pub mod postprocess;
pub mod segment;
pub mod vad;
#[cfg(feature = "whisper")]
pub mod whisper;

pub use endpoint::EndpointState;
pub use engine::{
    CommitResult, CommitStatus, CoreError, FlowCore, RouteInfo, SessionPoll, Timings, Usd,
};

/// flow-core version (reported by the hybrid router and server).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Revision of the deterministic pipeline (VAD, hallucination guard,
/// spoken cleanup, Wispr post-processing, paste segments). Bump it whenever
/// any of them changes output for the same input, so a hybrid client and
/// its server can detect pipeline skew (D17).
pub const POSTPROCESS_REV: &str = "2026-10-24.1";
