//! Meeting detection (D48): notice a Zoom or Cisco Webex / Jabber call,
//! then ask "Record this meeting?" or start recording, and stop the
//! recording it started when the call ends.
//!
//! Signals (sources in `docs/findings/2026-11-02-meeting-detection-signals.md`):
//!
//! * **Process** (the gate): a known client exe is running. Being open is
//!   never enough to start: the clients sit in the tray all day.
//! * **Window**: Zoom's meeting window (`ZPContentViewWndClass`) or its
//!   floating share toolbar (`ZPFloatToolbarClass`), visible, owned by
//!   `zoom.exe`; Zoom's share host `CptHost.exe` runs only while sharing
//!   in a meeting.
//! * **Audio session** (WASAPI, per process): an *active* capture session
//!   (the microphone) of the client = a call; an active render session of
//!   the client only keeps a call alive (a chat ping plays sound too).
//!
//! The pure part ([`classify`], [`Detector`], [`AutoDetect`]) runs on
//! [`Snapshot`]s, so the tests replay synthetic signal fixtures
//! (`tests/fixtures/meeting-detect/`). The Windows probe ([`win`]) only
//! looks at windows and audio sessions while a known client runs, every
//! [`POLL`]; with detection off it does nothing at all.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the probe looks (process list; windows + audio sessions only
/// while a client runs).
pub const POLL: Duration = Duration::from_secs(3);

/// The setting (`prefs.meeting_detection`). Off by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    /// "Ask me": a prompt "Record this meeting?".
    Ask,
    /// "Start automatically": recording starts, the indicator shows.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum App {
    Zoom,
    Webex,
    Jabber,
}

impl App {
    /// Shown in the prompt and stored as the meeting's app.
    pub fn label(self) -> &'static str {
        match self {
            App::Zoom => "Zoom",
            App::Webex => "Webex",
            App::Jabber => "Jabber",
        }
    }
}

/// What one look at the system saw. Exe names are lowercase file names
/// (`zoom.exe`), as [`crate::context::exe_name`] makes them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    /// Running processes (only the known ones matter).
    pub processes: Vec<String>,
    /// Visible top-level windows of known clients.
    pub windows: Vec<Window>,
    /// Audio sessions of known clients, on every active endpoint.
    pub audio: Vec<AudioSession>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    pub exe: String,
    pub class: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dataflow {
    /// A microphone (capture endpoint).
    Capture,
    /// A speaker / headset output (render endpoint).
    Render,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSession {
    pub exe: String,
    pub flow: Dataflow,
    /// `AudioSessionStateActive` (a stream is running), not just open.
    pub active: bool,
}

/// One client: which exes are it, and which of its signals mean a call.
struct Profile {
    app: App,
    /// Any of these running = the client is open (the gate).
    exes: &'static [&'static str],
    /// Visible window classes (of one of `exes`) that only a meeting shows.
    meeting_windows: &'static [&'static str],
    /// Helper processes that run only inside a meeting.
    meeting_procs: &'static [&'static str],
}

const PROFILES: &[Profile] = &[
    Profile {
        app: App::Zoom,
        exes: &["zoom.exe"],
        meeting_windows: &["ZPContentViewWndClass", "ZPFloatToolbarClass"],
        meeting_procs: &["cpthost.exe"],
    },
    Profile {
        app: App::Webex,
        // The Webex App (several CiscoCollabHost.exe; webexhost.exe joins
        // meetings) and the classic Webex Meetings client (atmgr.exe,
        // webexmta.exe).
        exes: &[
            "ciscocollabhost.exe",
            "webexhost.exe",
            "atmgr.exe",
            "webexmta.exe",
        ],
        meeting_windows: &[],
        meeting_procs: &[],
    },
    Profile {
        app: App::Jabber,
        exes: &["ciscojabber.exe"],
        meeting_windows: &[],
        meeting_procs: &[],
    },
];

/// Every exe the probe has to look for.
pub fn known_exes() -> impl Iterator<Item = &'static str> {
    PROFILES
        .iter()
        .flat_map(|p| p.exes.iter().chain(p.meeting_procs))
        .copied()
}

/// Is any known client running? (The probe looks no further otherwise.)
pub fn any_client(processes: &[String]) -> bool {
    processes
        .iter()
        .any(|p| PROFILES.iter().any(|f| f.exes.contains(&p.as_str())))
}

/// How sure one look is that `app` is in a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Open, nothing more (or only sound playing): keeps a call alive.
    Alive,
    /// Its microphone stream runs; a mic test does that too, so it has to
    /// last longer before it counts ([`Tuning::mic_polls`]).
    Mic,
    /// A meeting-only window or helper process.
    Meeting,
}

/// Per client, the strongest thing this look saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sighting {
    pub app: App,
    pub level: Level,
}

/// What `snap` says about each running client. `Alive` is reported only
/// for a client that plays sound (render session active).
pub fn classify(snap: &Snapshot) -> Vec<Sighting> {
    let mut out = Vec::new();
    for p in PROFILES {
        let mine = |exe: &str| p.exes.contains(&exe);
        if !snap.processes.iter().any(|e| mine(e)) {
            continue;
        }
        let window = snap
            .windows
            .iter()
            .any(|w| mine(&w.exe) && p.meeting_windows.contains(&w.class.as_str()));
        let helper = snap
            .processes
            .iter()
            .any(|e| p.meeting_procs.contains(&e.as_str()));
        let session = |f: Dataflow| {
            snap.audio
                .iter()
                .any(|s| s.active && s.flow == f && mine(&s.exe))
        };
        let level = if window || helper {
            Some(Level::Meeting)
        } else if session(Dataflow::Capture) {
            Some(Level::Mic)
        } else if session(Dataflow::Render) {
            Some(Level::Alive)
        } else {
            None
        };
        if let Some(level) = level {
            out.push(Sighting { app: p.app, level });
        }
    }
    out
}

/// Debounce, in looks ([`POLL`] apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    /// Looks in a row with a meeting window / helper before it counts.
    pub meeting_polls: u32,
    /// Looks in a row with the microphone running before it counts.
    pub mic_polls: u32,
    /// Looks in a row with no sign of the call before it has ended.
    pub end_polls: u32,
}

impl Default for Tuning {
    /// 2 × 3 s for a meeting window, 4 × 3 s for a microphone stream, the
    /// end after 4 × 3 s of nothing (a muted call keeps its streams).
    fn default() -> Self {
        Tuning {
            meeting_polls: 2,
            mic_polls: 4,
            end_polls: 4,
        }
    }
}

/// What the detector wants done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Show "Record this meeting?" (Ask).
    Prompt(App),
    /// Take the prompt down (the call ended, or the user recorded anyway).
    Withdraw,
    /// Start recording (Auto, or the user said yes).
    Start(App),
    /// Stop the recording the detector started (the call ended).
    Stop(App),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No call (or one not yet seen long enough).
    Idle,
    /// A call; the prompt is up.
    Asking(App),
    /// A call recorded by us: stopped when it ends.
    Ours(App),
    /// A call we leave alone until it ends: the user said no, stopped our
    /// recording, or was already recording when it began.
    Ignoring(App),
}

/// The call state machine. Feed it one [`classify`] result per look.
pub struct Detector {
    tuning: Tuning,
    state: State,
    /// Looks in a row the candidate (`app`, its best level) was seen.
    streak: Option<(App, u32, u32)>,
    misses: u32,
}

impl Detector {
    pub fn new(tuning: Tuning) -> Detector {
        Detector {
            tuning,
            state: State::Idle,
            streak: None,
            misses: 0,
        }
    }

    /// The app of the call being followed, if any.
    pub fn current(&self) -> Option<App> {
        match self.state {
            State::Idle => None,
            State::Asking(a) | State::Ours(a) | State::Ignoring(a) => Some(a),
        }
    }

    /// Whether the detector is recording a call (and will stop it).
    pub fn is_ours(&self) -> bool {
        matches!(self.state, State::Ours(_))
    }

    /// One look. `recording` = a meeting recording runs now (started by
    /// anyone).
    pub fn step(&mut self, seen: &[Sighting], mode: Mode, recording: bool) -> Vec<Action> {
        if mode == Mode::Off {
            return self.off();
        }
        match self.state {
            State::Idle => self.idle(seen, mode, recording),
            State::Asking(app) | State::Ours(app) | State::Ignoring(app) => {
                let mut out = Vec::new();
                if let State::Ours(_) = self.state {
                    if !recording {
                        // The user stopped it: don't restart in this call.
                        self.state = State::Ignoring(app);
                    }
                }
                if let State::Asking(_) = self.state {
                    if recording {
                        // Recorded by hand meanwhile: it's theirs.
                        out.push(Action::Withdraw);
                        self.state = State::Ignoring(app);
                    }
                }
                if seen.iter().any(|s| s.app == app) {
                    self.misses = 0;
                    return out;
                }
                self.misses += 1;
                if self.misses < self.tuning.end_polls {
                    return out;
                }
                match self.state {
                    State::Asking(_) => out.push(Action::Withdraw),
                    State::Ours(_) => out.push(Action::Stop(app)),
                    _ => {}
                }
                self.reset();
                out
            }
        }
    }

    fn idle(&mut self, seen: &[Sighting], mode: Mode, recording: bool) -> Vec<Action> {
        let best = seen
            .iter()
            .filter(|s| s.level >= Level::Mic)
            .max_by_key(|s| s.level);
        let Some(best) = best else {
            self.streak = None;
            return Vec::new();
        };
        let (n, meet) = match self.streak {
            Some((app, n, meet)) if app == best.app => {
                (n + 1, meet + (best.level == Level::Meeting) as u32)
            }
            _ => (1, (best.level == Level::Meeting) as u32),
        };
        self.streak = Some((best.app, n, meet));
        let due = n >= self.tuning.mic_polls || meet >= self.tuning.meeting_polls;
        if !due {
            return Vec::new();
        }
        let app = best.app;
        self.streak = None;
        self.misses = 0;
        if recording {
            self.state = State::Ignoring(app);
            return Vec::new();
        }
        match mode {
            Mode::Ask => {
                self.state = State::Asking(app);
                vec![Action::Prompt(app)]
            }
            _ => {
                self.state = State::Ours(app);
                vec![Action::Start(app)]
            }
        }
    }

    fn off(&mut self) -> Vec<Action> {
        // A recording we started stays (it is the user's now).
        let out = match self.state {
            State::Asking(_) => vec![Action::Withdraw],
            _ => Vec::new(),
        };
        self.reset();
        out
    }

    fn reset(&mut self) {
        self.state = State::Idle;
        self.streak = None;
        self.misses = 0;
    }

    /// The user's answer to the prompt (closing it / its timeout = no).
    pub fn answer(&mut self, record: bool) -> Vec<Action> {
        let State::Asking(app) = self.state else {
            return Vec::new();
        };
        if record {
            self.state = State::Ours(app);
            vec![Action::Start(app)]
        } else {
            self.state = State::Ignoring(app);
            Vec::new()
        }
    }
}

/// The shell side of [`AutoDetect`].
pub trait Host: Send + Sync {
    /// Whether a meeting recording runs, and its id.
    fn recording(&self) -> Option<String>;
    /// Start recording a meeting of `app`; the meeting id.
    fn start(&self, app: App) -> Result<String, String>;
    /// Stop the recording `id` (only if it is still the one running).
    fn stop(&self, id: &str) -> Result<(), String>;
    /// Show / take down the "Record this meeting?" prompt.
    fn prompt(&self, app: Option<App>);
    /// A line for the debug log.
    fn log(&self, what: &str) {
        let _ = what;
    }
}

/// [`Detector`] wired to the recorder: starts and stops only recordings
/// it started itself (a recording started by hand is never stopped by a
/// call ending).
pub struct AutoDetect {
    host: Arc<dyn Host>,
    inner: Mutex<Auto>,
}

struct Auto {
    detector: Detector,
    /// The recording the detector started (and may stop).
    ours: Option<String>,
    /// The last look's signals, logged when they change (the manual check
    /// on a real PC reads them from the debug log).
    last: String,
}

impl AutoDetect {
    pub fn new(host: Arc<dyn Host>, tuning: Tuning) -> AutoDetect {
        AutoDetect {
            host,
            inner: Mutex::new(Auto {
                detector: Detector::new(tuning),
                ours: None,
                last: String::new(),
            }),
        }
    }

    /// One poll of the watcher: with detection off nothing is probed,
    /// asked of the recorder or logged (D48: nothing runs while off); a
    /// prompt still up when it is turned off is taken down.
    pub fn look(&self, probe: &mut dyn Probe, mode: Mode) {
        if mode == Mode::Off {
            self.off();
        } else {
            let snap = probe.snapshot();
            self.tick(&snap, mode);
        }
    }

    /// Detection is off: leave any call (a recording we started stays,
    /// it is the user's now) and forget the last signals, so turning it
    /// on again logs them afresh. No host call unless a prompt was up.
    fn off(&self) {
        let mut g = self.inner.lock().unwrap();
        g.last.clear();
        let actions = g.detector.step(&[], Mode::Off, false);
        self.run(&mut g, actions);
    }

    /// One look at the system.
    pub fn tick(&self, snap: &Snapshot, mode: Mode) {
        if mode == Mode::Off {
            self.off();
            return;
        }
        let seen = classify(snap);
        let mut g = self.inner.lock().unwrap();
        let signals = describe(snap);
        if signals != g.last {
            self.host.log(&format!("signals: {signals}"));
            g.last = signals;
        }
        let rec = self.host.recording();
        // "Ours" only while the recording running is the one we started.
        let ours_running = matches!((&g.ours, &rec), (Some(a), Some(b)) if a == b);
        let recording = if g.detector.is_ours() {
            ours_running
        } else {
            rec.is_some()
        };
        let actions = g.detector.step(&seen, mode, recording);
        self.run(&mut g, actions);
    }

    /// The prompt's answer.
    pub fn answer(&self, record: bool) {
        let mut g = self.inner.lock().unwrap();
        let actions = g.detector.answer(record);
        if !record {
            self.host.log("prompt declined");
        }
        self.host.prompt(None);
        self.run(&mut g, actions);
    }

    fn run(&self, g: &mut Auto, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Prompt(app) => {
                    self.host.log(&format!("{} call: asking", app.label()));
                    self.host.prompt(Some(app));
                }
                Action::Withdraw => {
                    self.host.log("prompt withdrawn");
                    self.host.prompt(None);
                }
                Action::Start(app) => match self.host.start(app) {
                    Ok(id) => {
                        self.host
                            .log(&format!("{} call: recording {id}", app.label()));
                        g.ours = Some(id);
                    }
                    Err(e) => {
                        // Next look sees no recording and leaves this call.
                        self.host
                            .log(&format!("{} call: could not record: {e}", app.label()));
                    }
                },
                Action::Stop(app) => {
                    if let Some(id) = g.ours.take() {
                        self.host
                            .log(&format!("{} call ended: stopping {id}", app.label()));
                        if let Err(e) = self.host.stop(&id) {
                            self.host.log(&format!("stop {id}: {e}"));
                        }
                    }
                }
            }
        }
        if !g.detector.is_ours() {
            g.ours = None;
        }
    }
}

/// The signals of known clients in one line (exes, window classes,
/// session states; never titles), for the debug log.
pub fn describe(snap: &Snapshot) -> String {
    let known: Vec<&str> = known_exes().collect();
    let mut procs: Vec<&str> = snap
        .processes
        .iter()
        .map(String::as_str)
        .filter(|p| known.contains(p))
        .collect();
    procs.sort_unstable();
    procs.dedup();
    if procs.is_empty() {
        return "no meeting app running".into();
    }
    let mut parts = Vec::new();
    for w in &snap.windows {
        parts.push(format!("window {}:{}", w.exe, w.class));
    }
    for a in &snap.audio {
        let f = match a.flow {
            Dataflow::Capture => "mic",
            Dataflow::Render => "out",
        };
        let st = if a.active { "active" } else { "idle" };
        parts.push(format!("{f} {}:{st}", a.exe));
    }
    parts.sort_unstable();
    parts.dedup();
    parts.insert(0, procs.join(","));
    parts.join(" · ")
}

/// Where the looks come from (the Windows probe; a fixture in tests).
pub trait Probe: Send {
    fn snapshot(&mut self) -> Snapshot;
}

/// Look every [`POLL`] while the setting is on; never looks while it is
/// off (`mode()` is read each time, so the setting applies at once).
pub fn spawn_watcher(
    auto: Arc<AutoDetect>,
    mut probe: Box<dyn Probe>,
    mode: Arc<dyn Fn() -> Mode + Send + Sync>,
) {
    let _ = std::thread::Builder::new()
        .name("murmur-meeting-detect".into())
        .spawn(move || {
            super::lower_thread_priority();
            loop {
                auto.look(probe.as_mut(), mode());
                std::thread::sleep(POLL);
            }
        });
}

#[cfg(windows)]
pub mod win;

#[cfg(test)]
#[path = "detect_tests.rs"]
mod tests;
