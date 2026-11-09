//! Windows shell of Murmur, the self-hosted dictation app.
//!
//! Pure, platform-independent logic (chord matching, dictation state machine,
//! resampling, paste planning, context truncation/bucketing, prefs) lives in
//! plain modules and is unit-tested on any host. Win32 code sits behind
//! `#[cfg(windows)]` in `win` submodules.

#[cfg(all(feature = "app", windows))]
pub mod app;
pub mod audio;
pub mod cloud;
pub mod context;
pub mod controller;
pub mod core_adapter;
pub mod core_api;
pub mod credentials;
pub mod debug_log;
pub mod dev_settings;
pub mod dictation;
pub mod dictionary;
pub mod ducking;
pub mod history;
pub mod hotkeys;
pub mod input;
pub mod insights;
pub mod keyboard_language;
pub mod learn;
pub mod meetings;
pub mod model_download;
pub mod model_import;
pub mod notes;
pub mod old_data;
pub mod openrouter;
pub mod overlay;
pub mod paste;
pub mod prefs;
pub mod recent_apps;
pub mod router_adapter;
pub mod secrets;
pub mod server_admin;
#[cfg(feature = "remote")]
pub mod server_library;
pub mod shell;
pub mod single_instance;
pub mod sounds;
pub mod suggestions;
pub mod transforms;
