//! Overlay pill hit-testing (pure). The 512x614 overlay canvas is
//! click-through; in hands-free mode the shell polls the cursor and turns
//! click-through off only while it is over the locked pill, so the ✕/✓
//! buttons work without the transparent canvas eating clicks meant for the
//! app underneath.
//!
//! Geometry mirrors `ui/dist/overlay.css`: the pill is centred horizontally,
//! its bottom 14 CSS px above the canvas bottom; locked (hands-free) = 187 x 36.
//!
//! The "Fix →" / "Choose a model →" link on an error / fell-back bubble is
//! the other clickable spot. Its place depends on the bubble text, so the
//! overlay reports the link's box itself (`overlay_action` `link_rect`,
//! CSS px in the canvas; `null` once the link is gone) and click-through is
//! off over that box too, in any pill state. [`accept_link_rect`] bounds
//! what the page may claim so it can never make the whole canvas opaque.

use serde::Deserialize;

use crate::dictation::OverlayState;

pub const PILL_BOTTOM_GAP: f64 = 14.0;
pub const LOCKED_W: f64 = 187.0;
pub const PILL_H: f64 = 36.0;
/// Slack around the pill so a click on the rounded edge still lands.
pub const HIT_MARGIN: f64 = 3.0;

/// The overlay canvas (CSS px; `tauri.conf.json` window size).
pub const CANVAS_W: f64 = 512.0;
pub const CANVAS_H: f64 = 614.0;
/// Largest link box the overlay may report (CSS px): wider than the
/// longest label ("Choose a model →", ~120 px) but far from the canvas.
pub const MAX_LINK_W: f64 = 200.0;
pub const MAX_LINK_H: f64 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
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

/// Check a link box reported by the overlay page (CSS px in the canvas):
/// finite, non-empty, inside the canvas and no bigger than one link.
pub fn accept_link_rect(r: Rect) -> Result<Rect, String> {
    let ok = [r.left, r.top, r.right, r.bottom]
        .iter()
        .all(|v| v.is_finite())
        && r.left >= 0.0
        && r.top >= 0.0
        && r.right <= CANVAS_W
        && r.bottom <= CANVAS_H
        && r.right > r.left
        && r.bottom > r.top
        && r.right - r.left <= MAX_LINK_W
        && r.bottom - r.top <= MAX_LINK_H;
    if ok {
        Ok(r)
    } else {
        Err(format!("link rect out of bounds: {r:?}"))
    }
}

/// Screen rect (physical px) of the link box `css` (CSS px in the canvas)
/// inside the overlay window `win` at DPI `scale`, with [`HIT_MARGIN`].
pub fn link_screen_rect(win: Rect, scale: f64, css: Rect) -> Rect {
    Rect {
        left: win.left + (css.left - HIT_MARGIN) * scale,
        top: win.top + (css.top - HIT_MARGIN) * scale,
        right: win.left + (css.right + HIT_MARGIN) * scale,
        bottom: win.top + (css.bottom + HIT_MARGIN) * scale,
    }
}

/// Should the overlay ignore the mouse (be click-through) right now?
/// `link` = the bubble link's box (CSS px) while one is shown.
pub fn click_through(
    state: &OverlayState,
    win: Rect,
    scale: f64,
    cursor: (f64, f64),
    link: Option<Rect>,
) -> bool {
    let (x, y) = cursor;
    let on_pill =
        *state == OverlayState::ActiveLocked && locked_pill_rect(win, scale).contains(x, y);
    let on_link = link.is_some_and(|l| link_screen_rect(win, scale, l).contains(x, y));
    !(on_pill || on_link)
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
        assert!(!click_through(
            &OverlayState::ActiveLocked,
            w,
            1.0,
            on_pill,
            None
        ));
        assert!(click_through(
            &OverlayState::ActiveLocked,
            w,
            1.0,
            canvas,
            None
        ));
        assert!(click_through(
            &OverlayState::ActivePtt,
            w,
            1.0,
            on_pill,
            None
        ));
        assert!(click_through(
            &OverlayState::Processing,
            w,
            1.0,
            on_pill,
            None
        ));
    }

    /// "Fix →" at CSS (300, 500)-(340, 518) in the canvas.
    fn link() -> Rect {
        Rect {
            left: 300.0,
            top: 500.0,
            right: 340.0,
            bottom: 518.0,
        }
    }

    #[test]
    fn link_box_is_clickable_in_any_state_while_shown() {
        let w = win(1.0);
        let on_link = (1000.0 + 320.0, 400.0 + 509.0);
        let beside = (1000.0 + 360.0, 400.0 + 509.0);
        let error = OverlayState::Error {
            message: "Dictation disabled — local model not loaded".into(),
        };
        for st in [
            OverlayState::Resting,
            error.clone(),
            OverlayState::ActiveLocked,
        ] {
            assert!(!click_through(&st, w, 1.0, on_link, Some(link())), "{st:?}");
            assert!(click_through(&st, w, 1.0, beside, Some(link())), "{st:?}");
        }
        // Bubble gone (page reported null): click-through again.
        assert!(click_through(&error, w, 1.0, on_link, None));
        assert!(click_through(&OverlayState::Resting, w, 1.0, on_link, None));
    }

    #[test]
    fn link_box_scales_with_dpi_and_keeps_margin() {
        let r = link_screen_rect(win(1.5), 1.5, link());
        assert_eq!(r.left, 1000.0 + 297.0 * 1.5);
        assert_eq!(r.top, 400.0 + 497.0 * 1.5);
        assert_eq!(r.right, 1000.0 + 343.0 * 1.5);
        assert_eq!(r.bottom, 400.0 + 521.0 * 1.5);
        let w = win(1.5);
        let edge = (1000.0 + 341.0 * 1.5, 400.0 + 509.0 * 1.5);
        assert!(!click_through(
            &OverlayState::Resting,
            w,
            1.5,
            edge,
            Some(link())
        ));
    }

    #[test]
    fn page_cannot_claim_more_than_a_link() {
        assert_eq!(accept_link_rect(link()), Ok(link()));
        let r = |left, top, right, bottom| Rect {
            left,
            top,
            right,
            bottom,
        };
        for bad in [
            r(0.0, 0.0, CANVAS_W, CANVAS_H), // whole canvas
            r(10.0, 10.0, 300.0, 30.0),      // too wide
            r(10.0, 10.0, 50.0, 100.0),      // too tall
            r(-5.0, 10.0, 20.0, 30.0),       // outside
            r(500.0, 600.0, 520.0, 620.0),   // past the canvas edge
            r(40.0, 10.0, 20.0, 30.0),       // inverted
            r(20.0, 10.0, 20.0, 30.0),       // empty
            r(f64::NAN, 10.0, 20.0, 30.0),
            r(10.0, 10.0, f64::INFINITY, 30.0),
        ] {
            assert!(accept_link_rect(bad).is_err(), "{bad:?}");
        }
    }
}
