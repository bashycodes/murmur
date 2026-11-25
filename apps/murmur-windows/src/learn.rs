//! Learn from edits, platform-free.
//!
//! After a paste the shell keeps re-reading the focused control (UIA on
//! Windows, `context/uia.rs`) for up to [`WatchOpts::window_ms`]. The
//! dictated region is located between two anchors taken from the field
//! context at dictation start (the text just before and just after the
//! caret), so edits elsewhere in the document don't count. When the
//! watch ends, the last region is word-aligned against what was pasted
//! (`murmur_core::learn_edit`), and the surviving substitutions become
//! **suggestions** the user must accept (`suggestions.rs`): nothing is
//! ever added to the dictionary automatically.
//!
//! End reasons: anchor mismatch / region removed / paste never observed don't count (no
//! learning); a timeout or a focus change keep the last observation, and a
//! cleared field (a chat box after send) keeps the last good one (D110).
//!
//! Timing (D110): the field is read every [`WatchOpts::poll_ms`] (250 ms by
//! default) at a fixed rate, so a fix that stays in the field for 300 ms is
//! seen before a send or clear. When focus leaves, the old control is read
//! once more ([`FieldReader::final_read`]). A clear can't be read ahead of:
//! the app empties the field in reply to Enter or its send button, and UIA
//! only reports a text change after it happened.
//!
//! Placeholders: a control that reports its hint as its value while empty
//! is read as empty when the text equals its UIA Name or HelpText (read
//! once when the watch starts, [`without_placeholder`]). A hint shown only
//! by drawing it, or one exposed nowhere else, can't be told apart from
//! typed text.

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

    /// The watched control's text after [`Read::FocusChanged`], read without
    /// the focus check (focus is already elsewhere). `None` when the control
    /// is gone or this reader can't tell.
    fn final_read(&mut self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchOpts {
    /// Total observation window; long enough to fix a word and move on.
    pub window_ms: u64,
    /// The pasted text must show up in the field within this time.
    pub baseline_ms: u64,
    /// Time between the starts of two reads (fixed rate, never below
    /// [`MIN_POLL_MS`]).
    pub poll_ms: u64,
}

/// Default re-read interval (D110; was 1000 ms).
pub const DEFAULT_POLL_MS: u64 = 250;
/// Cost bound: at most one field read per this many ms.
pub const MIN_POLL_MS: u64 = 250;

impl Default for WatchOpts {
    fn default() -> Self {
        WatchOpts {
            window_ms: 45_000,
            baseline_ms: 3_000,
            poll_ms: DEFAULT_POLL_MS,
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
    /// The whole field became empty (sent or cleared): the last good
    /// observation before it counts, the empty field never does.
    Cleared,
}

impl EndReason {
    /// Whether the last observation may be learned from (an anchor
    /// mismatch or a removed region says nothing reliable about the edit).
    pub fn counts(self) -> bool {
        matches!(
            self,
            EndReason::Timeout | EndReason::FocusChanged | EndReason::Cleared
        )
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
    /// The last region that wasn't a pure deletion from the one before
    /// (Backspace / Delete anywhere, nothing typed): what a clear learns
    /// from, so deleting text on the way to an empty field isn't read as a
    /// correction.
    last_good: Option<String>,
}

/// `next` is `prev` with characters removed and none added (a subsequence).
/// Accepted trade-off: a fix that only deletes ("colour" → "color") made
/// right before a clear isn't learned; after a timeout or a focus change it
/// still is (those learn from the last region).
fn is_deletion_of(next: &str, prev: &str) -> bool {
    if next.len() >= prev.len() {
        return false;
    }
    let mut rest = prev.chars();
    next.chars().all(|c| rest.any(|p| p == c))
}

/// The read text, or `""` when it is only the field's placeholder: some
/// controls report their hint (UIA Name or HelpText) as the value while
/// empty, which would otherwise look like the dictation was replaced.
pub fn without_placeholder(text: String, hints: &[String]) -> String {
    let t = text.trim();
    if !t.is_empty() && hints.iter().any(|h| h.trim() == t) {
        String::new()
    } else {
        text
    }
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
            last_good: None,
        }
    }

    fn remember(&mut self, r: String) {
        if !self
            .last_good
            .as_deref()
            .is_some_and(|g| is_deletion_of(&r, g))
        {
            self.last_good = Some(r.clone());
        }
        self.last_region = Some(r);
    }

    /// Focus left the control; `final_text` is its text read once more
    /// afterwards ([`FieldReader::final_read`]). A final text whose region
    /// can't be found is ignored (the last observation stands); an empty one
    /// is a clear.
    pub fn focus_left(&mut self, final_text: Option<&str>) -> Outcome {
        if let (true, Some(t)) = (self.baseline_seen, final_text) {
            if normalize(t).is_empty() {
                return self.outcome(EndReason::Cleared);
            }
            if let Some(r) = region(t, &self.anchors, self.pasted_len) {
                let r = normalize(r);
                if !r.is_empty() {
                    self.remember(r);
                }
            }
        }
        self.outcome(EndReason::FocusChanged)
    }

    pub fn observe(&mut self, now_ms: u64, read: Read) -> Step {
        let elapsed = now_ms.saturating_sub(self.started_ms);
        let baseline_over = elapsed >= self.opts.baseline_ms.min(self.opts.window_ms);
        match read {
            Read::FocusChanged => return self.finish(EndReason::FocusChanged),
            Read::Unavailable => {}
            Read::Text(t) if self.baseline_seen && normalize(&t).is_empty() => {
                return self.finish(EndReason::Cleared);
            }
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
                            self.remember(r);
                        } else if baseline_over {
                            return self.finish(EndReason::PasteNotObserved);
                        }
                    } else if r.is_empty() {
                        return self.finish(EndReason::RegionRemoved);
                    } else {
                        self.remember(r);
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
        Step::Done(self.outcome(reason))
    }

    fn outcome(&self, reason: EndReason) -> Outcome {
        let last = if reason == EndReason::Cleared {
            &self.last_good
        } else {
            &self.last_region
        };
        let edited = if reason.counts() && self.baseline_seen {
            last.clone()
                .filter(|r| *r != self.pasted_norm && !r.is_empty())
        } else {
            None
        };
        Outcome { reason, edited }
    }
}

/// Drive `reader` until the watch ends. Reads start every
/// [`WatchOpts::poll_ms`] (fixed rate, so a slow read doesn't stretch the
/// gap a quick fix can slip through), never closer than [`MIN_POLL_MS`].
/// `now` / `sleep` are injected so tests run in virtual time.
pub fn run_watch(
    reader: &mut dyn FieldReader,
    watch: &mut EditWatch,
    now: &dyn Fn() -> u64,
    sleep: &dyn Fn(u64),
) -> Outcome {
    let period = watch.opts.poll_ms.max(MIN_POLL_MS);
    loop {
        let started = now();
        let step = match reader.read() {
            Read::FocusChanged => {
                let last = reader.final_read();
                Step::Done(watch.focus_left(last.as_deref()))
            }
            read => watch.observe(started, read),
        };
        if let Step::Done(o) = step {
            return o;
        }
        let spent = now().saturating_sub(started);
        sleep(period.saturating_sub(spent).max(1));
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
        let f = "Dear Bob, I use cora note daily. Thanks";
        let r = region(f, &a("Dear Bob, ", " Thanks"), 30);
        assert_eq!(r, Some("I use cora note daily."));
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
        let mut w = EditWatch::new("I use cora note daily. ", a("Hi. ", ""), 0, FAST);
        assert_eq!(
            w.observe(0, Read::Text("Hi. I use cora note daily. ".into())),
            Step::Continue
        );
        assert_eq!(
            w.observe(3_000, Read::Text("Hi. I use Korra Note daily.".into())),
            Step::Continue
        );
        assert_eq!(
            w.observe(10_000, Read::Text("Hi. I use Korra Note daily.".into())),
            Step::Done(Outcome {
                reason: EndReason::Timeout,
                edited: Some("I use Korra Note daily.".into())
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
    fn a_clear_without_a_fix_learns_nothing() {
        let mut w = EditWatch::new("ship it", a("", ""), 0, FAST);
        w.observe(0, Read::Text("ship it".into()));
        assert_eq!(
            w.observe(800, Read::Text(" \r\n".into())),
            Step::Done(Outcome {
                reason: EndReason::Cleared,
                edited: None
            }),
            "the clear itself is not an edit"
        );
    }

    #[test]
    fn deleting_only_the_dictation_does_not_count() {
        let mut w = EditWatch::new("ship it", a("Note: ", ""), 0, FAST);
        w.observe(0, Read::Text("Note: ship it".into()));
        assert_eq!(
            w.observe(800, Read::Text("Note: ".into())),
            Step::Done(Outcome {
                reason: EndReason::RegionRemoved,
                edited: None
            })
        );
    }

    #[test]
    fn a_clear_keeps_the_fix_seen_before_it() {
        // Anchors around the caret are gone once the field empties: still a
        // clear, not an anchor mismatch.
        let mut w = EditWatch::new("teh plan", a("Re: ", " ok"), 0, FAST);
        w.observe(0, Read::Text("Re: teh plan ok".into()));
        w.observe(250, Read::Text("Re: the plan ok".into()));
        assert_eq!(
            w.observe(500, Read::Text(String::new())),
            Step::Done(Outcome {
                reason: EndReason::Cleared,
                edited: Some("the plan".into())
            })
        );
    }

    #[test]
    fn backspacing_to_empty_is_not_a_fix() {
        let mut w = EditWatch::new("see you at noon", a("", ""), 0, FAST);
        w.observe(0, Read::Text("see you at noon".into()));
        w.observe(250, Read::Text("see you at no".into()));
        w.observe(500, Read::Text("see you".into()));
        assert_eq!(
            w.observe(750, Read::Text(String::new())),
            Step::Done(Outcome {
                reason: EndReason::Cleared,
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

    /// The field as a function of virtual time; focus leaves at `focus_ms`.
    struct Timed<'a> {
        t: &'a Cell<u64>,
        text: fn(u64) -> &'static str,
        focus_ms: u64,
        read_cost_ms: u64,
        reads: Vec<u64>,
    }
    impl FieldReader for Timed<'_> {
        fn read(&mut self) -> Read {
            let now = self.t.get();
            self.reads.push(now);
            self.t.set(now + self.read_cost_ms);
            if now >= self.focus_ms {
                Read::FocusChanged
            } else {
                Read::Text((self.text)(now).into())
            }
        }
        fn final_read(&mut self) -> Option<String> {
            Some((self.text)(self.t.get()).into())
        }
    }

    fn timed(t: &Cell<u64>, text: fn(u64) -> &'static str, focus_ms: u64) -> Outcome {
        let mut r = Timed {
            t,
            text,
            focus_ms,
            read_cost_ms: 40,
            reads: Vec::new(),
        };
        let mut w = EditWatch::new("I use cora daily", a("", ""), 0, WatchOpts::default());
        let o = run_watch(&mut r, &mut w, &|| t.get(), &|ms| t.set(t.get() + ms));
        // Fixed rate: reads start every 250 ms even though each takes 40.
        assert!(
            r.reads.windows(2).all(|p| p[1] - p[0] == 250),
            "{:?}",
            r.reads
        );
        o
    }

    #[test]
    fn a_quick_fix_then_clear_is_learned() {
        // Fixed at 1.01 s, sent 290 ms later.
        let o = timed(
            &Cell::new(0),
            |t| match t {
                0..1_010 => "I use cora daily",
                1_010..1_300 => "I use Korra daily",
                _ => "",
            },
            u64::MAX,
        );
        assert_eq!(o.reason, EndReason::Cleared);
        assert_eq!(o.edited.as_deref(), Some("I use Korra daily"));
    }

    #[test]
    fn a_quick_fix_then_focus_change_is_learned() {
        // Fixed at 1.01 s, focus leaves at 1.1 s, before the next read:
        // the final read of the old control still sees it.
        let o = timed(
            &Cell::new(0),
            |t| {
                if t < 1_010 {
                    "I use cora daily"
                } else {
                    "I use Korra daily"
                }
            },
            1_100,
        );
        assert_eq!(o.reason, EndReason::FocusChanged);
        assert_eq!(o.edited.as_deref(), Some("I use Korra daily"));
    }

    #[test]
    fn no_fix_then_clear_or_focus_change_learns_nothing() {
        let o = timed(
            &Cell::new(0),
            |t| if t < 1_300 { "I use cora daily" } else { "" },
            u64::MAX,
        );
        assert_eq!((o.reason, o.edited), (EndReason::Cleared, None));
        let o = timed(&Cell::new(0), |_| "I use cora daily", 1_100);
        assert_eq!((o.reason, o.edited), (EndReason::FocusChanged, None));
    }

    #[test]
    fn a_final_read_after_focus_left_that_lost_the_anchors_is_ignored() {
        let mut w = EditWatch::new("teh plan", a("Re: ", ""), 0, FAST);
        w.observe(0, Read::Text("Re: teh plan".into()));
        w.observe(250, Read::Text("Re: the plan".into()));
        let o = w.focus_left(Some("something else"));
        assert_eq!(o.reason, EndReason::FocusChanged);
        assert_eq!(o.edited.as_deref(), Some("the plan"));
    }

    #[test]
    fn a_deletion_mid_text_before_a_clear_is_not_a_fix() {
        let mut w = EditWatch::new("see you at noon", a("", ""), 0, FAST);
        w.observe(0, Read::Text("see you at noon".into()));
        w.observe(250, Read::Text("see at noon".into()));
        assert_eq!(
            w.observe(500, Read::Text(String::new())),
            Step::Done(Outcome {
                reason: EndReason::Cleared,
                edited: None
            })
        );
        // A fix then a deletion: the fix is still the last good one.
        let mut w = EditWatch::new("teh plan for today", a("", ""), 0, FAST);
        w.observe(0, Read::Text("teh plan for today".into()));
        w.observe(250, Read::Text("the plan for today".into()));
        w.observe(500, Read::Text("the plan today".into()));
        match w.observe(750, Read::Text(String::new())) {
            Step::Done(o) => assert_eq!(o.edited.as_deref(), Some("the plan for today")),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn deletion_is_a_subsequence_check() {
        assert!(is_deletion_of("see at noon", "see you at noon"));
        assert!(is_deletion_of("", "x"));
        assert!(!is_deletion_of("the plan", "teh plan"), "same length");
        assert!(!is_deletion_of("the pln", "teh plan"), "reordered");
    }

    #[test]
    fn a_placeholder_reads_as_empty() {
        let hints = ["Type a message".to_string(), String::new()];
        assert_eq!(without_placeholder(" Type a message ".into(), &hints), "");
        assert_eq!(
            without_placeholder("hello".into(), &hints),
            "hello",
            "real text stays"
        );
        assert_eq!(without_placeholder(String::new(), &hints), "");
    }

    #[test]
    fn cleared_counts() {
        assert!(EndReason::Cleared.counts());
        assert!(!EndReason::RegionRemoved.counts());
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
            bound_edit("I use korra", "I use Korra. How are you today"),
            "I use Korra."
        );
        assert_eq!(
            bound_edit("meet at noon", "Ok so meet at 12 noon"),
            "meet at 12 noon",
            "leading insertions trimmed, inner ones kept"
        );
        assert_eq!(bound_edit("x", ""), "");
    }
}
