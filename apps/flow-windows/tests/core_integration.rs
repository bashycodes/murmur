//! Shell controller driven against the REAL flow-core (through
//! `core_adapter`), with only the OS side faked: proves
//! hotkey → capture → push_audio → commit → paste plan → history.
//!
//! * MockAsr backend (always runs): deterministic transcript, exercises the
//!   whole flow-core session engine + post-processing.
//! * whisper.cpp with the tiny model on jfk.wav (runs when the model is on
//!   disk; set FLOW_MODEL / FLOW_WAV or fetch with `tools/fetch-models.sh`).

use flow_windows::context::RawSnapshot;
use flow_windows::controller::{AudioSink, Controller, Platform};
use flow_windows::core_adapter::{FlowCoreAdapter, LlmSpec, LlmStatus, ModelSpec, ModelStatus};
use flow_windows::dictation::{Input, OverlayState, Phase, Sound};
use flow_windows::history::History;
use flow_windows::hotkeys::HotkeyAction;
use flow_windows::learn::{FieldReader, Read};
use flow_windows::prefs::{Prefs, Retention};
use flow_windows::suggestions::SuggestionStore;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

#[derive(Default)]
struct FakeOs {
    log: Mutex<Vec<String>>,
    sink: Mutex<Option<AudioSink>>,
    pastes: Mutex<Vec<String>>,
    previews: Mutex<Vec<String>>,
    exe: String,
    copies: Mutex<Vec<String>>,
    /// Learn-from-edits: how the user rewrites the last paste (from, to).
    edit: Option<(String, String)>,
}

/// Field as UIA would see it: the paste, then the user's correction, then
/// focus moves away.
struct ScriptedField {
    reads: std::collections::VecDeque<Read>,
}

impl FieldReader for ScriptedField {
    fn read(&mut self) -> Read {
        self.reads.pop_front().unwrap_or(Read::FocusChanged)
    }
}

impl FakeOs {
    fn new(exe: &str) -> Self {
        FakeOs {
            exe: exe.into(),
            ..FakeOs::default()
        }
    }
    fn push(&self, s: impl Into<String>) {
        self.log.lock().unwrap().push(s.into());
    }
    fn feed(&self, pcm: &[i16]) {
        // 40 ms chunks, like the capture thread.
        for c in pcm.chunks(640) {
            if let Some(s) = self.sink.lock().unwrap().as_mut() {
                s(c);
            }
        }
    }
    fn pastes(&self) -> Vec<String> {
        self.pastes.lock().unwrap().clone()
    }
}

impl Platform for FakeOs {
    fn play(&self, s: Sound) {
        self.push(format!("play {s:?}"));
    }
    fn duck(&self) {}
    fn unduck(&self) {}
    fn snapshot_context(&self) -> Option<RawSnapshot> {
        Some(RawSnapshot {
            exe_path: format!(r"C:\Apps\{}", self.exe),
            window_title: "Untitled - Notepad".into(),
            ..RawSnapshot::default()
        })
    }
    fn start_capture(&self, _d: Option<String>, sink: AudioSink) -> Result<(), String> {
        *self.sink.lock().unwrap() = Some(sink);
        Ok(())
    }
    fn stop_capture(&self) {
        *self.sink.lock().unwrap() = None;
    }
    fn paste(&self, text: &str, _p: &Prefs) -> Result<(), String> {
        self.pastes.lock().unwrap().push(text.to_string());
        Ok(())
    }
    fn overlay(&self, st: &OverlayState) {
        self.push(format!("overlay {st:?}"));
    }
    fn level(&self, _l: f32) {}
    fn max_duration_warning(&self, _r: u64) {}
    fn preview(&self, text: &str) {
        self.previews.lock().unwrap().push(text.to_string());
    }
    fn send_enter(&self) -> Result<(), String> {
        self.push("enter");
        Ok(())
    }
    fn copy_text(&self, text: &str) -> Result<(), String> {
        self.copies.lock().unwrap().push(text.to_string());
        Ok(())
    }
    fn field_reader(&self) -> Option<Box<dyn FieldReader>> {
        let (from, to) = self.edit.clone()?;
        let pasted = self.pastes().last()?.clone();
        let edited = pasted.replace(&from, &to);
        Some(Box::new(ScriptedField {
            reads: [
                Read::Text(pasted),
                Read::Unavailable,
                Read::Text(edited.clone()),
                Read::Text(edited),
            ]
            .into(),
        }))
    }
}

struct Rig {
    c: Controller,
    rx: mpsc::Receiver<Input>,
    os: Arc<FakeOs>,
    t: Arc<AtomicU64>,
    history: Arc<Mutex<History>>,
}

fn rig(core: Arc<FlowCoreAdapter>, os: FakeOs, prefs: Prefs) -> Rig {
    rig_with(core, os, prefs, None)
}

fn rig_with(
    core: Arc<FlowCoreAdapter>,
    os: FakeOs,
    prefs: Prefs,
    suggestions: Option<Arc<Mutex<SuggestionStore>>>,
) -> Rig {
    let os = Arc::new(os);
    let (tx, rx) = mpsc::channel();
    let t = Arc::new(AtomicU64::new(0));
    let tc = t.clone();
    let history = Arc::new(Mutex::new(History::open_in_memory().unwrap()));
    let mut c = Controller::new(
        os.clone(),
        core,
        Arc::new(RwLock::new(prefs)),
        tx,
        Box::new(move || tc.load(Ordering::SeqCst)),
    )
    .with_history(history.clone());
    if let Some(s) = suggestions {
        c = c.with_suggestions(s);
    }
    Rig {
        c,
        rx,
        os,
        t,
        history,
    }
}

impl Rig {
    fn at(&mut self, ms: u64, i: Input) {
        self.t.store(ms, Ordering::SeqCst);
        self.c.handle(i);
    }
    /// Feed async completions back until the machine rests.
    fn settle(&mut self, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while self.c.machine().is_active() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let i = self
                .rx
                .recv_timeout(left)
                .expect("completion before timeout");
            self.c.handle(i);
        }
    }
    fn dictate(&mut self, pcm: &[i16], timeout: Duration) {
        self.at(0, Input::Hotkey(HotkeyAction::PttDown));
        self.os.feed(pcm);
        let ms = 1000 + pcm.len() as u64 / 16;
        self.at(ms, Input::Hotkey(HotkeyAction::PttUp));
        self.settle(timeout);
    }
}

fn mock_core(text: &str) -> Arc<FlowCoreAdapter> {
    let a = FlowCoreAdapter::new();
    let st = a.load_now(&ModelSpec::Mock { text: text.into() });
    assert!(st.is_ready(), "{st:?}");
    a
}

fn tone(secs: f32) -> Vec<i16> {
    (0..(16_000.0 * secs) as usize)
        .map(|i| ((i as f32 * 0.07).sin() * 6000.0) as i16)
        .collect()
}

#[test]
fn mock_asr_audio_to_commit_to_paste_and_history() {
    let prefs = Prefs {
        dictionary: serde_json::from_str(
            r#"[{"word":"gh","replacement":"GitHub","isSnippet":false}]"#,
        )
        .unwrap(),
        ..Prefs::default()
    };
    let mut r = rig(
        mock_core("push it to gh today"),
        FakeOs::new("notepad.exe"),
        prefs,
    );
    r.dictate(&tone(1.5), Duration::from_secs(10));
    let pastes = r.os.pastes();
    assert_eq!(pastes.len(), 1, "{:?}", r.os.log.lock().unwrap());
    assert!(
        pastes[0].contains("GitHub"),
        "dictionary from prefs applied by the core: {pastes:?}"
    );
    assert!(matches!(r.c.machine().phase(), Phase::Pasted { .. }));

    let h = r.history.lock().unwrap();
    let e = h.latest(Retention::Keep, i64::MAX / 2).unwrap().unwrap();
    assert_eq!(e.app, "notepad.exe");
    assert_eq!(e.raw, "push it to gh today");
    assert_eq!(e.pasted.as_deref(), Some(pastes[0].as_str()));
    assert!(e.formatted.is_none(), "no LLM: RAW_TRANSCRIPT");
    assert!((1400..=1600).contains(&e.duration_ms), "{}", e.duration_ms);
    drop(h);

    // Paste last transcript re-inserts the same text.
    r.at(5000, Input::Hotkey(HotkeyAction::PasteLast));
    for _ in 0..200 {
        if r.os.pastes().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(r.os.pastes(), vec![pastes[0].clone(), pastes[0].clone()]);
}

#[test]
fn spoken_press_enter_sends_enter_after_paste() {
    let mut r = rig(
        mock_core("ship it press enter"),
        FakeOs::new("slack.exe"),
        Prefs::default(),
    );
    r.dictate(&tone(1.2), Duration::from_secs(10));
    let pastes = r.os.pastes();
    assert_eq!(pastes.len(), 1);
    assert!(
        !pastes[0].to_lowercase().contains("press enter"),
        "{pastes:?}"
    );
    assert!(r.os.log.lock().unwrap().iter().any(|l| l == "enter"));
}

#[test]
fn silence_is_empty_and_pastes_nothing() {
    let mut r = rig(
        mock_core("ghost"),
        FakeOs::new("notepad.exe"),
        Prefs::default(),
    );
    r.dictate(&vec![0i16; 16_000], Duration::from_secs(10));
    assert!(r.os.pastes().is_empty());
    assert_eq!(r.c.machine().phase(), &Phase::Idle);
    assert_eq!(r.history.lock().unwrap().count().unwrap(), 0);
}

#[test]
fn retention_never_keeps_no_history() {
    let prefs = Prefs {
        retention: Retention::Never,
        ..Prefs::default()
    };
    let mut r = rig(mock_core("secret words"), FakeOs::new("notepad.exe"), prefs);
    r.dictate(&tone(1.2), Duration::from_secs(10));
    assert_eq!(r.os.pastes().len(), 1);
    assert_eq!(r.history.lock().unwrap().count().unwrap(), 0);
}

#[test]
fn missing_model_gives_error_pill_not_a_hang() {
    let a = FlowCoreAdapter::new();
    let st = a.load_now(&ModelSpec::Whisper {
        path: PathBuf::from("/definitely/not/here/ggml-base.bin"),
        threads: None,
    });
    assert!(matches!(st, ModelStatus::Missing { .. }));
    let mut r = rig(a, FakeOs::new("notepad.exe"), Prefs::default());
    r.at(0, Input::Hotkey(HotkeyAction::PttDown));
    let i = r.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    r.c.handle(i);
    assert!(matches!(r.c.machine().phase(), Phase::Error { .. }));
    let log = r.os.log.lock().unwrap();
    assert!(
        log.iter()
            .any(|l| l.contains("Error") && l.contains("not found")),
        "{log:?}"
    );
}

#[test]
fn live_partials_reach_the_overlay_preview() {
    let prefs = Prefs {
        live_preview: true,
        live_preview_interval_ms: 250,
        ..Prefs::default()
    };
    let mut r = rig(mock_core("live words"), FakeOs::new("notepad.exe"), prefs);
    r.at(0, Input::Hotkey(HotkeyAction::PttDown));
    r.os.feed(&tone(1.5));
    let mut seen = false;
    for k in 0..200 {
        r.at(600 + k, Input::Tick);
        if r.os
            .previews
            .lock()
            .unwrap()
            .iter()
            .any(|p| p == "live words")
        {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen, "partial shown while listening");
    r.at(3000, Input::Hotkey(HotkeyAction::PttUp));
    assert_eq!(
        r.os.previews.lock().unwrap().last().map(String::as_str),
        Some(""),
        "preview cleared when listening ends"
    );
    r.settle(Duration::from_secs(10));
}

fn find(env: &str, rel: &[&str]) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(env) {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    rel.iter().map(|r| root.join(r)).find(|p| p.is_file())
}

#[cfg(feature = "whisper")]
#[test]
fn real_tiny_model_on_jfk_pastes_the_speech() {
    // Worktree-local models/ first, then the main checkout's.
    let model = find(
        "FLOW_MODEL",
        &[
            "../../models/ggml-tiny.en.bin",
            "../../../../../models/ggml-tiny.en.bin",
        ],
    );
    let wav = find(
        "FLOW_WAV",
        &["../../models/jfk.wav", "../../../../../models/jfk.wav"],
    );
    let (Some(model), Some(wav)) = (model, wav) else {
        eprintln!("SKIP: tiny model / jfk.wav not found (tools/fetch-models.sh)");
        return;
    };
    let mut reader = hound::WavReader::open(&wav).unwrap();
    let spec = reader.spec();
    assert_eq!((spec.sample_rate, spec.channels), (16_000, 1));
    let pcm: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();

    let a = FlowCoreAdapter::new();
    let st = a.load_now(&ModelSpec::Whisper {
        path: model,
        threads: None,
    });
    assert!(st.is_ready(), "{st:?}");
    let prefs = Prefs {
        live_preview: false,
        ..Prefs::default()
    };
    let mut r = rig(a, FakeOs::new("notepad.exe"), prefs);
    r.dictate(&pcm, Duration::from_secs(120));
    let pastes = r.os.pastes();
    assert_eq!(pastes.len(), 1, "{:?}", r.os.log.lock().unwrap());
    let t = pastes[0].to_lowercase();
    assert!(t.contains("ask not what your country"), "{t}");
    let e = r
        .history
        .lock()
        .unwrap()
        .latest(Retention::Keep, i64::MAX / 2)
        .unwrap()
        .unwrap();
    assert_eq!(e.lang.as_deref(), Some("en"));
    assert!(e.words > 15);
}

#[test]
fn missing_llm_plugin_still_dictates_raw() {
    let a = mock_core("send the notes to the team");
    let st = a.load_llm_now(Some(LlmSpec {
        // Any existing file passes the GGUF check; the plugin lookup fails first.
        model_path: PathBuf::from(env!("CARGO_MANIFEST_PATH")),
        lib_path: Some(PathBuf::from("/nonexistent/libflow_llm.so")),
        threads: Some(1),
    }));
    assert!(matches!(st, LlmStatus::PluginMissing { .. }), "{st:?}");
    let mut r = rig(a, FakeOs::new("notepad.exe"), Prefs::default());
    r.dictate(&tone(1.2), Duration::from_secs(10));
    assert_eq!(r.os.pastes().len(), 1);
    let e = r
        .history
        .lock()
        .unwrap()
        .latest(Retention::Keep, i64::MAX / 2)
        .unwrap()
        .unwrap();
    assert!(e.formatted.is_none());
}

/// The real llama.cpp plugin (`cargo build --release -p flow-llm` in the
/// repo root) and a Qwen GGUF (`tools/fetch-models.sh --llm`); skips
/// without them. FLOW_LLM_LIB / FLOW_LLM_MODEL override the lookup.
fn real_llm() -> Option<LlmSpec> {
    let plugin = flow_windows::core_adapter::llm_plugin_file();
    let lib = find(
        "FLOW_LLM_LIB",
        &[
            &format!("../../target/release/{plugin}"),
            &format!("../../target/debug/{plugin}"),
        ],
    );
    let model = find(
        "FLOW_LLM_MODEL",
        &[
            "../../models/qwen2.5-0.5b-instruct-q4_k_m.gguf",
            "../../../../../models/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        ],
    );
    match (lib, model) {
        (Some(lib), Some(model)) => Some(LlmSpec {
            model_path: model,
            lib_path: Some(lib),
            threads: None,
        }),
        _ => {
            eprintln!("SKIP: flow_llm plugin or Qwen GGUF not found");
            None
        }
    }
}

/// Generous: this shared CPU box runs 0.5B at 0.75-2.7 s per call.
fn llm_prefs() -> Prefs {
    Prefs {
        llm_enabled: true,
        llm_timeout_ms: 30_000,
        live_preview: false,
        ..Prefs::default()
    }
}

#[test]
fn real_llm_plugin_formats_through_the_shell() {
    let Some(spec) = real_llm() else { return };
    let messy = "um so i think we should uh meet tomorrow at three to go over the budget";
    let a = mock_core(messy);
    let st = a.load_llm_now(Some(spec));
    assert!(st.is_ready(), "{st:?}");
    let mut r = rig(a, FakeOs::new("outlook.exe"), llm_prefs());
    r.dictate(&tone(2.0), Duration::from_secs(60));
    let pastes = r.os.pastes();
    assert_eq!(pastes.len(), 1, "{:?}", r.os.log.lock().unwrap());
    let e = r
        .history
        .lock()
        .unwrap()
        .latest(Retention::Keep, i64::MAX / 2)
        .unwrap()
        .unwrap();
    eprintln!("raw: {}\npasted: {}", e.raw, pastes[0]);
    assert_eq!(e.raw, messy, "raw ASR kept for undo-AI-edit");
    assert_eq!(
        e.formatted.as_deref(),
        Some(pastes[0].as_str()),
        "FORMATTED: the LLM output passed the guardrail and was pasted"
    );
    let t = pastes[0].to_lowercase();
    assert!(t.contains("budget") && t.contains("tomorrow"), "{t}");
}

#[cfg(feature = "whisper")]
#[test]
fn real_whisper_then_real_llm_in_the_shell_process() {
    let Some(spec) = real_llm() else { return };
    let model = find(
        "FLOW_MODEL",
        &[
            "../../models/ggml-tiny.en.bin",
            "../../../../../models/ggml-tiny.en.bin",
        ],
    );
    let wav = find(
        "FLOW_WAV",
        &["../../models/jfk.wav", "../../../../../models/jfk.wav"],
    );
    let (Some(model), Some(wav)) = (model, wav) else {
        eprintln!("SKIP: tiny model / jfk.wav not found");
        return;
    };
    let pcm: Vec<i16> = hound::WavReader::open(&wav)
        .unwrap()
        .samples::<i16>()
        .map(Result::unwrap)
        .collect();
    let a = FlowCoreAdapter::new();
    assert!(a
        .load_now(&ModelSpec::Whisper {
            path: model,
            threads: None
        })
        .is_ready());
    // whisper's ggml is linked into this test binary; llama's lives in the
    // plugin (D10). Both must coexist in one process.
    assert!(a.load_llm_now(Some(spec)).is_ready());
    let mut r = rig(a, FakeOs::new("notepad.exe"), llm_prefs());
    r.dictate(&pcm, Duration::from_secs(120));
    let pastes = r.os.pastes();
    assert_eq!(pastes.len(), 1, "{:?}", r.os.log.lock().unwrap());
    let t = pastes[0].to_lowercase();
    eprintln!("pasted: {t}");
    assert!(t.contains("your country"), "{t}");
    let e = r
        .history
        .lock()
        .unwrap()
        .latest(Retention::Keep, i64::MAX / 2)
        .unwrap()
        .unwrap();
    assert!(e.raw.to_lowercase().contains("ask not what your country"));
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn copy_last_puts_the_transcript_on_the_clipboard() {
    let mut r = rig(
        mock_core("copy this please"),
        FakeOs::new("notepad.exe"),
        Prefs::default(),
    );
    r.dictate(&tone(1.2), Duration::from_secs(10));
    r.at(5000, Input::Hotkey(HotkeyAction::CopyLast));
    wait_for("copy", || !r.os.copies.lock().unwrap().is_empty());
    assert_eq!(*r.os.copies.lock().unwrap(), r.os.pastes());
    assert_eq!(r.os.pastes().len(), 1, "copy does not paste");
}

#[test]
fn user_correction_becomes_a_suggestion_not_an_entry() {
    let os = FakeOs {
        edit: Some(("whisper flow".into(), "Wispr Flow".into())),
        ..FakeOs::new("slack.exe")
    };
    let prefs = Prefs {
        learn_window_ms: 1_000,
        learn_poll_ms: 100,
        ..Prefs::default()
    };
    let store = Arc::new(Mutex::new(SuggestionStore::in_memory()));
    let mut r = rig_with(
        mock_core("i use whisper flow every day"),
        os,
        prefs,
        Some(store.clone()),
    );
    r.dictate(&tone(1.5), Duration::from_secs(10));
    wait_for("suggestion", || {
        !store.lock().unwrap().data.pending.is_empty()
    });
    let s = store.lock().unwrap().data.pending.clone();
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!(s[0].word, "Wispr Flow");
    assert_eq!(s[0].heard, "whisper flow");
    assert_eq!(s[0].app, "slack.exe");
    wait_for("edited column", || {
        r.history
            .lock()
            .unwrap()
            .latest(Retention::Keep, i64::MAX / 2)
            .unwrap()
            .unwrap()
            .edited
            .is_some()
    });
    let e = r
        .history
        .lock()
        .unwrap()
        .latest(Retention::Keep, i64::MAX / 2)
        .unwrap()
        .unwrap();
    assert!(e.edited.unwrap().contains("Wispr Flow"));
    // Never auto-added: prefs' dictionary is untouched (the controller only
    // reads it), and accepting is a separate, explicit step.
    let accepted = store
        .lock()
        .unwrap()
        .data
        .accept("Wispr Flow", false)
        .unwrap();
    assert_eq!(accepted.word, "Wispr Flow");
}

#[test]
fn learning_off_or_untouched_paste_suggests_nothing() {
    for (learn, edit) in [
        (false, Some(("whisper".to_string(), "Wispr".to_string()))),
        (true, None),
    ] {
        let os = FakeOs {
            edit,
            ..FakeOs::new("notepad.exe")
        };
        let prefs = Prefs {
            learn_from_edits: learn,
            learn_window_ms: 1_000,
            learn_poll_ms: 100,
            ..Prefs::default()
        };
        let store = Arc::new(Mutex::new(SuggestionStore::in_memory()));
        let mut r = rig_with(mock_core("whisper rocks"), os, prefs, Some(store.clone()));
        r.dictate(&tone(1.2), Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1_500));
        assert!(store.lock().unwrap().data.pending.is_empty());
    }
}
