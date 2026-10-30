//! Global hotkeys: platform-independent chord matcher + WH_KEYBOARD_LL hook.

pub mod capture;
pub mod chord;
pub mod watchdog;
#[cfg(windows)]
pub mod win;

pub use chord::*;
