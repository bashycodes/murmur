//! flow-core: shared Rust core of the local Wispr Flow clone (spec §9 v2).
//!
//! See `docs/core-architecture.md`. Entry point: [`FlowCore`].
pub mod align;
pub mod asr;
pub mod audio;
pub mod cleanup;
pub mod config;
pub mod dictionary;
mod engine;
pub mod learn;
#[cfg(feature = "llm")]
pub mod llama;
#[cfg(feature = "parakeet")]
pub mod parakeet;
pub mod postprocess;
#[cfg(feature = "whisper")]
pub mod whisper;

pub use engine::{CommitResult, CommitStatus, CoreError, FlowCore, Timings};
