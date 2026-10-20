//! Global hotkeys: platform-independent chord matcher + WH_KEYBOARD_LL hook.

pub mod chord;
#[cfg(windows)]
pub mod win;

pub use chord::*;
