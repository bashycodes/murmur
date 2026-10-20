//! Orchestrator: runs the dictation machine and executes its effects
//! against a [`Platform`] (OS side) and a [`CoreApi`] (flow-core side).
//!
//! Platform-independent so the whole hotkey → capture → commit → paste
//! loop is integration-tested on Linux with fakes. Slow work (commit, paste,
//! UIA snapshot) runs on short-lived worker threads and reports back through
//! the same input channel, so the loop itself never blocks.

use crate::audio::LevelMeter;
use crate::context::{build_field_context, RawSnapshot};
use crate::core_api::{to_outcome, CommitResult, CommitStatus, CoreApi, FieldContext};
use crate::dictation::{Dictation, Effect, Input, OverlayState, Phase, SessionId, Sound};
use crate::history::{self, History};
use crate::hotkeys::HotkeyAction;
use crate::learn::{self, Anchors, EditWatch, FieldReader, Read};
use crate::prefs::Prefs;
use crate::suggestions::SuggestionStore;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

pub type AudioSink = Box<dyn FnMut(&[i16]) + Send + 'static>;

/// Everything OS-specific the controller needs.
pub trait Platform: Send + Sync + 'static {
    fn play(&self, s: Sound);
    fn duck(&self);
    fn unduck(&self);
    /// Blocking but bounded (UIA with timeout). Called on a worker thread.
    fn snapshot_context(&self) -> Option<RawSnapshot>;
    fn start_capture(&self, device: Option<String>, sink: AudioSink) -> Result<(), String>;
    fn stop_capture(&self);
    /// Blocking (clipboard restore delay). Called on a worker thread.
    fn paste(&self, text: &str, prefs: &Prefs) -> Result<(), String>;
    fn overlay(&self, st: &OverlayState);
    fn level(&self, level: f32);
    fn max_duration_warning(&self, remaining_ms: u64);
    /// Live partial transcript above the pill (`""` = hide).
    fn preview(&self, _text: &str) {}
    /// Spoken "press enter": send Enter after the paste. Worker thread.
    fn send_enter(&self) -> Result<(), String> {
        Ok(())
    }
    /// Plain clipboard copy ("copy last transcript"). Worker thread.
    fn copy_text(&self, _text: &str) -> Result<(), String> {
        Err("clipboard not available".into())
    }
    /// A reader bound to the control focused right now (learn-from-edits),
    /// `None` when it can't be read (no text pattern, password field).
    /// Called and used on the watch thread.
    fn field_reader(&self) -> Option<Box<dyn FieldReader>> {
        None
    }
    /// A history row was added or changed (the History tab refreshes).
    fn history_changed(&self) {}
    /// New dictionary suggestions are waiting (`pending` in total).
    fn suggestions_changed(&self, _pending: usize) {}
}

/// Per-session facts collected for the history row.
#[derive(Default)]
struct SessionInfo {
    context: Option<FieldContext>,
    samples: Arc<AtomicU64>,
    result: Option<CommitResult>,
}

#[derive(Default)]
struct Shared {
    sessions: Mutex<HashMap<SessionId, SessionInfo>>,
    /// Last transcript (pasted or not): "paste last transcript" source when
    /// history is off/unavailable.
    last_text: Mutex<Option<String>>,
}

pub struct Controller {
    machine: Dictation,
    platform: Arc<dyn Platform>,
    core: Arc<dyn CoreApi>,
    prefs: Arc<RwLock<Prefs>>,
    tx: Sender<Input>,
    dictating: Arc<AtomicBool>,
    clock: Box<dyn Fn() -> u64 + Send>,
    shared: Arc<Shared>,
    history: Option<Arc<Mutex<History>>>,
    suggestions: Option<Arc<Mutex<SuggestionStore>>>,
    /// Bumped per dictation: a running edit watch stops when it changes.
    watch_gen: Arc<AtomicU64>,
    preview_shown: String,
}

impl Controller {
    pub fn new(
        platform: Arc<dyn Platform>,
        core: Arc<dyn CoreApi>,
        prefs: Arc<RwLock<Prefs>>,
        tx: Sender<Input>,
        clock: Box<dyn Fn() -> u64 + Send>,
    ) -> Self {
        let policy = prefs.read().unwrap().policy();
        Controller {
            machine: Dictation::new(policy),
            platform,
            core,
            prefs,
            tx,
            dictating: Arc::new(AtomicBool::new(false)),
            clock,
            shared: Arc::default(),
            history: None,
            suggestions: None,
            watch_gen: Arc::default(),
            preview_shown: String::new(),
        }
    }

    /// Collect learn-from-edits suggestions into `store` (needs
    /// `learn_from_edits` in prefs and a readable field).
    pub fn with_suggestions(mut self, store: Arc<Mutex<SuggestionStore>>) -> Self {
        self.suggestions = Some(store);
        self
    }

    /// Record transcripts to the local SQLite history (retention from prefs).
    pub fn with_history(mut self, h: Arc<Mutex<History>>) -> Self {
        if let Ok(g) = h.lock() {
            let _ = g.prune(self.prefs.read().unwrap().retention, history::now_ms());
        }
        self.history = Some(h);
        self
    }

    /// Shared with the keyboard hook (swallow Esc only while dictating).
    pub fn dictating_flag(&self) -> Arc<AtomicBool> {
        self.dictating.clone()
    }

    pub fn machine(&self) -> &Dictation {
        &self.machine
    }

    pub fn handle(&mut self, input: Input) {
        let now = (self.clock)();
        match &input {
            Input::Hotkey(HotkeyAction::PasteLast) => {
                self.paste_last();
                return;
            }
            Input::Hotkey(HotkeyAction::CopyLast) => {
                self.copy_last();
                return;
            }
            Input::PasteFinished { session, result } => self.record(*session, result),
            _ => {}
        }
        let tick = input == Input::Tick;
        let effects = self.machine.handle(input, now);
        self.dictating
            .store(self.machine.is_active(), Ordering::Relaxed);
        for e in effects {
            self.execute(e);
        }
        if tick {
            self.update_preview();
        }
        if !matches!(self.machine.phase(), Phase::Listening(_)) && !self.preview_shown.is_empty() {
            self.preview_shown.clear();
            self.platform.preview("");
        }
        self.drop_finished_sessions();
    }

    /// Sessions that are neither listening nor processing are done.
    fn drop_finished_sessions(&self) {
        let live = match self.machine.phase() {
            Phase::Listening(l) => Some(l.session),
            Phase::Processing { session, .. } => Some(*session),
            _ => None,
        };
        self.shared
            .sessions
            .lock()
            .unwrap()
            .retain(|id, _| Some(*id) == live);
    }

    fn update_preview(&mut self) {
        if !matches!(self.machine.phase(), Phase::Listening(_)) {
            return;
        }
        if !self.prefs.read().unwrap().live_preview {
            return;
        }
        let p = self.core.partial().unwrap_or_default();
        let p = p.trim();
        if p != self.preview_shown {
            self.preview_shown = p.to_string();
            self.platform.preview(p);
        }
    }

    /// Store the history row once the paste outcome is known, then watch
    /// the field for corrections.
    fn record(&self, session: SessionId, paste: &Result<(), String>) {
        let info = self.shared.sessions.lock().unwrap().remove(&session);
        let Some(info) = info else { return };
        let Some(r) = info.result else { return };
        *self.shared.last_text.lock().unwrap() = Some(r.final_text.clone());
        let mut row = None;
        if let Some(h) = &self.history {
            let ctx = info.context.clone().unwrap_or_default();
            let entry = history::Entry {
                id: 0,
                ts: history::now_ms(),
                app: ctx.app_id,
                url: ctx.url_host,
                raw: r.asr_text.clone(),
                formatted: (r.status == CommitStatus::Formatted).then(|| r.final_text.clone()),
                pasted: paste.is_ok().then(|| r.final_text.clone()),
                edited: None,
                duration_ms: (info.samples.load(Ordering::Relaxed) / 16) as i64,
                words: history::word_count(&r.final_text),
                lang: r.language.clone(),
            };
            let retention = self.prefs.read().unwrap().retention;
            match h.lock().unwrap().record(&entry, retention, entry.ts) {
                Ok(id) => row = id,
                Err(e) => eprintln!("[history] {e}"),
            }
            self.platform.history_changed();
        }
        if paste.is_ok() {
            self.watch_for_edits(info.context, &r, row);
        }
    }

    /// Learn from edits (03 §3): re-read the field for a while after the
    /// paste; corrections become suggestions, never dictionary entries.
    fn watch_for_edits(&self, ctx: Option<FieldContext>, r: &CommitResult, row: Option<i64>) {
        let prefs = self.prefs.read().unwrap().clone();
        // A sent message (press enter) clears the field: nothing to learn.
        if !prefs.learn_from_edits || !prefs.use_ax_context || r.press_enter {
            return;
        }
        // No snapshot = no anchors to find the dictated region by.
        let Some(ctx) = ctx else { return };
        let pasted = crate::paste::plain_text(&r.final_text);
        if pasted.trim().is_empty() {
            return;
        }
        let gen = self.watch_gen.load(Ordering::SeqCst);
        let watch_gen = self.watch_gen.clone();
        let platform = self.platform.clone();
        let core = self.core.clone();
        let history = self.history.clone();
        let suggestions = self.suggestions.clone();
        let live_prefs = self.prefs.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-learn".into())
            .spawn(move || {
                let Some(mut inner) = platform.field_reader() else {
                    return;
                };
                let mut reader = StopOnNewDictation {
                    inner: &mut *inner,
                    gen,
                    watch_gen: &watch_gen,
                };
                let t0 = std::time::Instant::now();
                let mut watch =
                    EditWatch::new(&pasted, Anchors::from_context(&ctx), 0, prefs.watch_opts());
                let outcome = learn::run_watch(
                    &mut reader,
                    &mut watch,
                    &|| t0.elapsed().as_millis() as u64,
                    &|ms| std::thread::sleep(Duration::from_millis(ms)),
                );
                drop(inner);
                let Some(edited) = outcome.edited else { return };
                if let (Some(h), Some(id)) = (&history, row) {
                    let _ = h.lock().unwrap().set_edited(id, &edited);
                    platform.history_changed();
                }
                let bounded = learn::bound_edit(&pasted, &edited);
                let cands = core.learn_edit(&pasted, &bounded);
                let Some(store) = suggestions else { return };
                let dict = live_prefs.read().unwrap().dictionary.clone();
                let mut s = store.lock().unwrap();
                if s.data.add(&cands, &ctx.app_id, history::now_ms(), &dict) > 0 {
                    if let Err(e) = s.save() {
                        eprintln!("[suggestions] {e}");
                    }
                    platform.suggestions_changed(s.data.pending.len());
                }
            });
        if let Err(e) = spawned {
            eprintln!("[learn] {e}");
        }
    }

    /// Last transcript: history first (survives restarts), else memory.
    fn last_text(&self) -> Option<String> {
        let retention = self.prefs.read().unwrap().retention;
        let mut text = self.shared.last_text.lock().unwrap().clone();
        if let Some(h) = &self.history {
            if let Ok(Some(e)) = h.lock().unwrap().latest(retention, history::now_ms()) {
                text = Some(e.best_text().to_string());
            }
        }
        text.filter(|t| !t.is_empty())
    }

    /// Re-insert the last transcript; ignored while a dictation is running.
    fn paste_last(&self) {
        if self.machine.is_active() {
            return;
        }
        let Some(text) = self.last_text() else { return };
        let prefs = self.prefs.read().unwrap().clone();
        let platform = self.platform.clone();
        std::thread::spawn(move || {
            if let Err(e) = platform.paste(&text, &prefs) {
                platform.overlay(&OverlayState::Error { message: e });
            }
        });
    }

    /// Put the last transcript on the clipboard (Wispr Shift+Alt+X).
    fn copy_last(&self) {
        let Some(text) = self.last_text() else { return };
        let text = crate::paste::plain_text(&text);
        let platform = self.platform.clone();
        std::thread::spawn(move || {
            if let Err(e) = platform.copy_text(&text) {
                eprintln!("[copy-last] {e}");
            }
        });
    }

    /// Event loop: ticks every 100 ms when idle of input.
    pub fn run(mut self, rx: Receiver<Input>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(i) => self.handle(i),
                Err(RecvTimeoutError::Timeout) => self.handle(Input::Tick),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn execute(&mut self, e: Effect) {
        let prefs = self.prefs.read().unwrap().clone();
        match e {
            Effect::Sound(s) => {
                if prefs.enable_sounds {
                    self.platform.play(s);
                }
            }
            Effect::Duck => {
                if prefs.mute_audio_while_dictating {
                    self.platform.duck();
                }
            }
            Effect::Unduck => {
                if prefs.mute_audio_while_dictating {
                    self.platform.unduck();
                }
            }
            Effect::BeginSession(session) => {
                self.watch_gen.fetch_add(1, Ordering::SeqCst);
                if let Err(message) = self.core.start_session(prefs.session_config()) {
                    let _ = self.tx.send(Input::StartFailed { session, message });
                    return;
                }
                let samples = Arc::new(AtomicU64::new(0));
                self.shared.sessions.lock().unwrap().insert(
                    session,
                    SessionInfo {
                        samples: samples.clone(),
                        ..SessionInfo::default()
                    },
                );
                if prefs.use_ax_context {
                    // set_context may legitimately arrive after audio started.
                    let platform = self.platform.clone();
                    let core = self.core.clone();
                    let shared = self.shared.clone();
                    std::thread::spawn(move || {
                        if let Some(raw) = platform.snapshot_context() {
                            let ctx = build_field_context(&raw);
                            if let Some(i) = shared.sessions.lock().unwrap().get_mut(&session) {
                                i.context = Some(ctx.clone());
                            }
                            core.set_context(ctx);
                        }
                    });
                }
                let core = self.core.clone();
                let platform = self.platform.clone();
                let mut meter = LevelMeter::default();
                let sink: AudioSink = Box::new(move |pcm: &[i16]| {
                    samples.fetch_add(pcm.len() as u64, Ordering::Relaxed);
                    core.push_audio(pcm);
                    for l in meter.push(pcm) {
                        platform.level(l);
                    }
                });
                if let Err(message) = self.platform.start_capture(prefs.microphone.clone(), sink) {
                    let _ = self.tx.send(Input::StartFailed { session, message });
                }
            }
            Effect::EndCapture => {
                self.platform.stop_capture();
                self.platform.level(0.0);
            }
            Effect::Commit(session) => {
                let core = self.core.clone();
                let tx = self.tx.clone();
                let shared = self.shared.clone();
                std::thread::spawn(move || {
                    let r = core.commit();
                    let outcome = to_outcome(&r);
                    if let Some(i) = shared.sessions.lock().unwrap().get_mut(&session) {
                        i.result = Some(r);
                    }
                    let _ = tx.send(Input::Committed { session, outcome });
                });
            }
            Effect::CancelSession(_) => self.core.cancel(),
            Effect::Paste { session, text } => {
                let press_enter = self
                    .shared
                    .sessions
                    .lock()
                    .unwrap()
                    .get(&session)
                    .and_then(|i| i.result.as_ref())
                    .is_some_and(|r| r.press_enter);
                let platform = self.platform.clone();
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let mut result = platform.paste(&text, &prefs);
                    if result.is_ok() && press_enter {
                        result = platform.send_enter();
                    }
                    let _ = tx.send(Input::PasteFinished { session, result });
                });
            }
            Effect::MaxDurationWarning { remaining_ms } => {
                self.platform.max_duration_warning(remaining_ms)
            }
            Effect::Overlay(st) => self.platform.overlay(&st),
        }
    }
}

/// Ends a watch as soon as the next dictation starts: its paste would
/// otherwise look like the user typing into the watched region.
struct StopOnNewDictation<'a> {
    inner: &'a mut dyn FieldReader,
    gen: u64,
    watch_gen: &'a AtomicU64,
}

impl FieldReader for StopOnNewDictation<'_> {
    fn read(&mut self) -> Read {
        if self.watch_gen.load(Ordering::SeqCst) != self.gen {
            return Read::FocusChanged;
        }
        self.inner.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_api::StubCore;
    use crate::dictation::Phase;
    use crate::hotkeys::HotkeyAction;
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        sink: Mutex<Option<AudioSink>>,
        levels: Mutex<Vec<f32>>,
        fail_capture: bool,
    }

    impl Fake {
        fn push(&self, s: impl Into<String>) {
            self.log.lock().unwrap().push(s.into());
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        fn feed_audio(&self, pcm: &[i16]) {
            if let Some(s) = self.sink.lock().unwrap().as_mut() {
                s(pcm);
            }
        }
    }

    impl Platform for Fake {
        fn play(&self, s: Sound) {
            self.push(format!("play {s:?}"));
        }
        fn duck(&self) {
            self.push("duck");
        }
        fn unduck(&self) {
            self.push("unduck");
        }
        fn snapshot_context(&self) -> Option<RawSnapshot> {
            Some(RawSnapshot {
                exe_path: r"C:\x\slack.exe".into(),
                ..Default::default()
            })
        }
        fn start_capture(&self, _d: Option<String>, sink: AudioSink) -> Result<(), String> {
            if self.fail_capture {
                return Err("no mic".into());
            }
            *self.sink.lock().unwrap() = Some(sink);
            self.push("capture start");
            Ok(())
        }
        fn stop_capture(&self) {
            *self.sink.lock().unwrap() = None;
            self.push("capture stop");
        }
        fn paste(&self, text: &str, _p: &Prefs) -> Result<(), String> {
            self.push(format!("paste {text}"));
            Ok(())
        }
        fn overlay(&self, st: &OverlayState) {
            self.push(format!("overlay {st:?}"));
        }
        fn level(&self, l: f32) {
            self.levels.lock().unwrap().push(l);
        }
        fn max_duration_warning(&self, _r: u64) {
            self.push("warn");
        }
    }

    struct Rig {
        c: Controller,
        rx: mpsc::Receiver<Input>,
        fake: Arc<Fake>,
        core: Arc<StubCore>,
        t: Arc<AtomicU64>,
    }

    fn rig(fake: Fake, prefs: Prefs) -> Rig {
        let fake = Arc::new(fake);
        let core = Arc::new(StubCore::new());
        let (tx, rx) = mpsc::channel();
        let t = Arc::new(AtomicU64::new(0));
        let tc = t.clone();
        let c = Controller::new(
            fake.clone(),
            core.clone(),
            Arc::new(RwLock::new(prefs)),
            tx,
            Box::new(move || tc.load(Ordering::SeqCst)),
        );
        Rig {
            c,
            rx,
            fake,
            core,
            t,
        }
    }

    impl Rig {
        fn at(&mut self, ms: u64, i: Input) {
            self.t.store(ms, Ordering::SeqCst);
            self.c.handle(i);
        }
        /// Feed back the next async completion.
        fn pump(&mut self) {
            let i = self
                .rx
                .recv_timeout(Duration::from_secs(2))
                .expect("completion");
            self.c.handle(i);
        }
    }

    #[test]
    fn full_ptt_dictation_pastes_core_text() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        assert!(r.c.dictating_flag().load(Ordering::SeqCst));
        r.fake.feed_audio(&[1000; 16_000]);
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        r.pump(); // Committed
        r.pump(); // PasteFinished
        assert!(matches!(r.c.machine().phase(), Phase::Pasted { .. }));
        assert!(!r.c.dictating_flag().load(Ordering::SeqCst));
        let log = r.fake.log();
        assert_eq!(
            log,
            vec![
                "play Start",
                "duck",
                "capture start",
                "overlay ActivePtt",
                "capture stop",
                "unduck",
                "play Stop",
                "overlay Processing",
                "paste [stub: 1.0 s of audio]",
                "play Paste",
                "overlay Resting",
            ]
        );
        assert_eq!(
            r.fake.levels.lock().unwrap().len(),
            6 + 1,
            "1 s = 6 x 160 ms + reset"
        );
    }

    #[test]
    fn context_snapshot_reaches_core() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        for _ in 0..100 {
            if r.core.last_context().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let ctx = r.core.last_context().expect("set_context called");
        assert_eq!(ctx.app_id, "slack.exe");
    }

    #[test]
    fn prefs_disable_sounds_and_ducking_and_context() {
        let prefs = Prefs {
            enable_sounds: false,
            mute_audio_while_dictating: false,
            use_ax_context: false,
            ..Prefs::default()
        };
        let mut r = rig(Fake::default(), prefs);
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        std::thread::sleep(Duration::from_millis(30));
        let log = r.fake.log();
        assert_eq!(log, vec!["capture start", "overlay ActivePtt"]);
        assert!(r.core.last_context().is_none());
    }

    #[test]
    fn capture_failure_surfaces_as_error() {
        let mut r = rig(
            Fake {
                fail_capture: true,
                ..Fake::default()
            },
            Prefs::default(),
        );
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.pump(); // StartFailed
        assert!(matches!(r.c.machine().phase(), Phase::Error { .. }));
        assert!(r
            .fake
            .log()
            .iter()
            .any(|l| l.contains("Error") && l.contains("no mic")));
    }

    #[test]
    fn silent_dictation_is_empty_and_pastes_nothing() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        r.pump();
        assert_eq!(r.c.machine().phase(), &Phase::Idle);
        assert!(!r.fake.log().iter().any(|l| l.starts_with("paste")));
    }
}
