//! Orchestrator: runs the dictation machine and executes its effects
//! against a [`Platform`] (OS side) and a [`CoreApi`] (flow-core side).
//!
//! Platform-independent so the whole hotkey → capture → commit → paste
//! loop is integration-tested on Linux with fakes. Slow work (commit, paste,
//! UIA snapshot) runs on short-lived worker threads and reports back through
//! the same input channel, so the loop itself never blocks.

use crate::audio::LevelMeter;
use crate::context::{build_field_context, RawSnapshot};
use crate::core_api::{to_outcome, CoreApi};
use crate::dictation::{Dictation, Effect, Input, OverlayState, Sound};
use crate::prefs::Prefs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
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
}

pub struct Controller {
    machine: Dictation,
    platform: Arc<dyn Platform>,
    core: Arc<dyn CoreApi>,
    prefs: Arc<RwLock<Prefs>>,
    tx: Sender<Input>,
    dictating: Arc<AtomicBool>,
    clock: Box<dyn Fn() -> u64 + Send>,
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
        }
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
        let effects = self.machine.handle(input, now);
        self.dictating
            .store(self.machine.is_active(), Ordering::Relaxed);
        for e in effects {
            self.execute(e);
        }
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
                if let Err(message) = self.core.start_session(prefs.session_config()) {
                    let _ = self.tx.send(Input::StartFailed { session, message });
                    return;
                }
                if prefs.use_ax_context {
                    // set_context may legitimately arrive after audio started.
                    let platform = self.platform.clone();
                    let core = self.core.clone();
                    std::thread::spawn(move || {
                        if let Some(raw) = platform.snapshot_context() {
                            core.set_context(build_field_context(&raw));
                        }
                    });
                }
                let core = self.core.clone();
                let platform = self.platform.clone();
                let mut meter = LevelMeter::default();
                let sink: AudioSink = Box::new(move |pcm: &[i16]| {
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
                std::thread::spawn(move || {
                    let r = core.commit();
                    let _ = tx.send(Input::Committed {
                        session,
                        outcome: to_outcome(&r),
                    });
                });
            }
            Effect::CancelSession(_) => self.core.cancel(),
            Effect::Paste { session, text } => {
                let platform = self.platform.clone();
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let result = platform.paste(&text, &prefs);
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
        assert_eq!(r.fake.levels.lock().unwrap().len(), 30 + 1, "30 Hz + reset");
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
