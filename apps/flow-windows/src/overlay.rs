//! Overlay pill hit-testing (pure). The 512x614 overlay canvas is
//! click-through; in hands-free mode the shell polls the cursor and turns
//! click-through off only while it is over the locked pill, so the ✕/✓
//! buttons work without the transparent canvas eating clicks meant for the
//! app underneath (Wispr does the same with `setShape` / alpha hit-testing).
//!
//! Geometry mirrors `ui/dist/overlay.css`: the pill is centred horizontally,
//! its bottom 14 CSS px above the canvas bottom; locked (hands-free) = 187 x 36.

use crate::dictation::OverlayState;

pub const PILL_BOTTOM_GAP: f64 = 14.0;
pub const LOCKED_W: f64 = 187.0;
pub const PILL_H: f64 = 36.0;
/// Slack around the pill so a click on the rounded edge still lands.
pub const HIT_MARGIN: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Screen rect (physical px) of the locked pill inside the overlay window
/// whose outer rect is `win` and DPI scale is `scale`.
pub fn locked_pill_rect(win: Rect, scale: f64) -> Rect {
    let cx = (win.left + win.right) / 2.0;
    let half = (LOCKED_W / 2.0 + HIT_MARGIN) * scale;
    let bottom = win.bottom - (PILL_BOTTOM_GAP - HIT_MARGIN) * scale;
    let top = win.bottom - (PILL_BOTTOM_GAP + PILL_H + HIT_MARGIN) * scale;
    Rect {
        left: cx - half,
        top,
        right: cx + half,
        bottom,
    }
}

/// Should the overlay ignore the mouse (be click-through) right now?
pub fn click_through(state: &OverlayState, win: Rect, scale: f64, cursor: (f64, f64)) -> bool {
    match state {
        OverlayState::ActiveLocked => !locked_pill_rect(win, scale).contains(cursor.0, cursor.1),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(scale: f64) -> Rect {
        Rect {
            left: 1000.0,
            top: 400.0,
            right: 1000.0 + 512.0 * scale,
            bottom: 400.0 + 614.0 * scale,
        }
    }

    #[test]
    fn locked_pill_is_bottom_centre() {
        let r = locked_pill_rect(win(1.0), 1.0);
        assert_eq!(r.left, 1256.0 - 93.5 - 3.0);
        assert_eq!(r.right, 1256.0 + 93.5 + 3.0);
        assert_eq!(r.bottom, 1014.0 - 11.0);
        assert_eq!(r.top, 1014.0 - 53.0);
    }

    #[test]
    fn scales_with_dpi() {
        let r = locked_pill_rect(win(1.5), 1.5);
        assert!((r.right - r.left - (187.0 + 6.0) * 1.5).abs() < 1e-9);
    }

    #[test]
    fn click_through_only_off_over_locked_pill() {
        let w = win(1.0);
        let on_pill = (1256.0, 1014.0 - 29.0);
        let canvas = (1100.0, 600.0);
        assert!(!click_through(&OverlayState::ActiveLocked, w, 1.0, on_pill));
        assert!(click_through(&OverlayState::ActiveLocked, w, 1.0, canvas));
        assert!(click_through(&OverlayState::ActivePtt, w, 1.0, on_pill));
        assert!(click_through(&OverlayState::Processing, w, 1.0, on_pill));
    }
}
