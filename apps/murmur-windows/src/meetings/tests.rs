//! The Notetaker service end to end on a synthetic two-source fixture: a
//! fake mic and a fake loopback feed the real recorder (with the loopback's
//! late start and silent gaps), the real long-recording engine chunks the
//! files, and a fake ASR names each chunk by its tone (150 Hz = the user,
//! 300 Hz = the call).

use super::*;
use flow_core::CommitResult;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const S: usize = 16_000;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "flow-meetings-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Voiced "speech" at `hz` with a 4 Hz syllable envelope.
fn voiced(hz: f32, ms: usize) -> Vec<i16> {
    (0..ms * S / 1000)
        .map(|i| {
            let t = i as f32 / S as f32;
            let env = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
            (8000.0 * env * (2.0 * std::f32::consts::PI * hz * t).sin()) as i16
        })
        .collect()
}

fn quiet(ms: usize) -> Vec<i16> {
    let mut x: u32 = 777;
    (0..ms * S / 1000)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((x >> 16) % 41) as i16 - 20
        })
        .collect()
}

fn result(text: &str) -> CommitResult {
    let mut r: CommitResult = serde_json::from_str(
        r#"{"status":"RAW_TRANSCRIPT","asr_text":"","final_text":"","language":"en","timings":{"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},"press_enter":false,"fallback_reason":null,"truncated":false,"error":null}"#,
    )
    .unwrap();
    r.final_text = text.into();
    r
}

/// Names a chunk by the pitch of its loud part.
#[derive(Default)]
struct ToneAsr {
    calls: AtomicUsize,
    /// Block every chunk until this is set to 0 (tests of a busy engine).
    hold: AtomicUsize,
}

impl ChunkTranscriber for ToneAsr {
    fn transcribe_chunk(&self, pcm: &[i16], _cfg: &SessionConfig) -> CommitResult {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let t0 = Instant::now();
        while self.hold.load(Ordering::SeqCst) != 0 && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let loud: Vec<i16> = pcm.iter().copied().filter(|s| s.abs() > 1000).collect();
        let crossings = loud.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        // Crossings per second of loud audio: 2 x the pitch.
        let hz = crossings as f32 * S as f32 / loud.len().max(1) as f32 / 2.0;
        // The user's chunks on this PC, the call's on the server (as if
        // this PC failed over): the meeting counts both.
        let (mut r, engine) = if hz < 220.0 {
            (result(&format!("user line {n}")), "local")
        } else {
            (result(&format!("caller sentence {n}")), "remote")
        };
        r.route = Some(flow_core::RouteInfo {
            engine: engine.into(),
            ..Default::default()
        });
        r
    }
}

type Sinks = Arc<Mutex<Option<(Sink, Sink)>>>;

struct FakeSources {
    sinks: Sinks,
    fail: bool,
}

impl AudioSources for FakeSources {
    fn start(&self, mic: Sink, system: Sink, _e: OnError) -> Result<Box<dyn Send>, String> {
        if self.fail {
            return Err("no microphone".into());
        }
        *self.sinks.lock().unwrap() = Some((mic, system));
        Ok(Box::new(()))
    }
}

struct Rig {
    m: Meetings,
    sinks: Sinks,
    now: Arc<AtomicU64>,
    asr: Arc<ToneAsr>,
    engines_built: Arc<AtomicUsize>,
    dir: PathBuf,
}

fn rig(name: &str, fail: bool) -> Rig {
    rig_with(name, fail, None)
}

fn rig_with(name: &str, fail: bool, summaries: Option<Summaries>) -> Rig {
    let dir = tmp(name);
    let sinks: Sinks = Arc::new(Mutex::new(None));
    let now = Arc::new(AtomicU64::new(50_000));
    let asr = Arc::new(ToneAsr::default());
    let engines_built = Arc::new(AtomicUsize::new(0));
    let n = now.clone();
    let wall_n = now.clone();
    let a = asr.clone();
    let built = engines_built.clone();
    let m = Meetings::new(Options {
        dir: dir.clone(),
        sources: Arc::new(FakeSources {
            sinks: sinks.clone(),
            fail,
        }),
        engine: Arc::new(move || {
            built.fetch_add(1, Ordering::SeqCst);
            Ok(a.clone() as Arc<dyn ChunkTranscriber + Send + Sync>)
        }),
        session: Arc::new(SessionConfig::default),
        wall: Arc::new(move || 1_700_000_000_000 + wall_n.load(Ordering::SeqCst)),
        mono: Arc::new(move || n.load(Ordering::SeqCst)),
        params: MeetingParams {
            min_ms: 2_000,
            soft_max_ms: 10_000,
            max_ms: 15_000,
            ..MeetingParams::default()
        },
        keep_audio: store::KEEP_AUDIO,
        summaries,
    });
    Rig {
        m,
        sinks,
        now,
        asr,
        engines_built,
        dir,
    }
}

impl Rig {
    /// Play a meeting into the fake devices in 10 ms callbacks: `mic` and
    /// `sys` are the full 16 kHz tracks; `sys_on(ms)` says whether the
    /// loopback delivers a block at that time (it sends nothing while
    /// nothing plays).
    fn play(&self, mic: &[i16], sys: &[i16], sys_on: impl Fn(usize) -> bool) {
        let t0 = self.now.load(Ordering::SeqCst);
        let mut g = self.sinks.lock().unwrap();
        let (ms, ss) = g.as_mut().unwrap();
        let blocks = mic.len().max(sys.len()) / 160;
        for b in 0..blocks {
            let r = b * 160..(b + 1) * 160;
            self.now.store(t0 + (b as u64 + 1) * 10, Ordering::SeqCst);
            if let Some(x) = mic.get(r.clone()) {
                ms(x);
            }
            if sys_on(b * 10) {
                if let Some(x) = sys.get(r) {
                    ss(x);
                }
            }
        }
    }

    fn wait_done(&self, id: &str) -> Meeting {
        let t0 = Instant::now();
        loop {
            let m = self.m.get(id).unwrap();
            if matches!(m.status, Status::Done | Status::Failed)
                && self.m.view().transcribing.is_none()
            {
                return m;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(20),
                "job did not finish: {m:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 20 s: the user speaks 1-4 s and 12-15 s, the call 5-10 s; the loopback
/// opens 300 ms late and only sends blocks while the call speaks.
fn two_party() -> (Vec<i16>, Vec<i16>) {
    let mut mic = quiet(1_000);
    mic.extend(voiced(150.0, 3_000));
    mic.extend(quiet(8_000));
    mic.extend(voiced(150.0, 3_000));
    mic.extend(quiet(5_000));
    let mut sys = vec![0; 5 * S];
    sys.extend(voiced(300.0, 5_000));
    sys.extend(vec![0; 10 * S]);
    (mic, sys)
}

#[test]
fn a_recorded_meeting_becomes_me_and_others_turns_in_time_order() {
    let r = rig("e2e", false);
    let id = r.m.start(Some("zoom.exe".into())).unwrap();
    assert_eq!(r.m.view().recording.as_ref().unwrap().id, id);
    assert!(r.m.start(None).is_err(), "one recording at a time");
    let (mic, sys) = two_party();
    r.play(&mic, &sys, |ms| (4_800..10_200).contains(&ms));
    assert_eq!(r.m.stop().unwrap(), id);
    assert!(r.m.view().recording.is_none());
    let m = r.wait_done(&id);
    assert_eq!(m.status, Status::Done, "{m:?}");
    assert_eq!(m.app.as_deref(), Some("zoom.exe"));
    assert!(
        (19_900..=20_100).contains(&m.duration_ms),
        "{}",
        m.duration_ms
    );
    let got: Vec<(transcript::Speaker, u64)> =
        m.turns.iter().map(|t| (t.speaker, t.start_ms)).collect();
    use transcript::Speaker::{Me, Others};
    assert_eq!(got.len(), 3, "{:?}", m.turns);
    let want = [(Me, 1_000u64), (Others, 5_000), (Me, 12_000)];
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g.0, w.0, "{:?}", m.turns);
        assert!(g.1.abs_diff(w.1) <= 300, "{g:?} vs {w:?}");
    }
    assert!(m.turns[0].text.starts_with("user"), "{:?}", m.turns);
    assert!(m.turns[1].text.starts_with("caller"), "{:?}", m.turns);
    let text = r.m.text(&id).unwrap();
    assert!(text.starts_with("[0:01] Me: user line"), "{text}");
    assert!(text.contains("\n[0:05] Others: caller sentence"), "{text}");
    assert_eq!(
        r.engines_built.load(Ordering::SeqCst),
        1,
        "own engine per job"
    );
    assert!(r.m.store().has_audio(&id), "the last 3 keep audio");
    // Which route served the chunks is kept with the meeting.
    let calls = r.asr.calls.load(Ordering::SeqCst) as u32;
    assert!(
        m.routes["local"] >= 1 && m.routes["remote"] >= 1,
        "{:?}",
        m.routes
    );
    assert_eq!(m.routes.values().sum::<u32>(), calls, "{:?}", m.routes);
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(r.dir.join(format!("{id}.json"))).unwrap())
            .unwrap();
    assert_eq!(json["routes"]["local"], m.routes["local"], "{json}");
}

#[test]
fn route_tally_logs_route_and_engine_but_never_the_text() {
    struct Fixed(Vec<Option<flow_core::RouteInfo>>, AtomicUsize);
    impl ChunkTranscriber for Fixed {
        fn transcribe_chunk(&self, _: &[i16], _: &SessionConfig) -> CommitResult {
            let n = self.1.fetch_add(1, Ordering::SeqCst);
            let mut r = result("secret words");
            r.route = self.0[n].clone();
            r
        }
    }
    let cloud = flow_core::RouteInfo {
        engine: "cloud".into(),
        provider: Some("openrouter-1".into()),
        model: Some("some/model".into()),
        fallback: Some("connect_failed".into()),
        ..Default::default()
    };
    let local = flow_core::RouteInfo {
        engine: "local".into(),
        ..Default::default()
    };
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let l = lines.clone();
    let t = RouteTally::new(
        Arc::new(Fixed(
            vec![Some(local.clone()), Some(cloud), None, Some(local)],
            AtomicUsize::new(0),
        )),
        move |x| l.lock().unwrap().push(x.to_string()),
    );
    for _ in 0..4 {
        t.transcribe_chunk(&[0; 16_000], &SessionConfig::default());
    }
    let got = lines.lock().unwrap().clone();
    assert_eq!(got.len(), 4);
    assert!(
        got[0].starts_with("chunk 1: 1000 ms audio, route local,"),
        "{got:?}"
    );
    assert!(
        got[1].contains(
            "route cloud (provider openrouter-1, model some/model, fallback connect_failed)"
        ),
        "{got:?}"
    );
    assert!(got[2].contains("route none"), "{got:?}");
    assert!(got.iter().all(|x| !x.contains("secret")), "{got:?}");
    let c = t.counts();
    assert_eq!((c["local"], c["cloud"], c["none"]), (2, 1, 1));
}

#[test]
fn both_files_are_the_same_length() {
    let r = rig("len", false);
    let id = r.m.start(None).unwrap();
    let (mic, sys) = two_party();
    r.play(&mic, &sys, |ms| (4_800..10_200).contains(&ms));
    r.m.stop().unwrap();
    r.wait_done(&id);
    let [a, b] = r.m.store().audio_paths(&id);
    let la = hound::WavReader::open(a).unwrap().duration();
    let lb = hound::WavReader::open(b).unwrap().duration();
    assert_eq!(la, lb);
}

#[test]
fn a_silent_pc_gives_a_me_only_transcript() {
    let r = rig("silent", false);
    let id = r.m.start(None).unwrap();
    let (mic, _) = two_party();
    r.play(&mic, &[], |_| false);
    r.m.stop().unwrap();
    let m = r.wait_done(&id);
    assert!(m.system_silent);
    assert!(m.turns.iter().all(|t| t.speaker == transcript::Speaker::Me));
    assert_eq!(m.turns.len(), 2);
}

#[test]
fn only_the_last_three_meetings_keep_audio() {
    let r = rig("keep", false);
    let mut ids = Vec::new();
    for i in 0..5u64 {
        r.now.fetch_add(60_000 + i, Ordering::SeqCst);
        let id = r.m.start(None).unwrap();
        r.play(&voiced(150.0, 1_000), &[], |_| false);
        r.m.stop().unwrap();
        r.wait_done(&id);
        ids.push(id);
    }
    let kept: Vec<bool> = ids.iter().map(|id| r.m.store().has_audio(id)).collect();
    assert_eq!(kept, [false, false, true, true, true]);
    assert_eq!(r.m.list().len(), 5, "transcripts stay");
    assert!(r.m.list().iter().all(|m| m.status == Status::Done));
}

#[test]
fn delete_removes_the_meeting_but_not_while_recording() {
    let r = rig("del", false);
    let id = r.m.start(None).unwrap();
    assert!(r.m.delete(&id).is_err());
    r.play(&voiced(150.0, 1_000), &[], |_| false);
    r.m.stop().unwrap();
    r.wait_done(&id);
    assert_eq!(r.m.delete(&id), Ok(true));
    assert!(r.m.get(&id).is_none());
    assert!(!r.m.store().has_audio(&id));
    assert!(r.m.list().is_empty());
}

#[test]
fn deleting_a_meeting_mid_transcription_cancels_it_and_it_stays_deleted() {
    let r = rig("delbusy", false);
    r.asr.hold.store(1, Ordering::SeqCst);
    let id = r.m.start(None).unwrap();
    let (mic, sys) = two_party();
    r.play(&mic, &sys, |ms| (4_800..10_200).contains(&ms));
    r.m.stop().unwrap();
    let t0 = Instant::now();
    while r.asr.calls.load(Ordering::SeqCst) == 0 {
        assert!(t0.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(r.m.view().transcribing.as_ref().unwrap().id, id);
    // While a meeting transcribes, a new recording starts at once (the
    // job runs on its own thread and engine).
    let t = Instant::now();
    let second = r.m.start(None).unwrap();
    assert!(t.elapsed() < Duration::from_secs(1));
    r.m.stop().unwrap();
    assert_eq!(r.m.delete(&id), Ok(true));
    r.asr.hold.store(0, Ordering::SeqCst);
    r.wait_done(&second);
    assert!(r.m.get(&id).is_none(), "the job did not write it back");
    assert!(!r.m.store().has_audio(&id));
}

#[test]
fn a_failed_capture_leaves_nothing_behind() {
    let r = rig("fail", true);
    assert_eq!(r.m.start(None).unwrap_err(), "no microphone");
    assert!(r.m.list().is_empty());
    assert!(!r.m.is_recording());
    assert_eq!(
        r.m.view().warning,
        "Could not start recording: no microphone"
    );
}

#[test]
fn the_app_in_front_is_named_unless_it_is_the_desktop_or_us() {
    assert_eq!(
        front_app("zoom.exe", "murmur.exe").as_deref(),
        Some("zoom.exe")
    );
    assert_eq!(front_app("explorer.exe", "murmur.exe"), None);
    assert_eq!(front_app("murmur.exe", "murmur.exe"), None);
    assert_eq!(front_app("", "murmur.exe"), None);
}

#[test]
fn an_engine_that_cannot_load_fails_the_meeting_and_retry_runs_it_again() {
    let dir = tmp("noengine");
    let sinks: Sinks = Arc::new(Mutex::new(None));
    let ok = Arc::new(AtomicUsize::new(0));
    let o = ok.clone();
    let m = Meetings::new(Options {
        dir: dir.clone(),
        sources: Arc::new(FakeSources {
            sinks: sinks.clone(),
            fail: false,
        }),
        engine: Arc::new(move || {
            if o.load(Ordering::SeqCst) == 0 {
                Err("speech model not found".into())
            } else {
                Ok(Arc::new(ToneAsr::default()) as Arc<dyn ChunkTranscriber + Send + Sync>)
            }
        }),
        session: Arc::new(SessionConfig::default),
        wall: Arc::new(|| 1),
        mono: Arc::new(|| 0),
        params: MeetingParams::default(),
        keep_audio: 3,
        summaries: None,
    });
    let id = m.start(None).unwrap();
    sinks.lock().unwrap().as_mut().unwrap().0(&voiced(150.0, 2_000));
    m.stop().unwrap();
    let wait = |want: Status| {
        let t0 = Instant::now();
        loop {
            let x = m.get(&id).unwrap();
            if x.status == want && m.view().transcribing.is_none() {
                return x;
            }
            assert!(t0.elapsed() < Duration::from_secs(10), "{x:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let failed = wait(Status::Failed);
    assert_eq!(failed.error.as_deref(), Some("speech model not found"));
    assert!(m.store().has_audio(&id), "kept for a retry");
    ok.store(1, Ordering::SeqCst);
    m.retry(&id).unwrap();
    let done = wait(Status::Done);
    assert_eq!(done.turns.len(), 1);
    assert!(m.retry(&id).is_err(), "only failed meetings");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unfinished_meeting_is_transcribed_at_the_next_start() {
    let r = rig("recover", false);
    // A run that quit mid-recording: the meeting file says "recording".
    let id = r.m.start(None).unwrap();
    r.play(&voiced(150.0, 2_000), &[], |_| false);
    // Simulate the crash: the files are left as written so far.
    {
        let mut a = r.m.inner.active.lock().unwrap().take().unwrap();
        drop(std::mem::replace(&mut a.capture, Box::new(())));
        a.recorder.stop().unwrap();
    }
    assert_eq!(r.m.get(&id).unwrap().status, Status::Recording);
    // An old one without audio.
    let lost = Meeting {
        id: "m1".into(),
        started_ms: 1,
        status: Status::Transcribing,
        ..Meeting::default()
    };
    r.m.store().save(&lost).unwrap();
    r.m.recover();
    let m = r.wait_done(&id);
    assert_eq!(m.status, Status::Done);
    assert_eq!(m.duration_ms, 2_000);
    assert_eq!(m.turns.len(), 1);
    assert_eq!(r.m.get("m1").unwrap().status, Status::Failed);
}

#[test]
fn listener_sees_recording_and_progress() {
    let r = rig("events", false);
    let seen = Arc::new(Mutex::new(Vec::<View>::new()));
    let s = seen.clone();
    r.m.on_change(move |v| s.lock().unwrap().push(v.clone()));
    let id = r.m.start(None).unwrap();
    r.play(&voiced(150.0, 1_000), &[], |_| false);
    r.m.stop().unwrap();
    r.wait_done(&id);
    // The job's last notify (after it clears its progress and prunes) can
    // land after wait_done saw the view settle: wait for that event, the
    // one the listener must end on.
    let t0 = Instant::now();
    let v = loop {
        let v = seen.lock().unwrap().clone();
        let ended = v.iter().any(|v| v.transcribing.is_some())
            && v.last().is_some_and(|v| v.transcribing.is_none());
        if ended || t0.elapsed() > Duration::from_secs(20) {
            break v;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(v.first().unwrap().recording.is_some(), "{v:?}");
    assert!(v.iter().any(|v| v.transcribing.is_some()), "{v:?}");
    assert!(v.last().unwrap().transcribing.is_none(), "{v:?}");
}

#[test]
fn chunks_wait_while_a_dictation_runs() {
    let asr = Arc::new(ToneAsr::default());
    let busy = Arc::new(AtomicBool::new(true));
    let y = Arc::new(YieldToDictation::new(
        asr.clone(),
        busy.clone(),
        Duration::from_secs(10),
    ));
    let y2 = y.clone();
    let t = std::thread::spawn(move || {
        y2.transcribe_chunk(&voiced(150.0, 500), &SessionConfig::default())
    });
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        asr.calls.load(Ordering::SeqCst),
        0,
        "waits for the dictation"
    );
    busy.store(false, Ordering::SeqCst);
    let r = t.join().unwrap();
    assert!(r.final_text.starts_with("user"));
    // Cancelled while waiting: no wait for the dictation to end.
    busy.store(true, Ordering::SeqCst);
    y.abort();
    let t0 = Instant::now();
    y.transcribe_chunk(&voiced(150.0, 500), &SessionConfig::default());
    assert!(t0.elapsed() < Duration::from_secs(1));
}

#[test]
fn meeting_chunks_skip_the_ai_cleanup_but_keep_the_dictation_settings() {
    let mut p = crate::prefs::Prefs::default();
    p.dictionary.push(crate::core_api::DictEntry {
        word: "Kubernetes".into(),
        replacement: None,
        is_snippet: false,
    });
    p.languages = vec!["he".into()];
    let c = session_config(&p);
    assert!(c.raw_mode);
    assert_eq!(c.dictionary.entries.len(), 1);
    assert_eq!(c.languages, ["he"]);
}

/// A CPU-heavy engine: every chunk takes `CHUNK` and logs when it ran.
struct SlowAsr {
    tone: ToneAsr,
    ran: Mutex<Vec<(Instant, Instant)>>,
}

const CHUNK: Duration = Duration::from_millis(300);

impl ChunkTranscriber for SlowAsr {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> CommitResult {
        let t0 = Instant::now();
        std::thread::sleep(CHUNK);
        let r = self.tone.transcribe_chunk(pcm, cfg);
        self.ran.lock().unwrap().push((t0, Instant::now()));
        r
    }
}

/// Owner rule (D45): a meeting transcription never blocks a dictation.
/// A dictation started mid-job runs on its own engine and returns at once
/// (at most the one chunk already in flight overlaps it), no new meeting
/// chunk starts while it runs, and the meeting finishes afterwards.
#[test]
fn a_dictation_started_mid_job_is_not_held_up_by_the_meeting() {
    let dir = tmp("yield");
    let sinks: Sinks = Arc::new(Mutex::new(None));
    let now = Arc::new(AtomicU64::new(50_000));
    let slow = Arc::new(SlowAsr {
        tone: ToneAsr::default(),
        ran: Mutex::new(Vec::new()),
    });
    let dictating = Arc::new(AtomicBool::new(false));
    let (s, d, n) = (slow.clone(), dictating.clone(), now.clone());
    let m = Meetings::new(Options {
        dir: dir.clone(),
        sources: Arc::new(FakeSources {
            sinks: sinks.clone(),
            fail: false,
        }),
        engine: Arc::new(move || {
            Ok(Arc::new(YieldToDictation::new(
                s.clone(),
                d.clone(),
                Duration::from_secs(10),
            )) as Arc<dyn ChunkTranscriber + Send + Sync>)
        }),
        session: Arc::new(SessionConfig::default),
        wall: Arc::new(|| 1_700_000_000_000),
        mono: Arc::new(move || n.load(Ordering::SeqCst)),
        params: MeetingParams {
            min_ms: 2_000,
            soft_max_ms: 10_000,
            max_ms: 15_000,
            ..MeetingParams::default()
        },
        keep_audio: store::KEEP_AUDIO,
        summaries: None,
    });
    let r = Rig {
        m,
        sinks,
        now,
        asr: Arc::new(ToneAsr::default()),
        engines_built: Arc::new(AtomicUsize::new(0)),
        dir,
    };
    let id = r.m.start(None).unwrap();
    let (mic, sys) = two_party();
    r.play(&mic, &sys, |ms| (4_800..10_200).contains(&ms));
    r.m.stop().unwrap();

    // The first meeting chunk is running: the user starts dictating.
    let t0 = Instant::now();
    while slow.tone.calls.load(Ordering::SeqCst) == 0 && slow.ran.lock().unwrap().is_empty() {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "the job never started"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // (SlowAsr counts a chunk once it ends; wait for the first to begin.)
    std::thread::sleep(Duration::from_millis(50));
    let dictation_engine = ToneAsr::default();
    let d0 = Instant::now();
    dictating.store(true, Ordering::SeqCst);
    let said = dictation_engine.transcribe_chunk(&voiced(150.0, 1_000), &SessionConfig::default());
    let took = d0.elapsed();
    std::thread::sleep(Duration::from_millis(900)); // the user keeps talking
    let d1 = Instant::now();
    dictating.store(false, Ordering::SeqCst);

    assert!(said.final_text.starts_with("user line"), "{said:?}");
    assert!(took < CHUNK, "dictation waited {took:?}");
    let ran = slow.ran.lock().unwrap().clone();
    let overlapping = ran.iter().filter(|(s, e)| *s < d1 && *e > d0).count();
    assert!(
        overlapping <= 1,
        "{overlapping} meeting chunks ran during the dictation"
    );
    // A chunk whose busy check came just before d0 may begin right after.
    let started_during = ran
        .iter()
        .filter(|(s, _)| *s > d0 + Duration::from_millis(100) && *s < d1)
        .count();
    assert_eq!(started_during, 0, "a new chunk started while dictating");

    let done = r.wait_done(&id);
    assert_eq!(done.status, Status::Done, "{done:?}");
    assert_eq!(done.turns.len(), 3, "{:?}", done.turns);
    assert!(slow.ran.lock().unwrap().len() >= 3);
}

/// One switch, two halves: the shell (this const) and the app window's
/// page (`ui/src/pages/notetaker.ts`) must agree, or the UI would offer a
/// Meetings page whose commands all fail (or hide a working Notetaker).
#[test]
fn notetaker_switch_matches_the_ui() {
    let ui = include_str!("../../ui/src/pages/notetaker.ts");
    let line = format!("export const NOTETAKER_ENABLED = {NOTETAKER_ENABLED};");
    assert!(
        ui.contains(&line),
        "ui/src/pages/notetaker.ts must say `{line}`"
    );
}

// ---- D53: summaries ---------------------------------------------------------

use flow_core::asr::AbortFlag;
use flow_core::cleanup::{CleanError, CleanPrompt};

/// A fake AI engine: answers every summary step with `answer` and keeps
/// what it was given (synthetic transcripts only).
struct FakeLlm {
    answer: Mutex<String>,
    seen: Mutex<Vec<String>>,
}

impl Cleaner for FakeLlm {
    fn clean(&self, p: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
        self.seen
            .lock()
            .unwrap()
            .push(p.source.as_ref().unwrap().text.clone());
        Ok(self.answer.lock().unwrap().clone())
    }
    fn clean_cancellable(
        &self,
        p: &CleanPrompt,
        d: Instant,
        _c: &AbortFlag,
    ) -> Result<String, CleanError> {
        self.clean(p, d)
    }
    fn engine(&self) -> &'static str {
        "server"
    }
}

fn fake_llm(answer: &str) -> Arc<FakeLlm> {
    Arc::new(FakeLlm {
        answer: Mutex::new(answer.into()),
        seen: Mutex::default(),
    })
}

fn summaries_on(llm: Option<Arc<FakeLlm>>) -> Option<Summaries> {
    Some(Summaries {
        engine: Arc::new(move || llm.clone().map(|l| l as Arc<dyn Cleaner>)),
        options: flow_core::summary::Options::default(),
    })
}

impl Rig {
    fn wait_summary(&self, id: &str) -> store::MeetingSummary {
        let t0 = Instant::now();
        loop {
            let m = self.m.get(id).unwrap();
            if let Some(s) = m.summary.filter(|s| s.status != SummaryStatus::Pending) {
                if self.m.view().summarising.is_none() {
                    return s;
                }
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "no summary");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn record_two_party(&self) -> String {
        let id = self.m.start(None).unwrap();
        let (mic, sys) = two_party();
        self.play(&mic, &sys, |ms| (4_800..10_200).contains(&ms));
        self.m.stop().unwrap();
        id
    }
}

const ANSWER: &str = "SUMMARY:\nA made-up planning call.\nACTION ITEMS:\n- Me: send the plan";

#[test]
fn a_transcribed_meeting_gets_a_summary_and_regenerate_remakes_it() {
    let llm = fake_llm(ANSWER);
    let r = rig_with("summary", false, summaries_on(Some(llm.clone())));
    let id = r.record_two_party();
    assert_eq!(r.wait_done(&id).status, Status::Done);
    let s = r.wait_summary(&id);
    assert_eq!(s.status, SummaryStatus::Done, "{s:?}");
    assert_eq!(s.text, "A made-up planning call.");
    assert_eq!(s.action_items, ["Me: send the plan"]);
    assert_eq!(
        (s.engine.as_deref(), s.v),
        (Some("server"), store::SUMMARY_VERSION)
    );
    assert!(s.made_ms > 0);
    // The engine got the turns as "Speaker: text" lines, no times.
    let input = llm.seen.lock().unwrap()[0].clone();
    assert!(input.starts_with("Me: user line"), "{input}");
    assert!(input.contains("\nOthers: caller sentence"), "{input}");
    assert!(!input.contains('['), "{input}");
    // Stored in the meeting's JSON, versioned.
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(r.dir.join(format!("{id}.json"))).unwrap())
            .unwrap();
    assert_eq!(json["summary"]["v"], 1, "{json}");
    assert_eq!(json["summary"]["status"], "done", "{json}");
    // Regenerate.
    *llm.answer.lock().unwrap() = "SUMMARY:\nSecond take.\nACTION ITEMS:\n- None".into();
    r.m.summarise(&id).unwrap();
    let t0 = Instant::now();
    let s = loop {
        let s = r.wait_summary(&id);
        if s.text == "Second take." || t0.elapsed() > Duration::from_secs(20) {
            break s;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((s.text.as_str(), s.action_items.len()), ("Second take.", 0));
    assert_eq!(llm.seen.lock().unwrap().len(), 2);
}

#[test]
fn without_summaries_no_job_runs() {
    let r = rig("nosummary", false);
    let id = r.record_two_party();
    let m = r.wait_done(&id);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(r.m.get(&id).unwrap().summary, None, "{m:?}");
    assert_eq!(r.m.view().summarising, None);
    assert_eq!(r.m.summarise(&id), Err(SUMMARIES_OFF_MSG.into()));
}

#[test]
fn a_summary_with_no_ai_engine_fails_but_the_transcript_stays() {
    let r = rig_with("noengine", false, summaries_on(None));
    let id = r.record_two_party();
    let s = r.wait_summary(&id);
    assert_eq!(s.status, SummaryStatus::Failed);
    assert!(s.error.unwrap().contains("Summaries need AI cleanup"));
    let m = r.m.get(&id).unwrap();
    assert_eq!((m.status, m.turns.len()), (Status::Done, 3));
    // Only a transcribed meeting can be summarised.
    assert!(r.m.summarise("m404").is_err());
}

#[test]
fn a_summary_left_pending_is_made_at_the_next_start() {
    let llm = fake_llm(ANSWER);
    let r = rig_with("summary-recover", false, summaries_on(Some(llm.clone())));
    let m = Meeting {
        id: "m7".into(),
        started_ms: 7,
        status: Status::Done,
        turns: vec![transcript::Turn {
            speaker: transcript::Speaker::Others,
            start_ms: 0,
            end_ms: 1_000,
            text: "a made-up update on the schedule".into(),
            error: None,
        }],
        summary: Some(store::MeetingSummary::pending(None)),
        ..Meeting::default()
    };
    r.m.store().save(&m).unwrap();
    r.m.recover();
    let s = r.wait_summary("m7");
    assert_eq!(s.status, SummaryStatus::Done);
    assert_eq!(
        llm.seen.lock().unwrap().as_slice(),
        ["Others: a made-up update on the schedule"]
    );
}
