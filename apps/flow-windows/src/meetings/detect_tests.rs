//! Meeting detection on synthetic signal fixtures
//! (`tests/fixtures/meeting-detect/`): `signals.json` describes what the
//! probe would see (processes, windows, audio sessions), `scenarios.json`
//! replays sequences of looks against [`AutoDetect`] with a fake recorder.

use super::*;
use serde_json::Value;
use std::collections::HashMap;

const SIGNALS: &str = include_str!("../../tests/fixtures/meeting-detect/signals.json");
const SCENARIOS: &str = include_str!("../../tests/fixtures/meeting-detect/scenarios.json");

fn signals() -> HashMap<String, Snapshot> {
    let v: HashMap<String, Value> = serde_json::from_str(SIGNALS).unwrap();
    v.into_iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| (k, serde_json::from_value(v).unwrap()))
        .collect()
}

/// The recorder: one recording at a time; ids m1, m2, … for the
/// detector's, u1, u2, … for the user's.
#[derive(Default)]
struct FakeHost {
    rec: Mutex<Option<String>>,
    events: Mutex<Vec<String>>,
    n: Mutex<(u32, u32)>,
}

impl Host for FakeHost {
    fn recording(&self) -> Option<String> {
        self.rec.lock().unwrap().clone()
    }
    fn start(&self, app: App) -> Result<String, String> {
        let mut r = self.rec.lock().unwrap();
        if r.is_some() {
            return Err("a meeting is already being recorded".into());
        }
        let mut n = self.n.lock().unwrap();
        n.0 += 1;
        let id = format!("m{}", n.0);
        *r = Some(id.clone());
        self.events
            .lock()
            .unwrap()
            .push(format!("start:{}", app.label()));
        Ok(id)
    }
    fn stop(&self, id: &str) -> Result<(), String> {
        let mut r = self.rec.lock().unwrap();
        if r.as_deref() != Some(id) {
            return Err("not running".into());
        }
        *r = None;
        self.events.lock().unwrap().push(format!("stop:{id}"));
        Ok(())
    }
    fn prompt(&self, app: Option<App>) {
        self.events
            .lock()
            .unwrap()
            .push(format!("prompt:{}", app.map(App::label).unwrap_or("-")));
    }
}

impl FakeHost {
    fn user(&self, what: &str) {
        let mut r = self.rec.lock().unwrap();
        match what {
            "start" => {
                assert!(r.is_none(), "user start while recording");
                let mut n = self.n.lock().unwrap();
                n.1 += 1;
                *r = Some(format!("u{}", n.1));
            }
            "stop" => {
                assert!(r.is_some(), "user stop while not recording");
                *r = None;
            }
            other => panic!("unknown user step {other}"),
        }
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

fn mode_of(v: &Value) -> Mode {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn scenarios_replay() {
    let sig = signals();
    let doc: Value = serde_json::from_str(SCENARIOS).unwrap();
    let scenarios = doc["scenarios"].as_array().unwrap();
    assert!(scenarios.len() >= 15);
    for sc in scenarios {
        let name = sc["name"].as_str().unwrap();
        let host = Arc::new(FakeHost::default());
        let auto = AutoDetect::new(host.clone(), Tuning::default());
        let mut mode = mode_of(&sc["mode"]);
        for (i, st) in sc["steps"].as_array().unwrap().iter().enumerate() {
            let at = format!("{name} / step {i}");
            if let Some(m) = st.get("mode") {
                mode = mode_of(m);
            }
            if let Some(u) = st.get("user") {
                host.user(u.as_str().unwrap());
            }
            if let Some(a) = st.get("answer") {
                auto.answer(a.as_bool().unwrap());
            }
            if let Some(l) = st.get("look") {
                let snap = sig
                    .get(l.as_str().unwrap())
                    .unwrap_or_else(|| panic!("{at}: no signal {l}"));
                let times = st.get("times").and_then(Value::as_u64).unwrap_or(1);
                for _ in 0..times {
                    auto.tick(snap, mode);
                }
            }
            let want: Vec<String> = st["expect"]
                .as_array()
                .unwrap_or_else(|| panic!("{at}: no expect"))
                .iter()
                .map(|e| e.as_str().unwrap().to_string())
                .collect();
            assert_eq!(host.take(), want, "{at}");
            if let Some(r) = st.get("recording") {
                assert_eq!(host.recording().is_some(), r.as_bool().unwrap(), "{at}");
            }
        }
    }
}

#[test]
fn every_fixture_signal_is_used_and_parses() {
    let sig = signals();
    assert!(sig.len() >= 15);
    for (k, s) in &sig {
        for a in &s.audio {
            assert!(!a.exe.is_empty() && a.exe == a.exe.to_lowercase(), "{k}");
        }
        assert!(
            SCENARIOS.contains(&format!("\"{k}\"")),
            "signal {k} is in no scenario"
        );
    }
}

fn snap(processes: &[&str]) -> Snapshot {
    Snapshot {
        processes: processes.iter().map(|s| s.to_string()).collect(),
        ..Snapshot::default()
    }
}

#[test]
fn classify_levels() {
    let sig = signals();
    let level = |k: &str| classify(&sig[k]);
    assert_eq!(level("zoom_open"), vec![]);
    assert_eq!(level("nothing"), vec![]);
    assert_eq!(
        level("zoom_open_chat_ping"),
        vec![Sighting {
            app: App::Zoom,
            level: Level::Alive
        }]
    );
    assert_eq!(level("zoom_mic_test")[0].level, Level::Mic);
    assert_eq!(level("zoom_meeting")[0].level, Level::Meeting);
    assert_eq!(level("zoom_meeting_sharing")[0].level, Level::Meeting);
    assert_eq!(level("zoom_window_of_other_app"), vec![]);
    assert_eq!(level("webex_meeting")[0].app, App::Webex);
    assert_eq!(level("webex_meeting")[0].level, Level::Mic);
    assert_eq!(level("jabber_call")[0].app, App::Jabber);
    // CptHost.exe without Zoom running (a leftover) is not a meeting.
    assert_eq!(classify(&snap(&["cpthost.exe"])), vec![]);
}

#[test]
fn the_gate_knows_every_client() {
    assert!(!any_client(
        &snap(&["explorer.exe", "cpthost.exe"]).processes
    ));
    for exe in [
        "zoom.exe",
        "ciscocollabhost.exe",
        "atmgr.exe",
        "ciscojabber.exe",
    ] {
        assert!(any_client(&snap(&[exe]).processes), "{exe}");
    }
    let all: Vec<&str> = known_exes().collect();
    assert!(all.contains(&"cpthost.exe") && all.contains(&"webexhost.exe"));
}

#[test]
fn two_calls_at_once_follow_the_first() {
    let sig = signals();
    let mut both = sig["zoom_meeting"].clone();
    both.processes.extend(sig["jabber_call"].processes.clone());
    both.audio.extend(sig["jabber_call"].audio.clone());
    let mut d = Detector::new(Tuning::default());
    let seen = classify(&both);
    assert_eq!(d.step(&seen, Mode::Auto, false), vec![]);
    // The meeting window outranks Jabber's microphone.
    assert_eq!(
        d.step(&seen, Mode::Auto, false),
        vec![Action::Start(App::Zoom)]
    );
    // Zoom ends while Jabber goes on: Zoom's recording stops.
    let jab = classify(&sig["jabber_call"]);
    let mut out = Vec::new();
    for _ in 0..4 {
        out.extend(d.step(&jab, Mode::Auto, true));
    }
    assert_eq!(out, vec![Action::Stop(App::Zoom)]);
}

#[test]
fn mode_defaults_off_and_round_trips() {
    assert_eq!(Mode::default(), Mode::Off);
    for (m, s) in [
        (Mode::Off, "\"off\""),
        (Mode::Ask, "\"ask\""),
        (Mode::Auto, "\"auto\""),
    ] {
        assert_eq!(serde_json::to_string(&m).unwrap(), s);
        assert_eq!(serde_json::from_str::<Mode>(s).unwrap(), m);
    }
}

#[test]
fn describe_lists_signals_without_titles() {
    let sig = signals();
    let d = describe(&sig["zoom_meeting"]);
    assert_eq!(
        d,
        "zoom.exe · mic zoom.exe:active · out zoom.exe:active · window zoom.exe:ZPContentViewWndClass · window zoom.exe:ZPPTMainFrmWndClassEx"
    );
    assert!(!d.contains("Zoom Meeting"));
    assert_eq!(describe(&sig["nothing"]), "no meeting app running");
}

/// Counts what the detector asks of the system.
#[derive(Default)]
struct Watched {
    probes: Mutex<u32>,
    asks: Mutex<u32>,
    logs: Mutex<Vec<String>>,
    prompts: Mutex<Vec<Option<App>>>,
    snap: Snapshot,
}

impl Probe for Arc<Watched> {
    fn snapshot(&mut self) -> Snapshot {
        *self.probes.lock().unwrap() += 1;
        self.snap.clone()
    }
}

impl Host for Watched {
    fn recording(&self) -> Option<String> {
        *self.asks.lock().unwrap() += 1;
        None
    }
    fn start(&self, _: App) -> Result<String, String> {
        Err("not in this test".into())
    }
    fn stop(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn prompt(&self, app: Option<App>) {
        self.prompts.lock().unwrap().push(app);
    }
    fn log(&self, what: &str) {
        self.logs.lock().unwrap().push(what.to_string());
    }
}

#[test]
fn detection_off_probes_asks_and_logs_nothing() {
    let w = Arc::new(Watched::default());
    let auto = AutoDetect::new(w.clone(), Tuning::default());
    let mut probe = w.clone();
    for _ in 0..100 {
        auto.look(&mut probe, Mode::Off);
    }
    assert_eq!(*w.probes.lock().unwrap(), 0);
    assert_eq!(*w.asks.lock().unwrap(), 0);
    assert!(w.logs.lock().unwrap().is_empty(), "{:?}", w.logs.lock());
    assert!(w.prompts.lock().unwrap().is_empty());
    // Turned on: it looks and logs the signals once.
    auto.look(&mut probe, Mode::Ask);
    auto.look(&mut probe, Mode::Ask);
    assert_eq!(*w.probes.lock().unwrap(), 2);
    assert_eq!(
        *w.logs.lock().unwrap(),
        vec!["signals: no meeting app running".to_string()]
    );
    // Off again, then on: the signals are logged afresh.
    auto.look(&mut probe, Mode::Off);
    auto.look(&mut probe, Mode::Auto);
    assert_eq!(w.logs.lock().unwrap().len(), 2);
}

#[test]
fn turning_detection_off_takes_the_prompt_down() {
    let sig = signals();
    let w = Arc::new(Watched {
        snap: sig["zoom_meeting"].clone(),
        ..Watched::default()
    });
    let auto = AutoDetect::new(w.clone(), Tuning::default());
    let mut probe = w.clone();
    for _ in 0..3 {
        auto.look(&mut probe, Mode::Ask);
    }
    assert_eq!(*w.prompts.lock().unwrap(), vec![Some(App::Zoom)]);
    auto.look(&mut probe, Mode::Off);
    assert_eq!(*w.prompts.lock().unwrap(), vec![Some(App::Zoom), None]);
    let probes = *w.probes.lock().unwrap();
    auto.look(&mut probe, Mode::Off);
    assert_eq!(*w.probes.lock().unwrap(), probes);
}
