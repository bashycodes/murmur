//! Learn from edits, platform-free.
//!
//! After a paste the shell keeps re-reading the focused control (UIA on
//! Windows, `context/uia.rs`) for up to [`WatchOpts::window_ms`]. The
//! dictated region is located between two anchors taken from the field
//! context at dictation start (the text just before and just after the
//! caret), so edits elsewhere in the document don't count. When the
//! watch ends, the last region is word-aligned against what was pasted
//! (`flow_core::learn_edit`), and the surviving substitutions become
//! **suggestions** the user must accept (`suggestions.rs`): nothing is
//! ever added to the dictionary automatically.
//!
//! End reasons: anchor
//! mismatch / region removed / paste never observed don't count (no
//! learning); a timeout or a focus change keep the last observation.

use crate::context::{head_chars, tail_chars};
use crate::core_api::FieldContext;

/// Characters of before/after context used as anchors. Long enough to be
/// unique in practice, short enough to survive edits far from the paste.
pub const ANCHOR_CHARS: usize = 48;

/// One read of the watched control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// The whole text of the control (still focused).
    Text(String),
    /// Focus moved to another control or app: stop watching.
    FocusChanged,
    /// Transient failure (app busy); try again on the next poll.
    Unavailable,
}

/// Reads the control that was focused when the watch began. Created and
/// used on the watch thread (UIA objects are apartment-bound).
pub trait FieldReader {
    fn read(&mut self) -> Read;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchOpts {
    /// Total observation window (about 30 s).
    pub window_ms: u64,
    /// The pasted text must show up in the field within this time.
    pub baseline_ms: u64,
    pub poll_ms: u64,
}

impl Default for WatchOpts {
    fn default() -> Self {
        WatchOpts {
            window_ms: 45_000,
            baseline_ms: 3_000,
            poll_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Timeout,
    FocusChanged,
    /// The text around the paste changed so the region can't be found.
    AnchorMismatch,
    /// The user deleted the dictated text (or sent it: chat boxes clear).
    RegionRemoved,
    /// The pasted text never appeared between the anchors.
    PasteNotObserved,
}

impl EndReason {
    /// Whether the last observation may be learned from (it drops
    /// anchor_mismatch / transcript_region_removed observations).
    pub fn counts(self) -> bool {
        matches!(self, EndReason::Timeout | EndReason::FocusChanged)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub reason: EndReason,
    /// The corrected region, when it counts and differs from the paste.
    pub edited: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Continue,
    Done(Outcome),
}

/// Text before/after the caret at dictation start.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Anchors {
    pub before: String,
    pub after: String,
}

impl Anchors {
    pub fn from_context(ctx: &FieldContext) -> Anchors {
        Anchors {
            before: tail_chars(&ctx.before, ANCHOR_CHARS),
            after: head_chars(&ctx.after, ANCHOR_CHARS),
        }
    }
}

/// Collapse whitespace so trailing spaces / CRLF from the paste don't count
/// as edits.
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn match_starts(hay: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return vec![0];
    }
    hay.match_indices(needle).map(|(i, _)| i).collect()
}

/// The dictated region of `field`: between the `before` anchor and the
/// next `after` anchor, choosing the pair whose length is closest to
/// `expect_len` (the pasted length) when anchors repeat. `None` when the
/// anchors are gone.
pub fn region<'a>(field: &'a str, a: &Anchors, expect_len: usize) -> Option<&'a str> {
    let mut best: Option<(usize, usize, usize)> = None;
    for p in match_starts(field, &a.before) {
        let start = p + a.before.len();
        let end = if a.after.is_empty() {
            Some(field.len())
        } else {
            field[start..].find(&a.after).map(|i| start + i)
        };
        let Some(end) = end else { continue };
        let score = (end - start).abs_diff(expect_len);
        if best.is_none_or(|(_, _, s)| score < s) {
            best = Some((start, end, score));
        }
    }
    best.map(|(s, e, _)| &field[s..e])
}

/// Observation state machine for one paste. Time is injected.
#[derive(Debug, Clone)]
pub struct EditWatch {
    pasted_norm: String,
    pasted_len: usize,
    anchors: Anchors,
    started_ms: u64,
    opts: WatchOpts,
    baseline_seen: bool,
    last_region: Option<String>,
}

impl EditWatch {
    pub fn new(pasted: &str, anchors: Anchors, started_ms: u64, opts: WatchOpts) -> Self {
        EditWatch {
            pasted_norm: normalize(pasted),
            pasted_len: pasted.len(),
            anchors,
            started_ms,
            opts,
            baseline_seen: false,
            last_region: None,
        }
    }

    pub fn observe(&mut self, now_ms: u64, read: Read) -> Step {
        let elapsed = now_ms.saturating_sub(self.started_ms);
        let baseline_over = elapsed >= self.opts.baseline_ms.min(self.opts.window_ms);
        match read {
            Read::FocusChanged => return self.finish(EndReason::FocusChanged),
            Read::Unavailable => {}
            Read::Text(t) => match region(&t, &self.anchors, self.pasted_len) {
                None if !self.baseline_seen => {
                    if baseline_over {
                        return self.finish(EndReason::PasteNotObserved);
                    }
                }
                None => return self.finish(EndReason::AnchorMismatch),
                Some(r) => {
                    let r = normalize(r);
                    if !self.baseline_seen {
                        if r == self.pasted_norm {
                            self.baseline_seen = true;
                            self.last_region = Some(r);
                        } else if baseline_over {
                            return self.finish(EndReason::PasteNotObserved);
                        }
                    } else if r.is_empty() {
                        return self.finish(EndReason::RegionRemoved);
                    } else {
                        self.last_region = Some(r);
                    }
                }
            },
        }
        if elapsed >= self.opts.window_ms {
            let reason = if self.baseline_seen {
                EndReason::Timeout
            } else {
                EndReason::PasteNotObserved
            };
            return self.finish(reason);
        }
        Step::Continue
    }

    fn finish(&self, reason: EndReason) -> Step {
        let edited = if reason.counts() && self.baseline_seen {
            self.last_region
                .clone()
                .filter(|r| *r != self.pasted_norm && !r.is_empty())
        } else {
            None
        };
        Step::Done(Outcome { reason, edited })
    }
}

/// Drive `reader` until the watch ends. `now` / `sleep` are injected so
/// tests run in virtual time.
pub fn run_watch(
    reader: &mut dyn FieldReader,
    watch: &mut EditWatch,
    now: &dyn Fn() -> u64,
    sleep: &dyn Fn(u64),
) -> Outcome {
    loop {
        if let Step::Done(o) = watch.observe(now(), reader.read()) {
            return o;
        }
        sleep(watch.opts.poll_ms.max(1));
    }
}

/// Cut words the user typed before or after the dictated text out of the
/// observed region, so continuing to type isn't read as a correction of
/// the last pasted word.
///
/// Semi-global word alignment: leading and trailing words of `edited` are
/// free, the window with the lowest edit distance to `pasted` wins (ties:
/// the longer window, so an inserted word inside the dictation stays).
pub fn bound_edit(pasted: &str, edited: &str) -> String {
    let p: Vec<&str> = pasted.split_whitespace().collect();
    let e: Vec<&str> = edited.split_whitespace().collect();
    if p.is_empty() || e.is_empty() {
        return String::new();
    }
    let key = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let pk: Vec<String> = p.iter().map(|w| key(w)).collect();
    let ek: Vec<String> = e.iter().map(|w| key(w)).collect();
    let (n, m) = (p.len(), e.len());
    // dp[i][j]: cost of p[..i] against e[s..j] for the best start s.
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = dp[i - 1][j - 1] + usize::from(pk[i - 1] != ek[j - 1]);
            dp[i][j] = sub.min(dp[i - 1][j] + 1).min(dp[i][j - 1] + 1);
        }
    }
    let best = (0..=m).map(|j| dp[n][j]).min().unwrap_or(0);
    let end = (0..=m).rev().find(|&j| dp[n][j] == best).unwrap_or(m);
    let (mut i, mut j) = (n, end);
    while i > 0 && j > 0 {
        let sub = dp[i - 1][j - 1] + usize::from(pk[i - 1] != ek[j - 1]);
        if dp[i][j] == sub && pk[i - 1] == ek[j - 1] {
            (i, j) = (i - 1, j - 1);
        } else if dp[i][j] == dp[i][j - 1] + 1 {
            j -= 1; // inserted word inside the window
        } else if dp[i][j] == sub {
            (i, j) = (i - 1, j - 1);
        } else {
            i -= 1;
        }
    }
    e[j..end].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    fn a(before: &str, after: &str) -> Anchors {
        Anchors {
            before: before.into(),
            after: after.into(),
        }
    }

    const FAST: WatchOpts = WatchOpts {
        window_ms: 10_000,
        baseline_ms: 2_000,
        poll_ms: 500,
    };

    #[test]
    fn region_between_anchors() {
        let f = "Dear Bob, I use whisper flow daily. Thanks";
        let r = region(f, &a("Dear Bob, ", " Thanks"), 30);
        assert_eq!(r, Some("I use whisper flow daily."));
        assert_eq!(region(f, &a("Dear Alice, ", ""), 10), None);
        assert_eq!(region("hello", &a("", ""), 5), Some("hello"), "empty field");
    }

    #[test]
    fn repeated_anchors_pick_the_closest_length() {
        let f = "x. hi there. x. hi. x.";
        // anchors "x. " … "." twice; expected length 2 ("hi")
        let r = region(f, &a("x. ", "."), 2);
        assert_eq!(r, Some("hi"));
    }

    #[test]
    fn anchors_come_from_the_caret_context() {
        let ctx = FieldContext {
            before: format!("{}END", "a".repeat(100)),
            after: format!("START{}", "b".repeat(100)),
            ..FieldContext::default()
        };
        let an = Anchors::from_context(&ctx);
        assert_eq!(an.before.chars().count(), ANCHOR_CHARS);
        assert!(an.before.ends_with("END"));
        assert!(an.after.starts_with("START"));
    }

    #[test]
    fn edit_then_timeout_yields_the_correction() {
        let mut w = EditWatch::new("I use whisper flow daily. ", a("Hi. ", ""), 0, FAST);
        assert_eq!(
            w.observe(0, Read::Text("Hi. I use whisper flow daily. ".into())),
            Step::Continue
        );
        assert_eq!(
            w.observe(3_000, Read::Text("Hi. I use Wispr Flow daily.".into())),
            Step::Continue
        );
        assert_eq!(
            w.observe(10_000, Read::Text("Hi. I use Wispr Flow daily.".into())),
            Step::Done(Outcome {
                reason: EndReason::Timeout,
                edited: Some("I use Wispr Flow daily.".into())
            })
        );
    }

    #[test]
    fn focus_change_keeps_the_last_observation() {
        let mut w = EditWatch::new("teh plan", a("", ""), 0, FAST);
        w.observe(0, Read::Text("teh plan".into()));
        w.observe(500, Read::Unavailable);
        w.observe(1_000, Read::Text("the plan".into()));
        match w.observe(1_500, Read::FocusChanged) {
            Step::Done(o) => {
                assert_eq!(o.reason, EndReason::FocusChanged);
                assert_eq!(o.edited.as_deref(), Some("the plan"));
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn untouched_paste_learns_nothing() {
        let mut w = EditWatch::new("all good ", a("", ""), 0, FAST);
        w.observe(0, Read::Text("all good".into()));
        assert_eq!(
            w.observe(10_000, Read::Text("all  good\r\n".into())),
            Step::Done(Outcome {
                reason: EndReason::Timeout,
                edited: None
            }),
            "whitespace differences are not edits"
        );
    }

    #[test]
    fn sent_or_deleted_text_does_not_count() {
        let mut w = EditWatch::new("ship it", a("", ""), 0, FAST);
        w.observe(0, Read::Text("ship it".into()));
        assert_eq!(
            w.observe(800, Read::Text(String::new())),
            Step::Done(Outcome {
                reason: EndReason::RegionRemoved,
                edited: None
            })
        );
    }

    #[test]
    fn rewritten_surroundings_are_an_anchor_mismatch() {
        let mut w = EditWatch::new("word", a("Hello ", " bye"), 0, FAST);
        w.observe(0, Read::Text("Hello word bye".into()));
        assert_eq!(
            w.observe(800, Read::Text("totally different".into())),
            Step::Done(Outcome {
                reason: EndReason::AnchorMismatch,
                edited: None
            })
        );
    }

    #[test]
    fn paste_must_appear_within_the_baseline() {
        let mut w = EditWatch::new("late text", a("", ""), 0, FAST);
        assert_eq!(w.observe(0, Read::Text(String::new())), Step::Continue);
        assert_eq!(w.observe(1_000, Read::Text("other".into())), Step::Continue);
        assert_eq!(
            w.observe(2_000, Read::Text("other".into())),
            Step::Done(Outcome {
                reason: EndReason::PasteNotObserved,
                edited: None
            })
        );
    }

    struct Script(VecDeque<Read>);
    impl FieldReader for Script {
        fn read(&mut self) -> Read {
            self.0.pop_front().unwrap_or(Read::FocusChanged)
        }
    }

    #[test]
    fn run_watch_polls_in_virtual_time() {
        let t = Cell::new(0u64);
        let sleeps = RefCell::new(0);
        let mut r = Script(VecDeque::from([
            Read::Text("Hi kubernetes".into()),
            Read::Text("Hi Kubernetes".into()),
        ]));
        let mut w = EditWatch::new("Hi kubernetes", a("", ""), 0, FAST);
        let o = run_watch(&mut r, &mut w, &|| t.get(), &|ms| {
            *sleeps.borrow_mut() += 1;
            t.set(t.get() + ms);
        });
        assert_eq!(o.reason, EndReason::FocusChanged);
        assert_eq!(o.edited.as_deref(), Some("Hi Kubernetes"));
        assert_eq!(*sleeps.borrow(), 2);
    }

    #[test]
    fn bound_edit_drops_text_typed_after_the_dictation() {
        assert_eq!(
            bound_edit("I use wispr", "I use Wispr. How are you today"),
            "I use Wispr."
        );
        assert_eq!(
            bound_edit("meet at noon", "Ok so meet at 12 noon"),
            "meet at 12 noon",
            "leading insertions trimmed, inner ones kept"
        );
        assert_eq!(bound_edit("x", ""), "");
    }
}
