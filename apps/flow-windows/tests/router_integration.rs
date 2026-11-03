//! D17 end to end through the shell: controller → RouterAdapter →
//! (flow-remote WebSocket client → in-process flow-server with MockAsr) or
//! the local FlowCoreAdapter (MockAsr), with only the OS side faked.
//!
//! The two mocks say different things, so every paste names the engine
//! that produced it. Covers the four toggle combinations, server down at
//! start, the server dying mid-dictation (no lost audio), and the errors
//! shown on the pill.

#![cfg(feature = "remote")]

use flow_windows::context::RawSnapshot;
use flow_windows::controller::{AudioSink, Controller, Platform};
use flow_windows::core_adapter::{FlowCoreAdapter, ModelSpec};
use flow_windows::core_api::PasteKey;
use flow_windows::dev_settings::{BuildCaps, DevSettings, ServerSettings};
use flow_windows::dictation::{Input, OverlayState};
use flow_windows::hotkeys::HotkeyAction;
use flow_windows::prefs::Prefs;
use flow_windows::router_adapter::{router_config, RouterAdapter};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const TOKEN: &str = "test-token-0123456789abcdef";
const SERVER_SAYS: &str = "spoken on the server";
const LOCAL_SAYS: &str = "spoken on this pc";
const BOTH: BuildCaps = BuildCaps {
    local: true,
    remote: true,
    cloud: false,
};

#[derive(Default)]
struct FakeOs {
    log: Mutex<Vec<String>>,
    sink: Mutex<Option<AudioSink>>,
    pastes: Mutex<Vec<String>>,
}

impl FakeOs {
    fn feed(&self, pcm: &[i16]) {
        for c in pcm.chunks(640) {
            if let Some(s) = self.sink.lock().unwrap().as_mut() {
                s(c);
            }
        }
    }
    fn pastes(&self) -> Vec<String> {
        self.pastes.lock().unwrap().clone()
    }
    fn errors(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("Error"))
            .cloned()
            .collect()
    }
}

impl Platform for FakeOs {
    fn play(&self, _s: flow_windows::dictation::Sound) {}
    fn duck(&self) {}
    fn unduck(&self) {}
    fn snapshot_context(&self) -> Option<RawSnapshot> {
        Some(RawSnapshot {
            exe_path: r"C:\Windows\notepad.exe".into(),
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
    fn send_key(&self, key: PasteKey) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("key {key:?}"));
        Ok(())
    }
    fn overlay(&self, st: &OverlayState) {
        self.log.lock().unwrap().push(format!("overlay {st:?}"));
    }
    fn level(&self, _l: f32) {}
    fn max_duration_warning(&self, _r: u64) {}
}

/// In-process flow-server on 127.0.0.1:0 (MockAsr), on its own runtime.
struct Server {
    handle: flow_server::ServerHandle,
    _rt: tokio::runtime::Runtime,
}

impl Server {
    fn start() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let cfg = flow_server::Config {
            bind: vec!["127.0.0.1:0".into()],
            token: Some(TOKEN.into()),
            model: flow_core::config::ModelConfig {
                asr_backend: flow_core::config::AsrBackend::Mock {
                    text: SERVER_SAYS.into(),
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let handle = rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            flow_server::serve(cfg, vec![l]).await.unwrap()
        });
        Server { handle, _rt: rt }
    }
}

struct Rig {
    c: Controller,
    rx: mpsc::Receiver<Input>,
    os: Arc<FakeOs>,
    router: Arc<RouterAdapter>,
    t: Arc<AtomicU64>,
}

fn dev(server: &Server, allow_server: bool, allow_client: bool) -> DevSettings {
    let mut d = DevSettings {
        allow_server,
        allow_client,
        server: ServerSettings {
            host: "127.0.0.1".into(),
            port: server.handle.addr().port(),
            connect_timeout_ms: 500,
            ..ServerSettings::default()
        },
        token_dpapi: flow_windows::secrets::protect(TOKEN).unwrap(),
        ..DevSettings::for_build(BOTH)
    };
    // D36: server cleanup is its own switch (a configured server with it
    // on cleans this PC's dictations too); "server off" here means not
    // used at all.
    d.set_cleanup_on(flow_router::CleanupRoute::Server, allow_server);
    d
}

fn rig(d: &DevSettings) -> Rig {
    let local = FlowCoreAdapter::new();
    assert!(local
        .load_now(&ModelSpec::Mock {
            text: LOCAL_SAYS.into()
        })
        .is_ready());
    let router = RouterAdapter::new(
        router_config(d, BOTH).unwrap(),
        Some(local),
        Some(flow_remote::factory("flow-windows test")),
    );
    let os = Arc::new(FakeOs::default());
    let (tx, rx) = mpsc::channel();
    let t = Arc::new(AtomicU64::new(0));
    let tc = t.clone();
    let prefs = Prefs {
        live_preview: false,
        developer: d.clone(),
        ..Prefs::default()
    };
    let c = Controller::new(
        os.clone(),
        router.clone(),
        Arc::new(RwLock::new(prefs)),
        tx,
        Box::new(move || tc.load(Ordering::SeqCst)),
    );
    Rig {
        c,
        rx,
        os,
        router,
        t,
    }
}

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

impl Rig {
    fn at(&mut self, ms: u64, i: Input) {
        self.t.store(ms, Ordering::SeqCst);
        self.c.handle(i);
    }
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.c.machine().is_active() {
            let left = deadline.saturating_duration_since(Instant::now());
            let i = self.rx.recv_timeout(left).expect("completion in time");
            self.c.handle(i);
        }
    }
    /// Hold PTT, speak `ms`, release; `mid` runs halfway through.
    fn dictate_with(&mut self, ms: usize, mid: impl FnOnce()) {
        self.at(0, Input::Hotkey(HotkeyAction::PttDown));
        let pcm = speech(ms);
        let (a, b) = pcm.split_at(pcm.len() / 2);
        self.os.feed(a);
        mid();
        self.os.feed(b);
        self.at(1000 + ms as u64, Input::Hotkey(HotkeyAction::PttUp));
        self.settle();
    }
    fn dictate(&mut self) {
        self.dictate_with(1500, || {});
    }
    fn last_engine(&self) -> (Option<String>, Option<String>) {
        let l = self.router.last().expect("a route was recorded");
        (l.engine, l.fallback)
    }
}

fn said(pastes: &[String], what: &str) -> bool {
    pastes.len() == 1 && pastes[0].to_lowercase().contains(what)
}

#[test]
fn on_on_server_up_pastes_the_server_text() {
    let s = Server::start();
    let mut r = rig(&dev(&s, true, true));
    r.dictate();
    assert!(said(&r.os.pastes(), SERVER_SAYS), "{:?}", r.os.pastes());
    assert_eq!(r.last_engine(), (Some("remote".into()), None));
    assert_eq!(s.handle.sessions(), 1);
}

#[test]
fn on_on_server_down_falls_back_and_still_pastes() {
    let s = Server::start();
    s.handle.kill();
    let mut r = rig(&dev(&s, true, true));
    let t0 = Instant::now();
    r.dictate();
    assert!(said(&r.os.pastes(), LOCAL_SAYS), "{:?}", r.os.pastes());
    assert_eq!(
        r.last_engine(),
        (Some("local".into()), Some("connect_failed".into()))
    );
    assert!(t0.elapsed() < Duration::from_secs(10));
}

#[test]
fn on_on_server_dies_mid_dictation_loses_nothing() {
    let s = Server::start();
    let mut r = rig(&dev(&s, true, true));
    let h = &s.handle;
    r.dictate_with(3000, || {
        h.kill();
        std::thread::sleep(Duration::from_millis(200));
    });
    assert!(said(&r.os.pastes(), LOCAL_SAYS), "{:?}", r.os.pastes());
    let (engine, fallback) = r.last_engine();
    assert_eq!(engine.as_deref(), Some("local"));
    assert!(fallback.is_some());
    // I5: the local session got the whole utterance (the router replayed
    // what the server had received), nothing dropped.
    let last = r.router.last().unwrap();
    assert_eq!(last.dropped_samples, 0, "{last:?}");
    assert!(
        (2950..=3050).contains(&last.audio_ms),
        "local saw {} ms of 3000",
        last.audio_ms
    );
}

#[test]
fn on_off_server_up_uses_the_server() {
    let s = Server::start();
    let mut r = rig(&dev(&s, true, false));
    r.dictate();
    assert!(said(&r.os.pastes(), SERVER_SAYS), "{:?}", r.os.pastes());
}

#[test]
fn on_off_server_down_shows_server_unreachable() {
    let s = Server::start();
    s.handle.kill();
    let mut r = rig(&dev(&s, true, false));
    r.dictate();
    assert!(r.os.pastes().is_empty());
    let errs = r.os.errors();
    assert!(
        errs.iter().any(|e| e.contains("Server unreachable")),
        "{errs:?}"
    );
}

/// An in-process flow-server whose LLM ends the text with "!" (D36
/// `clean`), so a paste shows which engine cleaned it.
fn llm_server() -> Server {
    struct Bang;
    impl flow_core::cleanup::Cleaner for Bang {
        fn clean(
            &self,
            p: &flow_core::cleanup::CleanPrompt,
            _d: Instant,
        ) -> Result<String, flow_core::cleanup::CleanError> {
            Ok(format!("{}!", p.transcript.trim().trim_end_matches('.')))
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let model = flow_core::config::ModelConfig {
        asr_backend: flow_core::config::AsrBackend::Mock {
            text: SERVER_SAYS.into(),
        },
        ..Default::default()
    };
    let cfg = flow_server::Config {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model: model.clone(),
        ..Default::default()
    };
    let core = flow_core::FlowCore::from_engines(
        Arc::new(flow_core::asr::MockAsr::new(SERVER_SAYS)),
        Some(Arc::new(Bang) as Arc<dyn flow_core::cleanup::Cleaner>),
    );
    let pool = flow_server::Pool::from_cores(vec![Arc::new(core)], &model, cfg.max_queue);
    let handle = rt.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        flow_server::serve_with_pool(cfg, Arc::new(pool), vec![l])
            .await
            .unwrap()
    });
    Server { handle, _rt: rt }
}

#[test]
fn server_speech_off_server_cleanup_on_cleans_this_pcs_dictation_on_the_server() {
    // D36: speech on this PC only, this PC's AI off, cleanup order this PC
    // → server → cloud with the server on: the server cleans the text and
    // never gets the audio.
    let s = llm_server();
    let mut d = dev(&s, false, true);
    d.set_cleanup_on(flow_router::CleanupRoute::Server, true);
    let mut r = rig(&d);
    r.dictate();
    assert!(
        said(&r.os.pastes(), &format!("{LOCAL_SAYS}!")),
        "{:?}",
        r.os.pastes()
    );
    assert_eq!(r.last_engine(), (Some("local".into()), None));
    let last = r.router.last().unwrap();
    assert_eq!(last.llm_engine.as_deref(), Some("server"), "{last:?}");
    assert_eq!(s.handle.sessions(), 0, "no audio to the server");
    assert!(s.handle.connections() >= 1);
}

#[test]
fn off_on_never_connects() {
    let s = Server::start();
    let mut r = rig(&dev(&s, false, true));
    r.dictate();
    assert!(said(&r.os.pastes(), LOCAL_SAYS), "{:?}", r.os.pastes());
    assert_eq!(s.handle.connections(), 0);
}

#[test]
fn off_off_is_dictation_disabled() {
    let s = Server::start();
    let mut r = rig(&dev(&s, false, false));
    r.dictate();
    assert!(r.os.pastes().is_empty());
    let errs = r.os.errors();
    assert!(
        errs.iter().any(|e| e.contains("Dictation disabled")),
        "{errs:?}"
    );
    assert_eq!(s.handle.connections(), 0);
}

#[test]
fn toggles_apply_to_the_next_dictation() {
    let s = Server::start();
    let mut r = rig(&dev(&s, true, true));
    r.dictate();
    assert_eq!(r.last_engine().0.as_deref(), Some("remote"));
    r.router.apply(&dev(&s, false, true)).unwrap();
    r.at(5_000, Input::Tick);
    r.dictate();
    assert_eq!(r.last_engine().0.as_deref(), Some("local"));
    let pastes = r.os.pastes();
    assert_eq!(pastes.len(), 2, "{pastes:?}");
    assert!(pastes[1].to_lowercase().contains(LOCAL_SAYS));
}

#[test]
fn test_connection_and_test_session_against_a_real_server() {
    let s = Server::start();
    let d = dev(&s, true, true);
    let r = rig(&d);
    let server = flow_windows::router_adapter::server_config(&d, TOKEN).unwrap();
    let h = r.router.router().probe_config(&server);
    assert!(h.ok, "{h:?}");
    assert!(h.rtt_ms.is_some());
    let bad = flow_windows::router_adapter::server_config(&d, "wrong-token-0123456789").unwrap();
    let h = r.router.router().probe_config(&bad);
    assert!(!h.ok);
    assert_eq!(h.error.as_deref(), Some("auth_failed"), "{h:?}");
    let t = r.router.test_session(Prefs::default().session_config());
    assert_eq!(t.engine.as_deref(), Some("remote"), "{t:?}");
    assert_eq!(t.empty_reason.as_deref(), Some("no_speech"), "{t:?}");
}
