//! Synthetic meetings only (made-up sentences).

use super::*;
use std::sync::Mutex;

/// A cleaner that answers by task, records its prompts and can fail,
/// be slow, or ignore the abort flag.
struct Fake {
    engine: &'static str,
    fail: Option<CleanError>,
    summary: String,
    /// Notes that repeat the part instead of condensing it.
    echo_notes: bool,
    delay: Duration,
    prompts: Mutex<Vec<CleanPrompt>>,
    aborted: Mutex<u32>,
}

impl Fake {
    fn new(engine: &'static str, summary: &str) -> Arc<Fake> {
        Arc::new(Fake {
            engine,
            fail: None,
            summary: summary.into(),
            echo_notes: false,
            delay: Duration::ZERO,
            prompts: Mutex::default(),
            aborted: Mutex::default(),
        })
    }
    fn failing(engine: &'static str, e: CleanError) -> Arc<Fake> {
        Arc::new(Fake {
            fail: Some(e),
            ..Arc::into_inner(Fake::new(engine, "")).unwrap()
        })
    }
    fn tasks(&self) -> Vec<CleanTask> {
        self.prompts
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.source.as_ref().unwrap().task)
            .collect()
    }
}

impl Cleaner for Fake {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.clean_cancellable(p, d, &AbortFlag::default())
    }
    fn clean_cancellable(
        &self,
        p: &CleanPrompt,
        _d: Instant,
        cancel: &AbortFlag,
    ) -> Result<String, CleanError> {
        self.prompts.lock().unwrap().push(p.clone());
        let t0 = Instant::now();
        while t0.elapsed() < self.delay {
            if cancel.is_set() {
                *self.aborted.lock().unwrap() += 1;
                return Err(CleanError::Backend("cancelled".into()));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(e) = &self.fail {
            return Err(match e {
                CleanError::Backend(s) => CleanError::Backend(s.clone()),
                CleanError::Unavailable(s) => CleanError::Unavailable(s.clone()),
            });
        }
        let src = p.source.as_ref().unwrap();
        Ok(match src.task {
            CleanTask::MeetingNotes if self.echo_notes => src.text.clone(),
            CleanTask::MeetingNotes => format!("- note on {} chars", src.text.chars().count()),
            _ => self.summary.clone(),
        })
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
}

const ANSWER: &str = "SUMMARY:\nThe team moved the release.\nACTION ITEMS:\n- Me: send the notes\n- Others: book a room";

fn transcript(lines: usize) -> String {
    (0..lines)
        .map(|i| {
            format!(
                "{}: we talked about item number {i} on the shared plan",
                if i % 2 == 0 { "Me" } else { "Others" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_short_meeting_is_one_call() {
    let f = Fake::new("local", ANSWER);
    let s = summarise(Some(f.clone()), &transcript(3), &Options::default()).unwrap();
    assert_eq!(s.text, "The team moved the release.");
    assert_eq!(
        s.action_items,
        ["Me: send the notes", "Others: book a room"]
    );
    assert_eq!((s.engine.as_str(), s.calls), ("local", 1));
    assert_eq!(s.language.as_deref(), Some("en"));
    assert_eq!(f.tasks(), [CleanTask::MeetingSummary]);
    let p = &f.prompts.lock().unwrap()[0];
    assert!(p.user.contains("Others: we talked about item number 1"));
    assert!(p.system.contains("write everything else in English"));
}

#[test]
fn a_long_meeting_is_mapped_per_chunk_then_reduced() {
    let f = Fake::new("local", ANSWER);
    let t = transcript(200); // ~11k chars: 5 chunks of <= 2500
    let s = summarise(Some(f.clone()), &t, &Options::default()).unwrap();
    let tasks = f.tasks();
    let notes = tasks
        .iter()
        .filter(|t| **t == CleanTask::MeetingNotes)
        .count();
    assert_eq!(notes, chunks(&t, SMALL_CONTEXT_CHARS).len());
    assert!(notes >= 4, "{notes}");
    assert_eq!(tasks.last(), Some(&CleanTask::MeetingSummary));
    assert_eq!(s.calls as usize, notes + 1);
    for p in f.prompts.lock().unwrap().iter() {
        let src = p.source.as_ref().unwrap();
        assert!(src.text.chars().count() <= SMALL_CONTEXT_CHARS);
    }
}

#[test]
fn notes_that_never_shrink_give_up_after_max_rounds() {
    let f = Arc::new(Fake {
        echo_notes: true,
        ..Arc::into_inner(Fake::new("local", ANSWER)).unwrap()
    });
    let t = transcript(100); // ~5.5k chars: 3 chunks of <= 2500
    let parts = chunks(&t, SMALL_CONTEXT_CHARS).len();
    let e = summarise(Some(f.clone()), &t, &Options::default()).unwrap_err();
    assert_eq!(e, SummaryError::Failed("the meeting is too long".into()));
    // Every round condensed the parts; no summary call was made.
    assert_eq!(f.tasks(), vec![CleanTask::MeetingNotes; parts * MAX_ROUNDS]);
}

#[test]
fn the_cloud_takes_bigger_chunks() {
    let f = Fake::new("cloud", ANSWER);
    let s = summarise(Some(f.clone()), &transcript(200), &Options::default()).unwrap();
    assert_eq!(s.calls, 1, "fits one cloud call");
    assert_eq!(f.tasks(), [CleanTask::MeetingSummary]);
}

#[test]
fn a_summary_needing_too_many_cloud_calls_fails_before_spending() {
    let f = Fake::new("cloud", ANSWER);
    // Just over the cap: MAX_CLOUD_CALLS chunks of notes + the summary.
    let t = transcript(MAX_CLOUD_CALLS * CLOUD_CHUNK_CHARS / 50);
    assert!(chunks(&t, CLOUD_CHUNK_CHARS).len() >= MAX_CLOUD_CALLS);
    let e = summarise(Some(f.clone()), &t, &Options::default()).unwrap_err();
    assert!(e.to_string().contains("limit of 20 calls"), "{e}");
    assert!(f.tasks().is_empty(), "no call was made");
    // The same meeting on this PC's LLM has no cap.
    let local = Fake::new("local", ANSWER);
    assert!(summarise(Some(local), &t, &Options::default()).is_ok());
}

#[test]
fn chunks_cut_at_turns_and_never_exceed_the_limit() {
    let t = "Me: one two three\nOthers: four five six\nMe: seven";
    assert_eq!(
        chunks(t, 40),
        ["Me: one two three\nOthers: four five six", "Me: seven"]
    );
    // A turn longer than a chunk is cut at spaces, a word longer at any char.
    let long = format!("Me: {} {}", "word ".repeat(30).trim(), "x".repeat(25));
    for c in chunks(&long, 20) {
        assert!(c.chars().count() <= 20, "{c:?}");
    }
    let joined: String = chunks(&long, 20).concat().replace([' ', '\n'], "");
    assert_eq!(joined, long.replace(' ', ""));
    // Hebrew counts characters, not bytes.
    let he = "אני: ".to_string() + &"שלום ".repeat(10);
    assert_eq!(chunks(&he, 60).len(), 1);
}

#[test]
fn the_order_falls_through_to_the_next_engine() {
    let local = Fake::failing("local", CleanError::Backend("prompt too long".into()));
    let server = Fake::failing(
        "server",
        CleanError::Unavailable("server_no_summary".into()),
    );
    let cloud = Fake::new("cloud", ANSWER);
    let chain =
        crate::cleanup::ChainCleaner::build(vec![local.clone(), server.clone(), cloud.clone()]);
    let s = summarise(chain, &transcript(4), &Options::default()).unwrap();
    assert_eq!(s.engine, "cloud");
    assert_eq!(local.tasks().len(), 1);
    assert_eq!(server.tasks().len(), 1);
    // Every engine failing: the last reason.
    let chain = crate::cleanup::ChainCleaner::build(vec![local.clone(), server.clone()]);
    assert_eq!(
        summarise(chain, &transcript(4), &Options::default()),
        Err(SummaryError::Failed("server_no_summary".into()))
    );
}

#[test]
fn nothing_to_run_on() {
    assert_eq!(
        summarise(None, "Me: hello there", &Options::default()),
        Err(SummaryError::NoEngine)
    );
    let f = Fake::new("local", ANSWER);
    assert_eq!(
        summarise(Some(f), "  \n ", &Options::default()),
        Err(SummaryError::Empty)
    );
}

#[test]
fn a_hebrew_meeting_is_summarised_in_hebrew() {
    let t = "Me: נדבר על התקציב של הרבעון הבא\nOthers: בסדר, אני אשלח טבלה מחר";
    assert_eq!(meeting_language(t), Some("he"));
    let he = "SUMMARY:\nדיברו על התקציב.\nACTION ITEMS:\n- אחרים: לשלוח טבלה מחר";
    let f = Fake::new("local", he);
    let s = summarise(Some(f.clone()), t, &Options::default()).unwrap();
    assert_eq!(s.text, "דיברו על התקציב.");
    assert_eq!(s.action_items, ["אחרים: לשלוח טבלה מחר"]);
    assert_eq!(s.language.as_deref(), Some("he"));
    let p = &f.prompts.lock().unwrap()[0];
    assert!(p.system.contains("write everything else in Hebrew"));
    assert!(p.examples[0].1.contains("להכין את המצגת"), "Hebrew example");
    assert!(p
        .user
        .ends_with("Write the answer in Hebrew, like the meeting."));
}

#[test]
fn a_hebrew_meeting_answered_in_english_is_asked_again() {
    let t = "Me: נדבר על התקציב של הרבעון הבא";
    let f = Fake::new("local", ANSWER); // always English
    let s = summarise(Some(f.clone()), t, &Options::default()).unwrap();
    assert_eq!(f.tasks().len(), 2, "one retry");
    assert!(f.prompts.lock().unwrap()[1].user.contains("Hebrew only"));
    assert_eq!(s.calls, 2);
    assert_eq!(
        s.text, "The team moved the release.",
        "kept when the retry drifts too"
    );
}

#[test]
fn parse_accepts_decorated_and_hebrew_markers() {
    let (s, i) = parse("**Summary:** A short call.\n\n### Action items\n1. Me: write it up\n2) Others: check dates");
    assert_eq!(s, "A short call.");
    assert_eq!(i, ["Me: write it up", "Others: check dates"]);
    let (s, i) = parse("סיכום:\nשיחה קצרה.\nמשימות:\n- אין");
    assert_eq!((s.as_str(), i.len()), ("שיחה קצרה.", 0));
    let (s, i) = parse("SUMMARY:\nNothing decided.\nACTION ITEMS:\n- None.");
    assert_eq!((s.as_str(), i.len()), ("Nothing decided.", 0));
    // No markers: all summary.
    let (s, i) = parse("Just a paragraph.");
    assert_eq!((s.as_str(), i.len()), ("Just a paragraph.", 0));
    // A summary line that merely starts with the word is not a marker.
    let (s, _) = parse("SUMMARY:\nSummary of costs went well");
    assert_eq!(s, "Summary of costs went well");
}

#[test]
fn the_request_rebuilds_the_same_prompt_on_the_server() {
    let p = build_prompt(CleanTask::MeetingNotes, "Me: hello", Some("he"));
    let req = p.source.clone().unwrap();
    let wire = serde_json::to_string(&*req).unwrap();
    assert!(wire.contains(r#""task":"meeting_notes""#), "{wire}");
    let back: CleanRequest = serde_json::from_str(&wire).unwrap();
    let again = back.prompt_for(PromptScope::FULL);
    assert_eq!((again.system, again.user), (p.system, p.user));
    // A cleanup request has no task on the wire.
    let c = CleanRequest::default();
    assert!(!serde_json::to_string(&c).unwrap().contains("task"));
}

#[test]
fn a_dictation_preempts_the_call_in_flight_and_it_is_asked_again() {
    let f = Arc::new(Fake {
        delay: Duration::from_millis(400),
        ..Arc::into_inner(Fake::new("local", ANSWER)).unwrap()
    });
    let busy = Arc::new(AtomicBool::new(false));
    let opts = Options {
        yield_to: Some(busy.clone()),
        ..Options::default()
    };
    let b = busy.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        b.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        b.store(false, Ordering::SeqCst);
    });
    let s = summarise(Some(f.clone()), &transcript(2), &opts).unwrap();
    t.join().unwrap();
    assert_eq!(s.text, "The team moved the release.");
    assert_eq!(*f.aborted.lock().unwrap(), 1, "the first call gave way");
    assert_eq!(f.tasks().len(), 2);
}

#[test]
fn cancel_stops_the_call() {
    let f = Arc::new(Fake {
        delay: Duration::from_secs(5),
        ..Arc::into_inner(Fake::new("local", ANSWER)).unwrap()
    });
    let opts = Options::default();
    let c = opts.cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        c.store(true, Ordering::SeqCst);
    });
    let t0 = Instant::now();
    assert_eq!(
        summarise(Some(f), &transcript(2), &opts),
        Err(SummaryError::Cancelled)
    );
    assert!(t0.elapsed() < Duration::from_secs(2));
}
