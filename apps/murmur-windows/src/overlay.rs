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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

// ------------------------------------------------------------ placement
//
// The bar is re-placed on every show (it sometimes did not appear): the
// shell picks the monitor (under the cursor, else the foreground window's,
// else the primary) and reads its current work area and DPI; these pure
// functions turn that into the window rect and grade the result for the
// debug log.

/// Screen rect in physical px (a Win32 `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn contains_rect(&self, r: &PxRect) -> bool {
        r.left >= self.left && r.top >= self.top && r.right <= self.right && r.bottom <= self.bottom
    }

    pub fn intersects(&self, r: &PxRect) -> bool {
        self.left < r.right && r.left < self.right && self.top < r.bottom && r.top < self.bottom
    }
}

/// Gap (CSS px) between the canvas bottom and the work-area bottom.
pub const BAR_BOTTOM_GAP: f64 = 2.0;

/// DPI scale of a monitor (`GetDpiForMonitor` effective DPI; 96 = 100 %).
/// 0 (the call failed) counts as 100 %.
pub fn scale_for_dpi(dpi: u32) -> f64 {
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / 96.0
    }
}

/// Window rect (physical px) of the 512x614 canvas on a monitor whose work
/// area is `work` and DPI scale `scale`: horizontally centred, its bottom
/// [`BAR_BOTTOM_GAP`] above the work-area bottom, so above a bottom taskbar
/// and at the screen bottom when the taskbar sits on another edge.
/// Coordinates may be negative (a monitor left of or above the primary).
///
/// At a high scale on a small screen (175 % on 1080p) the canvas is taller
/// (or wider) than the work area; it is then cut to the work area, so the
/// window never reaches into the monitor above or beside. The pill sits at
/// the canvas bottom and stays whole; only the top of the transparent room
/// for the bubble above it is lost.
pub fn bar_rect(work: PxRect, scale: f64) -> PxRect {
    let gap = (BAR_BOTTOM_GAP * scale).round() as i32;
    let w = ((CANVAS_W * scale).round() as i32).min(work.width());
    let h = ((CANVAS_H * scale).round() as i32).min(work.height() - gap);
    let left = work.left + (work.width() - w) / 2;
    let bottom = work.bottom - gap;
    PxRect {
        left,
        top: bottom - h,
        right: left + w,
        bottom,
    }
}

/// Screen rect of the visible pill (the widest, hands-free one) inside the
/// canvas at `win`: what must be on screen for the user to see the bar.
pub fn pill_screen_rect(win: PxRect, scale: f64) -> PxRect {
    let r = locked_pill_rect(
        Rect {
            left: f64::from(win.left),
            top: f64::from(win.top),
            right: f64::from(win.right),
            bottom: f64::from(win.bottom),
        },
        scale,
    );
    PxRect {
        left: r.left.floor() as i32,
        top: r.top.floor() as i32,
        right: r.right.ceil() as i32,
        bottom: r.bottom.ceil() as i32,
    }
}

/// Should the bar be up: a dictation state on it (an error or notice too,
/// for as long as the machine holds it), a meeting recording (it carries
/// the chip) or the "always show" pref.
pub fn bar_wanted(state: &OverlayState, always: bool, meeting_recording: bool) -> bool {
    always || meeting_recording || *state != OverlayState::Resting
}

fn listening(st: &OverlayState) -> bool {
    matches!(
        st,
        OverlayState::ActivePtt | OverlayState::ActiveLocked | OverlayState::ActiveCommand
    )
}

/// Does `next` start a new dictation (or Command Mode instruction)? Then the
/// bar moves to the active monitor and is re-raised even if it was already
/// up (the "always show" bar).
pub fn starts_dictation(prev: &OverlayState, next: &OverlayState) -> bool {
    listening(next) && !listening(prev)
}

/// What the shell saw right after showing the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowCheck {
    pub visible: bool,
    /// The window has `WS_EX_TOPMOST`.
    pub topmost: bool,
    /// Window class (never the title) of a visible topmost window above the
    /// bar in z-order that overlaps the pill, if any.
    pub covered_by: Option<String>,
    /// Actual window rect (`GetWindowRect`).
    pub rect: PxRect,
    /// Work area of the monitor the bar was placed on.
    pub work: PxRect,
    pub scale: f64,
}

impl ShowCheck {
    pub fn pill_on_screen(&self) -> bool {
        self.work
            .contains_rect(&pill_screen_rect(self.rect, self.scale))
    }

    /// The debug-log line for one show: geometry, flags and a window class
    /// at most, never any dictation text or window title.
    pub fn line(&self) -> String {
        let r = |r: PxRect| format!("{},{},{},{}", r.left, r.top, r.right, r.bottom);
        format!(
            "show visible={} topmost={} covered_by={} rect={} work={} scale={:.2} pill_in_work={}",
            self.visible,
            self.topmost,
            self.covered_by.as_deref().unwrap_or("none"),
            r(self.rect),
            r(self.work),
            self.scale,
            self.pill_on_screen()
        )
    }

    /// Why the user may not see the bar although it was asked to show.
    pub fn problem(&self) -> Option<&'static str> {
        if !self.visible {
            Some("bar asked to show but the window is hidden")
        } else if !self.pill_on_screen() {
            Some("bar asked to show but the pill is outside the work area")
        } else if !self.topmost {
            Some("bar asked to show but the window is not topmost")
        } else if self.covered_by.is_some() {
            Some("bar asked to show but another topmost window covers the pill")
        } else {
            None
        }
    }
}

/// Did the page draw the bar after a show? Each watched show gets a number
/// (`begin`), the page acks it once a frame is painted (`ack`), and the
/// shell's watchdog asks `waiting`. Nothing is watched until the page has
/// registered its listener (`page_ready`): a show before that (the "always
/// show" bar at startup) has no one to ack it.
#[derive(Debug, Default)]
pub struct PaintWatch {
    ready: AtomicBool,
    shown: AtomicU64,
    painted: AtomicU64,
}

impl PaintWatch {
    pub fn page_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Number a new show, or `None` while the page is not listening yet.
    pub fn begin(&self) -> Option<u64> {
        if !self.ready.load(Ordering::SeqCst) {
            return None;
        }
        Some(self.shown.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// The page painted after show `seq`. A number never handed out is
    /// refused, so the page cannot silence later warnings.
    pub fn ack(&self, seq: u64) -> Result<(), String> {
        if seq == 0 || seq > self.shown.load(Ordering::SeqCst) {
            return Err(format!("painted ack for show {seq}, which never happened"));
        }
        self.painted.fetch_max(seq, Ordering::SeqCst);
        Ok(())
    }

    /// Show `seq` is still unpainted and still the latest show (a newer
    /// show has its own watchdog).
    pub fn waiting(&self, seq: u64) -> bool {
        self.painted.load(Ordering::SeqCst) < seq && self.shown.load(Ordering::SeqCst) == seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(left: i32, top: i32, right: i32, bottom: i32) -> PxRect {
        PxRect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn bar_sits_bottom_centre_of_primary_above_taskbar() {
        // 1920x1080, 40 px taskbar at the bottom.
        let r = bar_rect(px(0, 0, 1920, 1040), 1.0);
        assert_eq!(r, px(704, 424, 1216, 1038));
    }

    #[test]
    fn bar_follows_a_secondary_monitor_with_negative_coordinates() {
        let left_of_primary = px(-1920, 0, 0, 1040);
        let r = bar_rect(left_of_primary, 1.0);
        assert_eq!(r, px(-1216, 424, -704, 1038));
        assert!(left_of_primary.contains_rect(&pill_screen_rect(r, 1.0)));
        let above_primary = px(0, -1080, 1920, -40);
        let r = bar_rect(above_primary, 1.0);
        assert_eq!(r, px(704, -656, 1216, -42));
        assert!(above_primary.contains_rect(&pill_screen_rect(r, 1.0)));
    }

    #[test]
    fn bar_stays_in_work_area_with_taskbar_on_each_edge() {
        let cases = [
            ("bottom", px(0, 0, 1920, 1040), px(704, 424, 1216, 1038)),
            ("top", px(0, 40, 1920, 1080), px(704, 464, 1216, 1078)),
            ("left", px(48, 0, 1920, 1080), px(728, 464, 1240, 1078)),
            ("right", px(0, 0, 1872, 1080), px(680, 464, 1192, 1078)),
        ];
        for (edge, work, want) in cases {
            let r = bar_rect(work, 1.0);
            assert_eq!(r, want, "taskbar {edge}");
            assert!(
                work.contains_rect(&pill_screen_rect(r, 1.0)),
                "taskbar {edge}"
            );
        }
    }

    #[test]
    fn bar_scales_with_monitor_dpi() {
        assert_eq!(scale_for_dpi(120), 1.25);
        assert_eq!(scale_for_dpi(144), 1.5);
        assert_eq!(scale_for_dpi(0), 1.0);
        // 125 %: a 640x768 canvas; the 2.5 px gap rounds to 3.
        let work = px(0, 0, 1536, 824);
        let r = bar_rect(work, 1.25);
        assert_eq!((r.width(), r.height()), (640, 768));
        assert_eq!(r, px(448, 53, 1088, 821));
        assert!(work.contains_rect(&pill_screen_rect(r, 1.25)));
        // 150 % on a monitor left of the primary: a 768x921 canvas.
        let work = px(-2560, 0, 0, 1380);
        let r = bar_rect(work, 1.5);
        assert_eq!((r.width(), r.height()), (768, 921));
        assert_eq!(r, px(-1664, 456, -896, 1377));
        assert!(work.contains_rect(&pill_screen_rect(r, 1.5)));
    }

    #[test]
    fn only_entering_a_listening_state_starts_a_dictation() {
        use OverlayState::*;
        let notice = Notice {
            message: "No speech heard".into(),
        };
        for prev in [Resting, Processing, notice.clone()] {
            for next in [ActivePtt, ActiveLocked, ActiveCommand] {
                assert!(starts_dictation(&prev, &next), "{prev:?} -> {next:?}");
            }
        }
        // Push-to-talk turned hands-free: same dictation.
        assert!(!starts_dictation(&ActivePtt, &ActiveLocked));
        assert!(!starts_dictation(&ActivePtt, &Processing));
        assert!(!starts_dictation(&Processing, &Resting));
        assert!(!starts_dictation(&Resting, &notice));
    }

    #[test]
    fn an_error_keeps_the_bar_up_until_it_rests() {
        use OverlayState::*;
        let error = Error {
            message: "Microphone unavailable".into(),
        };
        let notice = Notice {
            message: "No speech heard".into(),
        };
        for always in [false, true] {
            assert!(bar_wanted(&error, always, false), "always={always}");
            assert!(bar_wanted(&notice, always, false), "always={always}");
        }
        // Rested: hidden, unless "always show" (or a meeting) keeps it up.
        assert!(!bar_wanted(&Resting, false, false));
        assert!(bar_wanted(&Resting, true, false));
        assert!(bar_wanted(&Resting, false, true));
    }

    #[test]
    fn show_check_flags_hidden_off_screen_and_not_topmost() {
        let work = px(0, 0, 1920, 1040);
        let ok = ShowCheck {
            visible: true,
            topmost: true,
            covered_by: None,
            rect: bar_rect(work, 1.0),
            work,
            scale: 1.0,
        };
        assert_eq!(ok.problem(), None);
        assert_eq!(
            ok.line(),
            "show visible=true topmost=true covered_by=none rect=704,424,1216,1038 \
             work=0,0,1920,1040 scale=1.00 pill_in_work=true"
        );
        let hidden = ShowCheck {
            visible: false,
            ..ok.clone()
        };
        assert!(hidden.problem().unwrap().contains("hidden"));
        // Stale startup placement: placed before the taskbar reserved its
        // space, so the pill now sits under it.
        let stale = ShowCheck {
            rect: bar_rect(px(0, 0, 1920, 1080), 1.0),
            ..ok.clone()
        };
        assert!(!stale.pill_on_screen());
        assert!(stale.problem().unwrap().contains("outside"));
        // Left on a monitor that was unplugged.
        let gone = ShowCheck {
            rect: bar_rect(px(1920, 0, 3840, 1040), 1.0),
            ..ok.clone()
        };
        assert!(gone.problem().unwrap().contains("outside"));
        let below = ShowCheck {
            topmost: false,
            ..ok.clone()
        };
        assert!(below.problem().unwrap().contains("not topmost"));
        let covered = ShowCheck {
            covered_by: Some("ExampleOverlayClass".into()),
            ..ok
        };
        assert!(covered.problem().unwrap().contains("covers the pill"));
        assert!(covered.line().contains("covered_by=ExampleOverlayClass"));
    }

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

    #[test]
    fn tall_canvas_is_cut_to_the_work_area_and_keeps_the_pill() {
        // 1920x1080 with a taskbar: at 175 % (1075 px) and 200 % (1228 px)
        // the canvas is taller than the 996 / 984 px work area.
        for (scale, work) in [(1.75, px(0, 0, 1920, 996)), (2.0, px(0, 0, 1920, 984))] {
            let r = bar_rect(work, scale);
            let gap = (BAR_BOTTOM_GAP * scale).round() as i32;
            assert!(work.contains_rect(&r), "{scale}: {r:?}");
            assert_eq!(r.top, work.top, "{scale}");
            assert_eq!(r.bottom, work.bottom - gap, "{scale}");
            assert_eq!(r.width(), (CANVAS_W * scale).round() as i32, "{scale}");
            assert!(work.contains_rect(&pill_screen_rect(r, scale)), "{scale}");
        }
        // Same on a monitor above the primary: never reaches past its top.
        let above = px(0, -1080, 1920, -84);
        let r = bar_rect(above, 2.0);
        assert!(above.contains_rect(&r));
        // A canvas wider than a narrow (portrait, scaled) work area.
        let narrow = px(0, 0, 900, 1500);
        let r = bar_rect(narrow, 2.0);
        assert_eq!((r.left, r.right), (0, 900));
        assert!(narrow.contains_rect(&pill_screen_rect(r, 2.0)));
    }

    #[test]
    fn paint_watch_waits_only_for_the_latest_show_once_the_page_listens() {
        let w = PaintWatch::default();
        // Startup "always show": the page has no listener yet.
        assert_eq!(w.begin(), None);
        assert!(w.ack(1).is_err());
        w.page_ready();
        let first = w.begin().unwrap();
        assert!(w.waiting(first));
        w.ack(first).unwrap();
        assert!(!w.waiting(first));
        // A newer show supersedes an unacked one.
        let a = w.begin().unwrap();
        let b = w.begin().unwrap();
        assert!(!w.waiting(a));
        assert!(w.waiting(b));
        // A late ack for the older show does not count for the newer one.
        w.ack(a).unwrap();
        assert!(w.waiting(b));
        // Numbers never handed out are refused and change nothing.
        assert!(w.ack(b + 1).is_err());
        assert!(w.ack(0).is_err());
        assert!(w.waiting(b));
        w.ack(b).unwrap();
        assert!(!w.waiting(b));
    }

    #[test]
    fn rects_overlap_only_when_they_share_area() {
        let a = px(0, 0, 10, 10);
        assert!(a.intersects(&px(5, 5, 15, 15)));
        assert!(a.intersects(&px(-5, -5, 1, 1)));
        assert!(!a.intersects(&px(10, 0, 20, 10)), "touching edges");
        assert!(!a.intersects(&px(0, 20, 10, 30)));
    }
}
