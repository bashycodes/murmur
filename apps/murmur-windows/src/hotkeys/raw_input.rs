//! Raw keyboard input registered by our own process (pure part).
//!
//! The Tauri event loop (tao) registers our process for raw mouse and
//! keyboard input by default, for its device events, delivered while one
//! of our windows has focus. The likely cause of the dictation key doing
//! nothing in setup's "Now try it" step (and in any Murmur window) while it
//! worked in every other app: with a raw keyboard registration in the
//! process, Windows appears not to call our `WH_KEYBOARD_LL` hook for keys
//! going to our own window. The app turns tao's device events off
//! ([`APP_DEVICE_EVENTS`]); a check run once the event loop is up says so
//! in the log if a raw keyboard registration is back.

/// When the event loop may receive device events (tao registers raw input
/// for them). Mirrors `tauri::DeviceEventFilter` without needing Tauri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEvents {
    /// Never: no raw input registration (`DeviceEventFilter::Always`).
    Off,
    /// While a window of ours has focus: tao's default (`Unfocused`).
    WhileFocused,
    /// Always, focused or not (`DeviceEventFilter::Never`).
    Always,
}

/// What the app asks the event loop for. Nothing in the app uses device
/// events, and any raw keyboard registration can starve the hook.
pub const APP_DEVICE_EVENTS: DeviceEvents = DeviceEvents::Off;

/// Does this setting register raw keyboard input in our process?
pub fn registers_raw_keyboard(d: DeviceEvents) -> bool {
    d != DeviceEvents::Off
}

/// HID usage page "Generic Desktop" and its keyboard usage.
pub const USAGE_PAGE_GENERIC: u16 = 0x01;
pub const USAGE_KEYBOARD: u16 = 0x06;

/// Does this list of `(usage page, usage)` raw-input registrations include
/// the keyboard?
pub fn includes_keyboard(devices: &[(u16, u16)]) -> bool {
    devices.contains(&(USAGE_PAGE_GENERIC, USAGE_KEYBOARD))
}

/// The log line for a raw keyboard registration found in our process.
pub const KEYBOARD_REGISTERED: &str =
    "raw keyboard input is registered in this process: the dictation key may not work while a Murmur window has focus";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_registers_no_raw_keyboard_input() {
        // tao's default (WhileFocused) is what left the try step deaf.
        assert_eq!(APP_DEVICE_EVENTS, DeviceEvents::Off);
        assert!(!registers_raw_keyboard(APP_DEVICE_EVENTS));
        assert!(registers_raw_keyboard(DeviceEvents::WhileFocused));
        assert!(registers_raw_keyboard(DeviceEvents::Always));
    }

    #[test]
    fn keyboard_registration_is_found_among_others() {
        assert!(includes_keyboard(&[(0x01, 0x02), (0x01, 0x06)]));
    }

    #[test]
    fn mouse_only_or_nothing_is_not_a_keyboard() {
        assert!(!includes_keyboard(&[]));
        assert!(!includes_keyboard(&[(0x01, 0x02)]));
        assert!(
            !includes_keyboard(&[(0x0C, 0x06)]),
            "keyboard usage on another page"
        );
    }
}
