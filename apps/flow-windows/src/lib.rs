//! Windows shell of the local Wispr Flow clone.
//!
//! Pure, platform-independent logic (chord matching, dictation state machine,
//! resampling, paste planning, context truncation/bucketing, prefs) lives in
//! plain modules and is unit-tested on any host. Win32 code sits behind
//! `#[cfg(windows)]` in `win` submodules.

pub mod audio;
pub mod context;
pub mod core_api;
pub mod dictation;
pub mod hotkeys;
pub mod input;
