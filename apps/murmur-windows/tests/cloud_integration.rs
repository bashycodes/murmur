//! D18/D28 end to end through the shell: controller → RouterAdapter → the
//! real flow-cloud OpenRouter backend → flow-cloud's in-process mock
//! OpenRouter over HTTP; the local FlowCoreAdapter (MockAsr) is the
//! fallback. Only the OS side and OpenRouter are faked.
//!
//! The mock is reached only through `flow_cloud::testing::override_base_url`
//! (feature `mock-server`, a dev-dependency): every config here is
//! production-shaped (the base URL is a constant, never configurable).
//! The key goes only through the credential store stand-in and
//! `Router::set_cloud_key`; every test checks that nothing the shell shows
//! (last dictation, route view, badge) contains it.

#![cfg(feature = "cloud")]

use flow_cloud::testing::{override_base_url, BaseOverride, MockServer, Reply};
use flow_windows::context::RawSnapshot;
use flow_windows::controller::{AudioSink, Controller, Platform};
use flow_windows::core_adapter::{FlowCoreAdapter, ModelSpec};
use flow_windows::core_api::PasteKey;
use flow_windows::credentials::{self, CredentialStore, MemoryStore};
use flow_windows::dev_settings::{BuildCaps, DevSettings, Route};
use flow_windows::dictation::{Input, OverlayState};
use flow_windows::hotkeys::HotkeyAction;
use flow_windows::prefs::Prefs;
use flow_windows::router_adapter::{router_config, RouterAdapter};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const KEY: &str = "sk-or-v1-LEAKCHECK-windows-0123456789";
const CLOUD_SAYS: &str = "spoken in the cloud";
const LOCAL_SAYS: &str = "spoken on this pc";
const CAPS: BuildCaps = BuildCaps {
    local: true,
    remote: false,
    cloud: true,
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

/// Point flow-cloud at the mock (held for the test; serialised).
fn mock(reply: Reply) -> (MockServer, BaseOverride) {
    let srv = MockServer::fixed(reply);
    let guard = override_base_url(&srv.url("/api/v1"));
    (srv, guard)
}

/// OpenRouter speech-to-text on (or off), with or without the audio
/// consent, in the given route order.
fn dev(consent: bool, allow_cloud: bool, order: Vec<Route>, client: bool) -> DevSettings {
    let mut d = DevSettings::for_build(CAPS);
    d.allow_client = client;
    d.allow_cloud = allow_cloud;
    d.route_order = order;
    if consent {
        d.cloud.openrouter.stt.consent_host = "openrouter.ai".into();
        d.cloud.openrouter.stt.consent_at_ms = 1;
    }
    d
}

struct Rig {
    c: Controller,
    rx: mpsc::Receiver<Input>,
    os: Arc<FakeOs>,
    router: Arc<RouterAdapter>,
    t: Arc<AtomicU64>,
}

fn rig(d: &DevSettings, store: &dyn CredentialStore) -> Rig {
    let local = FlowCoreAdapter::new();
    assert!(local
        .load_now(&ModelSpec::Mock {
            text: LOCAL_SAYS.into()
        })
        .is_ready());
    let router = RouterAdapter::new(router_config(d, CAPS).unwrap(), Some(local), None);
    assert!(router.cloud_capabilities().cloud);
    router.load_cloud_keys(store);
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

fn keyed() -> Arc<MemoryStore> {
    let s = MemoryStore::new();
    credentials::store_openrouter_key(&*s, KEY).unwrap();
    s
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
    fn dictate(&mut self) {
        let start = self.t.load(Ordering::SeqCst) + 10_000;
        self.at(start, Input::Hotkey(HotkeyAction::PttDown));
        self.os.feed(&speech(1500));
        self.at(start + 2500, Input::Hotkey(HotkeyAction::PttUp));
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.c.machine().is_active() {
            let left = deadline.saturating_duration_since(Instant::now());
            let i = self.rx.recv_timeout(left).expect("completion in time");
            self.c.handle(i);
        }
    }
    fn last(&self) -> flow_windows::router_adapter::LastDictation {
        self.router.last().expect("a route was recorded")
    }
    /// Nothing the shell shows or emits contains the key (C3).
    fn assert_no_key(&self) {
        let shown = serde_json::to_string(&self.router.route_view()).unwrap()
            + &serde_json::to_string(&self.last().badge()).unwrap()
            + &format!("{:?}", self.router.router().config())
            + &self.os.log.lock().unwrap().join("\n")
            + &self.os.pastes().join("\n");
        assert!(!shown.contains("LEAKCHECK"), "{shown}");
    }
}

fn said(pastes: &[String], what: &str) -> bool {
    pastes.len() == 1 && pastes[0].to_lowercase().contains(what)
}

/// OpenRouter's `/audio/transcriptions` answer (`usage.cost` in USD).
fn ok_reply() -> Reply {
    Reply::json(
        200,
        serde_json::json!({ "text": CLOUD_SAYS, "usage": {"seconds": 1.5, "cost": 0.0000125} }),
    )
    .header("X-Generation-Id", "gen-win-1")
}

#[test]
fn cloud_first_dictates_through_the_provider() {
    let (srv, _g) = mock(ok_reply());
    let d = dev(true, true, vec![Route::Cloud, Route::Client], true);
    let mut r = rig(&d, &*keyed());
    r.dictate();
    assert!(said(&r.os.pastes(), CLOUD_SAYS), "{:?}", r.os.pastes());
    assert_eq!(srv.count(), 1);
    let req = &srv.requests()[0];
    assert_eq!(req.path(), "/api/v1/audio/transcriptions", "{}", req.target);
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    // D28 defaults: the default model, most private routing, no
    // attribution headers.
    let body = req.json();
    assert_eq!(body["model"], "openai/whisper-large-v3-turbo");
    assert_eq!(body["provider"]["zdr"], true);
    assert_eq!(body["provider"]["data_collection"], "deny");
    assert!(body["input_audio"]["data"]
        .as_str()
        .is_some_and(|d| !d.is_empty()));
    assert_eq!(req.header("http-referer"), None);
    assert_eq!(req.header("x-openrouter-title"), None);
    let last = r.last();
    assert_eq!(last.engine.as_deref(), Some("cloud"));
    assert_eq!(last.fallback, None);
    assert_eq!(last.notice, None);
    let row = last.row.clone().unwrap();
    assert_eq!(
        row, "cloud · OpenRouter · openai/whisper-large-v3-turbo · $0.0000",
        "exact usage.cost, not an estimate"
    );
    assert_eq!(last.cost_usd, Some(0.0000125));
    assert!(!last.cost_estimated);
    let badge = last.badge().unwrap();
    assert_eq!(badge.letter, "C");
    assert_eq!(badge.title.as_deref(), Some("Sent to OpenRouter"));
    assert!(last.always_badge(), "C shows without the S/L setting");
    let usage = r.router.cloud_usage();
    assert_eq!(usage.providers.len(), 1);
    assert_eq!(usage.providers[0].requests, 1);
    assert!(usage.providers[0].billed_audio_ms > 0);
    r.assert_no_key();
}

#[test]
fn usage_survives_a_restart() {
    let (_srv, _g) = mock(ok_reply());
    let d = dev(true, true, vec![Route::Cloud, Route::Client], true);
    let store = keyed();
    let mut r = rig(&d, &*store);
    r.dictate();
    r.dictate();
    let before = r.router.cloud_usage();
    assert_eq!(before.providers[0].requests, 2);
    let dir = std::env::temp_dir().join(format!("flow-win-cloud-usage-{}", std::process::id()));
    let path = dir.join("cloud_usage.json");
    flow_windows::cloud::save_usage(&path, &before).unwrap();
    // "Restart": a new router restores the counters from disk.
    let r2 = rig(&d, &*store);
    assert!(r2.router.cloud_usage().providers.is_empty());
    r2.router
        .router()
        .set_cloud_usage(flow_windows::cloud::load_usage(&path).unwrap());
    assert_eq!(r2.router.cloud_usage(), before);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("LEAKCHECK"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn provider_error_falls_back_to_local_with_all_audio() {
    let (srv, _g) = mock(Reply::json(
        500,
        serde_json::json!({"error": {"message": "upstream exploded"}}),
    ));
    let d = dev(true, true, vec![Route::Cloud, Route::Client], true);
    let mut r = rig(&d, &*keyed());
    r.dictate();
    assert!(said(&r.os.pastes(), LOCAL_SAYS), "{:?}", r.os.pastes());
    assert!(srv.count() >= 1);
    let last = r.last();
    assert_eq!(last.engine.as_deref(), Some("local"));
    assert_eq!(last.fallback.as_deref(), Some("provider_error"));
    assert_eq!(last.audio_ms, 1500, "the local fallback got every sample");
    assert_eq!(last.dropped_samples, 0);
    assert_eq!(
        last.notice.as_deref(),
        Some("Cloud STT failed (the provider returned an error); used this PC")
    );
    let badge = last.badge().unwrap();
    assert_eq!((badge.letter, badge.fallback), ("L", true));
    assert!(badge.message.is_some() && last.always_badge());
    assert!(r.os.errors().is_empty(), "{:?}", r.os.errors());
    r.assert_no_key();
}

#[test]
fn rejected_key_falls_back_and_stays_off_until_the_key_changes() {
    let (srv, _g) = mock(Reply::json(401, serde_json::json!({"error": "bad key"})));
    let d = dev(true, true, vec![Route::Cloud, Route::Client], true);
    let store = keyed();
    let mut r = rig(&d, &*store);
    r.dictate();
    assert!(said(&r.os.pastes(), LOCAL_SAYS));
    assert_eq!(r.last().fallback.as_deref(), Some("auth_failed"));
    assert_eq!(
        r.last().notice.as_deref(),
        Some("Cloud STT failed (key rejected); used this PC")
    );
    // Circuit: the provider is not asked again with the same key.
    r.dictate();
    assert_eq!(srv.count(), 1, "sticky auth_failed");
    assert_eq!(r.last().engine.as_deref(), Some("local"));
    // A new key (Save in the Developer tab) re-enables it.
    credentials::store_openrouter_key(&*store, "sk-or-v1-LEAKCHECK-another-key-000").unwrap();
    r.router.load_cloud_keys(&*store);
    r.dictate();
    assert_eq!(srv.count(), 2);
    r.assert_no_key();
}

#[test]
fn no_key_or_no_consent_never_contacts_the_provider() {
    let (srv, _g) = mock(ok_reply());
    for (consent, store) in [(true, MemoryStore::new()), (false, keyed())] {
        let d = dev(consent, true, vec![Route::Cloud, Route::Client], true);
        let mut r = rig(&d, &*store);
        r.dictate();
        assert!(said(&r.os.pastes(), LOCAL_SAYS), "{:?}", r.os.pastes());
        assert_eq!(r.last().fallback.as_deref(), Some("cloud_not_configured"));
        assert_eq!(
            r.last().notice.as_deref(),
            Some("Cloud STT failed (set a key and accept the consent); used this PC")
        );
    }
    assert_eq!(srv.count(), 0, "C1");
}

#[test]
fn allow_cloud_off_is_silent_and_local() {
    let (srv, _g) = mock(ok_reply());
    let d = dev(true, false, vec![Route::Cloud, Route::Client], true);
    let mut r = rig(&d, &*keyed());
    r.dictate();
    assert!(said(&r.os.pastes(), LOCAL_SAYS));
    assert_eq!(srv.count(), 0, "C1: allow_cloud off");
    let last = r.last();
    assert_eq!((last.fallback, last.notice), (None, None));
}

#[test]
fn cloud_only_failure_is_an_error_pill_naming_the_cloud() {
    let (_srv, _g) = mock(Reply::json(401, serde_json::json!({"error": "bad key"})));
    let d = dev(true, true, vec![Route::Cloud], false);
    let mut r = rig(&d, &*keyed());
    r.dictate();
    assert!(r.os.pastes().is_empty());
    let errs = r.os.errors();
    assert!(
        errs.iter()
            .any(|e| e.contains("Cloud STT failed (key rejected)")),
        "{errs:?}"
    );
    // Without a key it never starts; the pill says what is missing.
    let d = dev(true, true, vec![Route::Cloud], false);
    let mut r = rig(&d, &*MemoryStore::new());
    r.dictate();
    let errs = r.os.errors();
    assert!(
        errs.iter()
            .any(|e| e.contains("Cloud STT failed (set a key and accept the consent)")),
        "{errs:?}"
    );
}

#[test]
fn test_uses_the_saved_settings_and_the_typed_key() {
    let (srv, _g) = mock(ok_reply());
    let d = dev(false, false, vec![], true);
    let r = rig(&d, &*MemoryStore::new());
    let cfg = d.cloud.openrouter.clone();
    let h = r
        .router
        .router()
        .probe_cloud(&cfg, Some(flow_router::SecretString::new(KEY)));
    assert!(h.ok, "{h:?}");
    assert_eq!(h.transcript.as_deref(), Some(CLOUD_SAYS));
    assert_eq!(h.provider.as_deref(), Some("openrouter"));
    assert!(!serde_json::to_string(&h).unwrap().contains("LEAKCHECK"));
    // No typed key and none stored: a clear error, no request.
    let n = srv.count();
    let h = r.router.router().probe_cloud(&cfg, None);
    assert_eq!(h.error.as_deref(), Some("cloud_not_configured"));
    assert_eq!(srv.count(), n);
}

#[test]
fn no_zero_retention_route_falls_back_and_names_the_reason() {
    // OpenRouter's 503 when no endpoint meets `zdr` / `data_collection`.
    let (srv, _g) = mock(Reply::json(
        503,
        serde_json::json!({"error": {"code": 503, "message": "No endpoints found that meet your routing requirements (zdr)"}}),
    ));
    let d = dev(true, true, vec![Route::Cloud, Route::Client], true);
    let mut r = rig(&d, &*keyed());
    r.dictate();
    assert!(said(&r.os.pastes(), LOCAL_SAYS), "{:?}", r.os.pastes());
    let last = r.last();
    assert_eq!(last.fallback.as_deref(), Some("no_private_route"));
    assert_eq!(
        last.notice.as_deref(),
        Some("Cloud STT failed (no zero-retention provider serves this model); used this PC")
    );
    // Sticky until the model or privacy changes: not asked again.
    let n = srv.count();
    r.dictate();
    assert_eq!(srv.count(), n);
    r.assert_no_key();
}

#[test]
fn the_production_config_never_names_a_base_url() {
    // C8: the shell's router JSON carries no URL at all; the host is
    // flow-cloud's constant.
    let d = dev(true, true, vec![Route::Cloud, Route::Client], true);
    let j = d.router_config_json(CAPS, "").to_string();
    assert!(!j.contains("http"), "{j}");
    assert!(!j.contains("base_url"), "{j}");
    assert_eq!(flow_cloud::BASE_URL, "https://openrouter.ai/api/v1");
}
