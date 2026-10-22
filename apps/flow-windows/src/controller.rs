//! Orchestrator: runs the dictation machine and executes its effects
//! against a [`Platform`] (OS side) and a [`CoreApi`] (flow-core side).
//!
//! Platform-independent so the whole hotkey → capture → commit → paste
//! loop is integration-tested on Linux with fakes. Slow work (commit, paste,
//! UIA snapshot) runs on short-lived worker threads and reports back through
//! the same input channel, so the loop itself never blocks.

use crate::audio::LevelMeter;
use crate::context::{build_field_context, RawSnapshot};
use crate::core_api::{to_outcome, CommitResult, CommitStatus, CoreApi, FieldContext, PasteKey};
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
/// Reports a capture stream that failed mid-dictation.
pub type CaptureError = Box<dyn Fn(String) + Send + Sync>;

/// Everything OS-specific the controller needs.
pub trait Platform: Send + Sync + 'static {
    fn play(&self, s: Sound);
    fn duck(&self);
    fn unduck(&self);
    /// Blocking but bounded (UIA with timeout). Called on a worker thread.
    fn snapshot_context(&self) -> Option<RawSnapshot>;
    fn start_capture(&self, device: Option<String>, sink: AudioSink) -> Result<(), String>;
    /// `start_capture` that reports a stream failure mid-capture (mic
    /// unplugged) through `on_error`. Default: errors are not reported.
    fn start_capture_reporting(
        &self,
        device: Option<String>,
        sink: AudioSink,
        on_error: CaptureError,
    ) -> Result<(), String> {
        let _ = on_error;
        self.start_capture(device, sink)
    }
    fn stop_capture(&self);
    /// Blocking (clipboard restore delay). Called on a worker thread.
    fn paste(&self, text: &str, prefs: &Prefs) -> Result<(), String>;
    fn overlay(&self, st: &OverlayState);
    fn level(&self, level: f32);
    fn max_duration_warning(&self, remaining_ms: u64);
    /// Live partial transcript above the pill (`""` = hide).
    fn preview(&self, _text: &str) {}
    /// "Transcribing via …" while recording/processing (`None` = hide).
    fn live_route(&self, _label: Option<&crate::core_api::LiveRouteLabel>) {}
    /// Press Tab or Enter (paste plan keys; Enter = spoken "press
    /// enter"). Worker thread.
    fn send_key(&self, _key: PasteKey) -> Result<(), String> {
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
    /// The foreground window now (S13 paste target). Cheap; any thread.
    fn focus_target(&self) -> Option<crate::paste::FocusTarget> {
        None
    }
    /// Is the focused control a password field? (`None` = unknown.)
    /// Bounded (UIA with a timeout); worker thread.
    fn focused_is_password(&self) -> Option<bool> {
        None
    }
    /// Wait (bounded) until the shortcut's modifiers are physically up
    /// before a shortcut-triggered paste (paste-last, C13).
    fn wait_shortcut_released(&self) {}
    /// A history row was added or changed (the History tab refreshes).
    fn history_changed(&self) {}
    /// New dictionary suggestions are waiting (`pending` in total).
    fn suggestions_changed(&self, _pending: usize) {}
}

/// Longest wait for a session start (model load + server connect are
/// bounded well below this by their own timeouts).
const START_WAIT: Duration = Duration::from_secs(30);

/// Outcome of a session's background start, awaited by commit and
/// set_context.
#[derive(Default)]
struct StartGate {
    result: Mutex<Option<Result<(), String>>>,
    cv: std::sync::Condvar,
}

impl StartGate {
    fn open(&self, r: Result<(), String>) {
        *self.result.lock().unwrap() = Some(r);
        self.cv.notify_all();
    }

    fn wait(&self, timeout: Duration) -> Result<(), String> {
        let g = self.result.lock().unwrap();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |r| r.is_none())
            .unwrap();
        g.clone()
            .unwrap_or_else(|| Err("dictation start timed out".into()))
    }
}

/// Per-session facts collected for the history row.
#[derive(Default)]
struct SessionInfo {
    context: Option<FieldContext>,
    samples: Arc<AtomicU64>,
    result: Option<CommitResult>,
    gate: Arc<StartGate>,
    /// Esc / processing timeout: a paste not yet started is dropped.
    cancelled: Arc<AtomicBool>,
    /// A paste worker is running: keep the session until `PasteFinished`
    /// (so a paste that did happen still reaches history).
    pasting: bool,
    /// Foreground window at dictation start: the only paste target (S13).
    target: Option<crate::paste::FocusTarget>,
    /// The focused control was a password field at start (UIA snapshot).
    password_at_start: Option<bool>,
}

#[derive(Default)]
struct Shared {
    sessions: Mutex<HashMap<SessionId, SessionInfo>>,
    /// The session whose start may still complete (0 = cancelled).
    live: AtomicU64,
    /// Serialises background starts.
    start_lock: Mutex<()>,
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
    /// Hands-free session whose endpoint (D19) already sent `Finish`.
    endpoint_finished: Option<SessionId>,
    /// Route the pill names right now (relabelled on failover).
    route_shown: Option<crate::core_api::SessionRoute>,
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
            endpoint_finished: None,
            route_shown: None,
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
            Input::Started { session, route } => {
                if self.current_session() == Some(*session) {
                    self.show_route(route.clone());
                }
                return;
            }
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
            self.poll_session();
        }
        if !matches!(self.machine.phase(), Phase::Listening(_)) && !self.preview_shown.is_empty() {
            self.preview_shown.clear();
            self.platform.preview("");
        }
        if !self.machine.is_active() && self.route_shown.is_some() {
            self.route_shown = None;
            self.platform.live_route(None);
        }
        self.drop_finished_sessions();
    }

    fn current_session(&self) -> Option<SessionId> {
        match self.machine.phase() {
            Phase::Listening(l) => Some(l.session),
            Phase::Processing { session, .. } => Some(*session),
            _ => None,
        }
    }

    /// Sessions that are neither listening nor processing are done.
    fn drop_finished_sessions(&self) {
        let live = self.current_session();
        self.shared
            .sessions
            .lock()
            .unwrap()
            .retain(|id, i| Some(*id) == live || i.pasting);
    }

    /// The partial timer (every 100 ms tick while listening): one
    /// `poll()` gives the live preview and the hands-free endpoint (D19).
    fn poll_session(&mut self) {
        let Phase::Listening(l) = self.machine.phase() else {
            return;
        };
        let (session, mode) = (l.session, l.mode);
        let p = self.core.poll();
        // `None` mid-recording = the session died: keep the last label
        // (the commit reports the error).
        if let Some(r) = p.route.clone() {
            self.show_route(r);
        }
        if self.prefs.read().unwrap().live_preview {
            let text = p.partial.as_deref().unwrap_or("").trim();
            if text != self.preview_shown {
                self.preview_shown = text.to_string();
                self.platform.preview(text);
            }
        }
        // Silence after speech ends a hands-free dictation exactly like
        // pressing the hands-free key again: stop sound, commit, paste.
        if p.endpoint_reached
            && mode == crate::dictation::Mode::Locked
            && self.endpoint_finished != Some(session)
        {
            self.endpoint_finished = Some(session);
            let _ = self.tx.send(Input::Finish);
        }
    }

    fn show_route(&mut self, r: crate::core_api::SessionRoute) {
        if self.route_shown.as_ref() != Some(&r) {
            let label = self.core.describe_route(&r);
            self.route_shown = Some(r);
            self.platform.live_route(Some(&label));
        }
    }

    /// Store the history row once the paste outcome is known, then watch
    /// the field for corrections.
    fn record(&self, session: SessionId, paste: &Result<(), String>) {
        let info = self.shared.sessions.lock().unwrap().remove(&session);
        let Some(info) = info else { return };
        let Some(r) = info.result else { return };
        // Cancelled before anything was inserted: the user dropped it.
        if paste.as_ref().err().map(String::as_str) == Some(PASTE_CANCELLED) {
            return;
        }
        let plain = r.plain();
        *self.shared.last_text.lock().unwrap() = Some(plain.clone());
        let mut row = None;
        if let Some(h) = &self.history {
            let ctx = info.context.clone().unwrap_or_default();
            let entry = history::Entry {
                id: 0,
                ts: history::now_ms(),
                app: ctx.app_id,
                url: ctx.url_host,
                raw: r.asr_text.clone(),
                formatted: (r.status == CommitStatus::Formatted).then(|| plain.clone()),
                pasted: paste.is_ok().then(|| plain.clone()),
                edited: None,
                duration_ms: (info.samples.load(Ordering::Relaxed) / 16) as i64,
                words: history::word_count(&plain),
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
        let sent = r.paste_segments().iter().any(|s| {
            matches!(
                s,
                crate::core_api::PasteSegment::Key {
                    key: PasteKey::Enter
                }
            )
        });
        if !prefs.learn_from_edits || !prefs.use_ax_context || sent {
            return;
        }
        // No snapshot = no anchors to find the dictated region by.
        let Some(ctx) = ctx else { return };
        let pasted = r.plain();
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
            platform.wait_shortcut_released();
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
                let mode = match self.machine.phase() {
                    Phase::Listening(l) => l.mode,
                    _ => crate::dictation::Mode::Ptt,
                };
                // The start can block (model-load wait, server connect up to
                // its timeout), so it runs on its own thread while the mic
                // is already open; audio is buffered until it returns, so
                // no first word is lost (review P3, changelog C2).
                let samples = Arc::new(AtomicU64::new(0));
                let gate = Arc::new(StartGate::default());
                let pending: Arc<Mutex<Option<Vec<i16>>>> = Arc::new(Mutex::new(Some(Vec::new())));
                self.shared.sessions.lock().unwrap().insert(
                    session,
                    SessionInfo {
                        samples: samples.clone(),
                        gate: gate.clone(),
                        target: self.platform.focus_target(),
                        ..SessionInfo::default()
                    },
                );
                self.shared.live.store(session, Ordering::SeqCst);
                {
                    let core = self.core.clone();
                    let shared = self.shared.clone();
                    let tx = self.tx.clone();
                    let pending = pending.clone();
                    let start_gate = gate.clone();
                    let cfg = prefs.session_config_for(mode);
                    let spawned =
                        std::thread::Builder::new()
                            .name("flow-start".into())
                            .spawn(move || {
                                // One start at a time: a late start of a
                                // cancelled session never overlaps the next.
                                let _serial = shared.start_lock.lock().unwrap();
                                let res = core.start_session_info(cfg);
                                let live = shared.live.load(Ordering::SeqCst) == session;
                                match res {
                                    Ok(_) if !live => {
                                        core.cancel();
                                        start_gate.open(Err("cancelled".into()));
                                    }
                                    Ok(route) => {
                                        // Flush what the mic captured meanwhile,
                                        // in order, before live pushes resume.
                                        let mut g = pending.lock().unwrap();
                                        if let Some(buf) = g.take() {
                                            for chunk in buf.chunks(1600) {
                                                core.push_audio(chunk);
                                            }
                                        }
                                        drop(g);
                                        start_gate.open(Ok(()));
                                        if let Some(route) = route {
                                            let _ = tx.send(Input::Started { session, route });
                                        }
                                    }
                                    Err(message) => {
                                        pending.lock().unwrap().take();
                                        start_gate.open(Err(message.clone()));
                                        let _ = tx.send(Input::StartFailed { session, message });
                                    }
                                }
                            });
                    if let Err(e) = spawned {
                        gate.open(Err(e.to_string()));
                        let _ = self.tx.send(Input::StartFailed {
                            session,
                            message: e.to_string(),
                        });
                        return;
                    }
                }
                if prefs.use_ax_context {
                    // set_context may legitimately arrive after audio started.
                    let platform = self.platform.clone();
                    let core = self.core.clone();
                    let shared = self.shared.clone();
                    let gate = gate.clone();
                    std::thread::spawn(move || {
                        if let Some(raw) = platform.snapshot_context() {
                            let ctx = build_field_context(&raw);
                            if let Some(i) = shared.sessions.lock().unwrap().get_mut(&session) {
                                i.context = Some(ctx.clone());
                                i.password_at_start = Some(raw.is_password);
                            }
                            if gate.wait(START_WAIT).is_ok()
                                && shared.live.load(Ordering::SeqCst) == session
                            {
                                core.set_context(ctx);
                            }
                        }
                    });
                }
                let core = self.core.clone();
                let platform = self.platform.clone();
                let mut meter = LevelMeter::default();
                let cap = prefs.policy().max_samples() as usize;
                let sink: AudioSink = Box::new(move |pcm: &[i16]| {
                    samples.fetch_add(pcm.len() as u64, Ordering::Relaxed);
                    let buffered = {
                        let mut g = pending.lock().unwrap();
                        match g.as_mut() {
                            Some(buf) => {
                                let room = cap.saturating_sub(buf.len());
                                buf.extend_from_slice(&pcm[..pcm.len().min(room)]);
                                true
                            }
                            None => false,
                        }
                    };
                    if !buffered {
                        core.push_audio(pcm);
                    }
                    for l in meter.push(pcm) {
                        platform.level(l);
                    }
                });
                // A stream that dies mid-dictation ends it with an error
                // pill instead of listening to nothing.
                let tx = Mutex::new(self.tx.clone());
                let on_error = Box::new(move |message: String| {
                    if let Ok(tx) = tx.lock() {
                        let _ = tx.send(Input::StartFailed { session, message });
                    }
                });
                if let Err(message) =
                    self.platform
                        .start_capture_reporting(prefs.microphone.clone(), sink, on_error)
                {
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
                let gate = shared
                    .sessions
                    .lock()
                    .unwrap()
                    .get(&session)
                    .map(|i| i.gate.clone());
                std::thread::spawn(move || {
                    // A start still connecting is waited for (its buffered
                    // audio is flushed first); a failed start is the error.
                    let started = gate.map_or(Ok(()), |g| g.wait(START_WAIT));
                    let r = match started {
                        Ok(()) => core.commit(),
                        Err(m) => CommitResult::failed(m),
                    };
                    let outcome = to_outcome(&r);
                    if let Some(i) = shared.sessions.lock().unwrap().get_mut(&session) {
                        i.result = Some(r);
                    }
                    let _ = tx.send(Input::Committed { session, outcome });
                });
            }
            Effect::CancelSession(session) => {
                if let Some(i) = self.shared.sessions.lock().unwrap().get(&session) {
                    i.cancelled.store(true, Ordering::SeqCst);
                }
                // A start still in flight sees this and cancels itself.
                let _ = self.shared.live.compare_exchange(
                    session,
                    0,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
                self.core.cancel();
            }
            Effect::Paste { session, text } => {
                // The core's paste plan (core-p1): text runs, Tab/Break and
                // a trailing Enter. `text` is only the fallback for a result
                // without one.
                let segs = self
                    .shared
                    .sessions
                    .lock()
                    .unwrap()
                    .get(&session)
                    .and_then(|i| i.result.as_ref())
                    .map(|r| r.paste_segments())
                    .unwrap_or_else(|| flow_core::paste::segments(&text, false));
                let steps = crate::paste::plan(&segs);
                let platform = self.platform.clone();
                let tx = self.tx.clone();
                let (cancelled, target, password_at_start) = {
                    let mut g = self.shared.sessions.lock().unwrap();
                    match g.get_mut(&session) {
                        Some(i) => {
                            i.pasting = true;
                            (i.cancelled.clone(), i.target.clone(), i.password_at_start)
                        }
                        None => (Arc::default(), None, None),
                    }
                };
                let plain = crate::paste::plain_text(&text);
                std::thread::spawn(move || {
                    // S13: only into the window the dictation started in,
                    // and never into a password field that appeared since.
                    let guard = |password_now: Option<bool>| {
                        crate::paste::paste_guard(
                            target.as_ref(),
                            platform.focus_target().as_ref(),
                            password_at_start,
                            password_now,
                        )
                    };
                    let first = if cancelled.load(Ordering::SeqCst) {
                        Err(PASTE_CANCELLED.to_string())
                    } else {
                        let pw = if target.is_some() {
                            platform.focused_is_password()
                        } else {
                            None
                        };
                        guard(pw).map_err(|g| {
                            if g == crate::paste::GuardFailure::Moved {
                                let _ = platform.copy_text(&plain);
                            }
                            g.message().to_string()
                        })
                    };
                    let result = first.and_then(|()| {
                        let stop = || {
                            if cancelled.load(Ordering::SeqCst) {
                                return Some(PASTE_CANCELLED.to_string());
                            }
                            guard(None).err().map(|g| g.message().to_string())
                        };
                        run_paste_plan_until(&*platform, &steps, &prefs, &stop)
                    });
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

/// `PasteFinished` error of a paste dropped by Esc / the timeout before
/// anything was inserted.
pub const PASTE_CANCELLED: &str = "cancelled";

/// Execute paste steps in order; the first failure stops the plan.
pub fn run_paste_plan(
    platform: &dyn Platform,
    steps: &[crate::paste::Step],
    prefs: &Prefs,
) -> Result<(), String> {
    run_paste_plan_until(platform, steps, prefs, &|| None)
}

/// Same, checking `stop` before every step (cancel flag, focus guard);
/// `Some(reason)` ends the plan with that error.
pub fn run_paste_plan_until(
    platform: &dyn Platform,
    steps: &[crate::paste::Step],
    prefs: &Prefs,
    stop: &dyn Fn() -> Option<String>,
) -> Result<(), String> {
    use crate::paste::Step;
    let typed_prefs = Prefs {
        paste_method: crate::prefs::PasteMethod::Type,
        ..prefs.clone()
    };
    for step in steps {
        if let Some(why) = stop() {
            return Err(why);
        }
        match step {
            Step::Insert { text, typed: false } => platform.paste(text, prefs)?,
            Step::Insert { text, typed: true } => platform.paste(text, &typed_prefs)?,
            Step::Key(k) => platform.send_key(*k)?,
            Step::Pause(ms) => std::thread::sleep(Duration::from_millis(*ms)),
        }
    }
    Ok(())
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
        /// `paste` blocks until this receives (tests of late cancels).
        paste_gate: Mutex<Option<mpsc::Receiver<()>>>,
        /// Foreground window (`focus_target`) and password-field answer.
        focus: Mutex<Option<crate::paste::FocusTarget>>,
        password: Mutex<Option<bool>>,
        on_error: Mutex<Option<CaptureError>>,
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
            if let Some(rx) = self.paste_gate.lock().unwrap().as_ref() {
                let _ = rx.recv_timeout(Duration::from_secs(2));
            }
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
        fn focus_target(&self) -> Option<crate::paste::FocusTarget> {
            self.focus.lock().unwrap().clone()
        }
        fn start_capture_reporting(
            &self,
            d: Option<String>,
            sink: AudioSink,
            on_error: CaptureError,
        ) -> Result<(), String> {
            *self.on_error.lock().unwrap() = Some(on_error);
            self.start_capture(d, sink)
        }
        fn focused_is_password(&self) -> Option<bool> {
            *self.password.lock().unwrap()
        }
        fn copy_text(&self, text: &str) -> Result<(), String> {
            self.push(format!("copy {text}"));
            Ok(())
        }
        fn live_route(&self, l: Option<&crate::core_api::LiveRouteLabel>) {
            self.push(format!(
                "route {}",
                l.map(|l| l.text.as_str()).unwrap_or("-")
            ));
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
        rig_slow(fake, prefs, Duration::ZERO)
    }

    /// A core whose `start_session*` blocks (model load / server connect).
    struct SlowStart {
        inner: Arc<StubCore>,
        delay: Duration,
    }

    impl CoreApi for SlowStart {
        fn start_session(&self, cfg: crate::core_api::SessionConfig) -> Result<(), String> {
            std::thread::sleep(self.delay);
            self.inner.start_session(cfg)
        }
        fn start_session_info(
            &self,
            cfg: crate::core_api::SessionConfig,
        ) -> Result<Option<crate::core_api::SessionRoute>, String> {
            std::thread::sleep(self.delay);
            self.inner.start_session_info(cfg)
        }
        fn push_audio(&self, pcm: &[i16]) {
            self.inner.push_audio(pcm)
        }
        fn set_context(&self, ctx: FieldContext) {
            self.inner.set_context(ctx)
        }
        fn partial(&self) -> Option<String> {
            None
        }
        fn poll(&self) -> crate::core_api::Poll {
            self.inner.poll()
        }
        fn commit(&self) -> CommitResult {
            self.inner.commit()
        }
        fn cancel(&self) {
            self.inner.cancel()
        }
        fn learn_edit(&self, p: &str, e: &str) -> Vec<crate::core_api::LearnCandidate> {
            self.inner.learn_edit(p, e)
        }
    }

    fn rig_slow(fake: Fake, prefs: Prefs, delay: Duration) -> Rig {
        let fake = Arc::new(fake);
        let core = Arc::new(StubCore::new());
        let api: Arc<dyn CoreApi> = if delay.is_zero() {
            core.clone()
        } else {
            Arc::new(SlowStart {
                inner: core.clone(),
                delay,
            })
        };
        let (tx, rx) = mpsc::channel();
        let t = Arc::new(AtomicU64::new(0));
        let tc = t.clone();
        let c = Controller::new(
            fake.clone(),
            api,
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
        /// The config of the `n`-th background start.
        fn wait_config(&self, n: usize) -> crate::core_api::SessionConfig {
            for _ in 0..400 {
                if self.core.starts() >= n {
                    return self.core.last_config().unwrap();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("start {n} never happened");
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

    #[test]
    fn hands_free_sessions_get_the_endpoint_ptt_does_not() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        assert_eq!(r.wait_config(1).endpoint_silence_ms, None);
        r.at(10, Input::Hotkey(HotkeyAction::Cancel));
        r.at(2000, Input::Hotkey(HotkeyAction::HandsFreeToggle));
        assert_eq!(r.wait_config(2).endpoint_silence_ms, Some(2000));
    }

    #[test]
    fn endpoint_reached_finishes_a_hands_free_dictation() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::HandsFreeToggle));
        r.fake.feed_audio(&[1000; 16_000]);
        r.at(1000, Input::Tick);
        assert!(r.rx.try_recv().is_err(), "no endpoint yet");
        r.core.set_poll(crate::core_api::Poll {
            endpoint_reached: true,
            ..Default::default()
        });
        r.at(3000, Input::Tick);
        r.at(3100, Input::Tick); // latched: only one Finish
        r.pump(); // Finish -> stop + commit
        assert!(matches!(r.c.machine().phase(), Phase::Processing { .. }));
        r.pump(); // Committed
        r.pump(); // PasteFinished
        assert!(matches!(r.c.machine().phase(), Phase::Pasted { .. }));
        let log = r.fake.log();
        assert!(log.contains(&"play Stop".to_string()), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("paste [stub: 1.0 s")));
        assert!(r.rx.try_recv().is_err(), "a single Finish");
    }

    #[test]
    fn endpoint_never_ends_push_to_talk() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.core.set_poll(crate::core_api::Poll {
            endpoint_reached: true,
            ..Default::default()
        });
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.at(3000, Input::Tick);
        assert!(r.rx.try_recv().is_err());
        assert!(matches!(r.c.machine().phase(), Phase::Listening(_)));
    }

    #[test]
    fn pill_names_the_live_route_and_follows_a_failover() {
        use crate::core_api::{Poll, SessionRoute};
        use flow_router::EngineKind;
        let mut r = rig(Fake::default(), Prefs::default());
        let server = SessionRoute {
            engine: EngineKind::Remote,
            server: Some("box:7313".into()),
            ..Default::default()
        };
        r.core.set_start_route(Some(server.clone()));
        r.core.set_poll(Poll {
            route: Some(server),
            ..Default::default()
        });
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.pump(); // Started (route of the background start)
        r.at(100, Input::Tick); // same route: no relabel
        r.core.set_poll(Poll {
            route: Some(SessionRoute {
                fallback: Some("disconnected".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        r.at(200, Input::Tick);
        r.core.set_poll(Poll::default()); // dead session: keep the label
        r.at(300, Input::Tick);
        r.fake.feed_audio(&[1000; 16_000]);
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        r.pump();
        r.pump();
        let routes: Vec<String> = r
            .fake
            .log()
            .into_iter()
            .filter(|l| l.starts_with("route"))
            .collect();
        assert_eq!(
            routes,
            vec![
                "route Transcribing via server…",
                "route Transcribing via this PC… (server unreachable)",
                "route -",
            ]
        );
    }

    #[test]
    fn slow_start_opens_the_mic_at_once_and_loses_no_audio() {
        let mut r = rig_slow(
            Fake::default(),
            Prefs::default(),
            Duration::from_millis(300),
        );
        let t0 = std::time::Instant::now();
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        assert!(
            t0.elapsed() < Duration::from_millis(200),
            "the controller thread must not wait for the start"
        );
        assert!(r.fake.log().contains(&"capture start".to_string()));
        r.fake.feed_audio(&[1000; 8_000]); // spoken while still connecting
        std::thread::sleep(Duration::from_millis(400));
        r.fake.feed_audio(&[1000; 8_000]);
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        r.pump(); // Committed
        r.pump(); // PasteFinished
        assert!(
            r.fake
                .log()
                .contains(&"paste [stub: 1.0 s of audio]".to_string()),
            "{:?}",
            r.fake.log()
        );
    }

    #[test]
    fn release_before_a_slow_start_finishes_still_commits_everything() {
        let mut r = rig_slow(
            Fake::default(),
            Prefs::default(),
            Duration::from_millis(300),
        );
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.fake.feed_audio(&[1000; 16_000]);
        r.at(600, Input::Hotkey(HotkeyAction::PttUp)); // start still pending
        r.pump(); // Committed (after the start)
        r.pump(); // PasteFinished
        assert!(r
            .fake
            .log()
            .contains(&"paste [stub: 1.0 s of audio]".to_string()));
    }

    #[test]
    fn cancel_during_a_slow_start_cancels_the_late_session() {
        let mut r = rig_slow(
            Fake::default(),
            Prefs::default(),
            Duration::from_millis(200),
        );
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.at(600, Input::Hotkey(HotkeyAction::Cancel));
        let cancels_before = r.core.cancels();
        for _ in 0..100 {
            if r.core.starts() == 1 && r.core.cancels() > cancels_before {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(r.core.starts(), 1);
        assert!(
            r.core.cancels() > cancels_before,
            "the session that started after Esc is cancelled"
        );
        assert_eq!(r.c.machine().phase(), &Phase::Idle);
    }

    #[test]
    fn a_cancelled_paste_plan_inserts_nothing() {
        let fake = Fake::default();
        let steps = crate::paste::plan(&flow_core::paste::segments("hi", true));
        let r = run_paste_plan_until(&fake, &steps, &Prefs::default(), &|| {
            Some(PASTE_CANCELLED.into())
        });
        assert_eq!(r, Err(PASTE_CANCELLED.to_string()));
        assert!(fake.log().is_empty());
    }

    #[test]
    fn esc_after_the_paste_started_keeps_it_in_history() {
        let (gate_tx, gate_rx) = mpsc::channel();
        let fake = Fake {
            paste_gate: Mutex::new(Some(gate_rx)),
            ..Fake::default()
        };
        let mut r = rig(fake, Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.fake.feed_audio(&[1000; 16_000]);
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        r.pump(); // Committed -> Paste dispatched, worker blocked in paste()
        std::thread::sleep(Duration::from_millis(50));
        r.at(1100, Input::Hotkey(HotkeyAction::Cancel));
        assert_eq!(r.c.machine().phase(), &Phase::Idle);
        gate_tx.send(()).unwrap();
        r.pump(); // PasteFinished (the paste did happen)
        assert_eq!(
            r.c.shared.last_text.lock().unwrap().as_deref(),
            Some("[stub: 1.0 s of audio]"),
            "a paste that happened is still the last transcript"
        );
    }

    #[test]
    fn processing_timeout_cancels_the_commit() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.wait_config(1);
        let before = r.core.cancels();
        // Hold the machine in processing without a Committed reply.
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        let _ = r.rx.recv_timeout(Duration::from_secs(2)); // drop Committed
        r.at(40_000, Input::Tick);
        assert!(r.core.cancels() > before, "timeout calls cancel()");
        assert!(matches!(r.c.machine().phase(), Phase::Error { .. }));
    }

    fn window(hwnd: usize) -> Option<crate::paste::FocusTarget> {
        Some(crate::paste::FocusTarget {
            hwnd,
            pid: hwnd as u32,
            exe: "chat.exe".into(),
        })
    }

    /// Dictate 1 s, switching focus/password state before the paste.
    fn dictate_then(r: &mut Rig, before_paste: impl FnOnce(&Fake)) {
        r.at(0, Input::Hotkey(HotkeyAction::PttDown));
        r.fake.feed_audio(&[1000; 16_000]);
        r.at(1000, Input::Hotkey(HotkeyAction::PttUp));
        before_paste(&r.fake);
        r.pump(); // Committed
        r.pump(); // PasteFinished
    }

    #[test]
    fn paste_lands_in_the_window_the_dictation_started_in() {
        let fake = Fake {
            focus: Mutex::new(window(1)),
            ..Fake::default()
        };
        let mut r = rig(fake, Prefs::default());
        dictate_then(&mut r, |_| {});
        assert!(r
            .fake
            .log()
            .contains(&"paste [stub: 1.0 s of audio]".to_string()));
    }

    #[test]
    fn alt_tab_while_processing_copies_instead_of_pasting() {
        let fake = Fake {
            focus: Mutex::new(window(1)),
            ..Fake::default()
        };
        let mut r = rig(fake, Prefs::default());
        dictate_then(&mut r, |f| *f.focus.lock().unwrap() = window(2));
        let log = r.fake.log();
        assert!(!log.iter().any(|l| l.starts_with("paste")), "{log:?}");
        assert!(log.contains(&"copy [stub: 1.0 s of audio]".to_string()));
        assert!(
            log.iter()
                .any(|l| l.contains("Error") && l.contains("focus moved")),
            "{log:?}"
        );
        assert_eq!(
            r.c.shared.last_text.lock().unwrap().as_deref(),
            Some("[stub: 1.0 s of audio]"),
            "still the last transcript (paste-last / history)"
        );
    }

    #[test]
    fn a_password_field_focused_since_the_start_gets_nothing() {
        let fake = Fake {
            focus: Mutex::new(window(1)),
            ..Fake::default()
        };
        let mut r = rig(fake, Prefs::default());
        dictate_then(&mut r, |f| *f.password.lock().unwrap() = Some(true));
        let log = r.fake.log();
        assert!(!log
            .iter()
            .any(|l| l.starts_with("paste") || l.starts_with("copy")));
        assert!(log.iter().any(|l| l.contains("password field")), "{log:?}");
    }

    #[test]
    fn a_mic_that_dies_mid_dictation_ends_it_with_an_error() {
        let mut r = rig(Fake::default(), Prefs::default());
        r.at(0, Input::Hotkey(HotkeyAction::HandsFreeToggle));
        (r.fake.on_error.lock().unwrap().as_ref().unwrap())("Microphone stopped working".into());
        r.pump();
        assert!(matches!(r.c.machine().phase(), Phase::Error { .. }));
        assert!(r
            .fake
            .log()
            .iter()
            .any(|l| l.contains("Microphone stopped working")));
    }
}
