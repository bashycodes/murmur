//! Notetaker (D45): record a meeting on this PC, then transcribe it into
//! "Me" / "Others" turns.
//!
//! * [`record`]: the microphone and the system audio (WASAPI loopback) as
//!   two aligned 16 kHz WAVs, written while recording.
//! * [`transcript`]: after the meeting ends, both files go through the
//!   long-recording engine (D44) and are merged by time.
//! * [`store`]: the meetings folder next to the history; only the last
//!   [`store::KEEP_AUDIO`] meetings keep their audio.
//! * Summaries (D53): after a transcript is done, a summary job on the
//!   same queue asks the user's AI cleanup order for a summary and action
//!   items (`flow_core::summary`), stored with the meeting; Regenerate
//!   ([`Meetings::summarise`]) runs it again.
//!
//! [`Meetings`] is the service the shell drives (tray item, shortcut,
//! Meetings page). It is platform independent: the OS captures come in
//! through [`AudioSources`], and the transcription engine through an
//! [`EngineFactory`] that the shell points at **its own** router and
//! on-device engine, built per job on the job's worker thread. A router
//! holds one session at a time (D44), so sharing the dictation router
//! would cancel a dictation at every chunk; with its own engine, dictation
//! keeps working while a meeting records or transcribes.

/// The Notetaker switch (D51). `false` = hidden: the shell builds no
/// [`Meetings`] service and no call detector at launch (no recorder, no
/// resumed transcription, nothing touches the microphone or system
/// audio), the tray has no meeting line, the meeting shortcut is not
/// installed, the meeting commands answer [`NOTETAKER_OFF_MSG`], and the
/// app window shows the static "coming later" Notetaker page. Meetings
/// already on disk are left as they are. This module and its tests are
/// unaffected. Re-enable with `true` here **and** `NOTETAKER_ENABLED` in
/// `ui/src/pages/notetaker.ts` (`notetaker_switch_matches_the_ui` keeps
/// the two equal).
pub const NOTETAKER_ENABLED: bool = false;

/// What a meeting command answers while the Notetaker is hidden.
pub const NOTETAKER_OFF_MSG: &str = "The Notetaker is not available in this version";

/// Regenerate without summaries configured (D53).
pub const SUMMARIES_OFF_MSG: &str = "Meeting summaries are off";

pub mod detect;
pub mod record;
pub mod store;
pub mod transcript;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use flow_core::cleanup::Cleaner;
use flow_core::config::SessionConfig;
use flow_router::meeting::{ChunkTranscriber, MeetingParams};
use serde::Serialize;

use record::{Clock, Recorder, Sink, Source};
use store::{Meeting, MeetingMeta, MeetingStore, MeetingSummary, Status, SummaryStatus};
use transcript::{CancelSlot, CANCELLED};

/// Reports a capture that failed mid-recording (device unplugged).
pub type OnError = Arc<dyn Fn(String) + Send + Sync>;

/// The OS side of a recording.
pub trait AudioSources: Send + Sync {
    /// Start capturing the microphone into `mic` and what the PC plays
    /// into `system`, both as 16 kHz mono. Dropping the returned value
    /// stops both. An error = nothing is recorded.
    fn start(&self, mic: Sink, system: Sink, on_error: OnError) -> Result<Box<dyn Send>, String>;
}

/// Builds the engine one transcription job runs on (its own router).
pub type EngineFactory =
    Arc<dyn Fn() -> Result<Arc<dyn ChunkTranscriber + Send + Sync>, String> + Send + Sync>;
/// The session settings a job's chunks run with (languages, dictionary).
pub type SessionSource = Arc<dyn Fn() -> SessionConfig + Send + Sync>;
/// Wall clock, ms since the Unix epoch.
pub type WallClock = Arc<dyn Fn() -> u64 + Send + Sync>;
/// The cleaner a summary runs on: the user's AI cleanup order at that
/// moment (D53); `None` = no AI engine is on.
pub type SummaryEngine = Arc<dyn Fn() -> Option<Arc<dyn Cleaner>> + Send + Sync>;

/// Meeting summaries (D53): where they run and how.
#[derive(Clone)]
pub struct Summaries {
    pub engine: SummaryEngine,
    /// Call timeout and the dictation flag to yield to; `cancel` is
    /// replaced per job.
    pub options: flow_core::summary::Options,
}

/// The transcript as the summary reads it: one `Me: …` / `Others: …`
/// line per turn, without times or untranscribed pieces.
pub fn summary_input(turns: &[transcript::Turn]) -> String {
    turns
        .iter()
        .filter(|t| t.error.is_none() && !t.text.trim().is_empty())
        .map(|t| format!("{}: {}", t.speaker.label(), t.text.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The session settings of a meeting's chunks: the user's dictation
/// settings (languages, dictionary, spoken cleanup and formatting) without
/// the AI cleanup pass (`raw_mode`): an LLM call per 30 s chunk would
/// multiply the time on a CPU-only PC, and a summary comes later from the
/// whole transcript.
pub fn session_config(prefs: &crate::prefs::Prefs) -> SessionConfig {
    let mut c = prefs.session_config();
    c.raw_mode = true;
    crate::core_adapter::to_core_session(&c)
}

/// The app a meeting is recorded in, for the list: the exe in front
/// (`exe_name` form) unless that is nothing, the desktop/taskbar or
/// Murmur itself (the shortcut pressed from Settings).
pub fn front_app(exe: &str, own_exe: &str) -> Option<String> {
    let exe = exe.trim();
    (!exe.is_empty() && exe != "explorer.exe" && !exe.eq_ignore_ascii_case(own_exe))
        .then(|| exe.to_string())
}

/// Runs a meeting's chunks only while no dictation is running: before
/// each chunk it waits (up to `max_wait`) for `busy` to clear, so a long
/// transcription never competes with a dictation for the CPU. A chunk
/// already running finishes (at most 30 s of audio).
pub struct YieldToDictation {
    inner: Arc<dyn ChunkTranscriber + Send + Sync>,
    busy: Arc<AtomicBool>,
    max_wait: Duration,
    aborted: AtomicBool,
}

impl YieldToDictation {
    pub fn new(
        inner: Arc<dyn ChunkTranscriber + Send + Sync>,
        busy: Arc<AtomicBool>,
        max_wait: Duration,
    ) -> Self {
        YieldToDictation {
            inner,
            busy,
            max_wait,
            aborted: AtomicBool::new(false),
        }
    }
}

impl ChunkTranscriber for YieldToDictation {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> flow_core::CommitResult {
        let t0 = Instant::now();
        while self.busy.load(Ordering::SeqCst)
            && !self.aborted.load(Ordering::SeqCst)
            && t0.elapsed() < self.max_wait
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.inner.transcribe_chunk(pcm, cfg)
    }

    fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
        self.inner.abort()
    }
}

/// Notes which route served each chunk of a job: a debug-log line per
/// chunk (route and engine, never the text) and a count per route for the
/// meeting's JSON ([`Meeting::routes`]).
pub struct RouteTally {
    inner: Arc<dyn ChunkTranscriber + Send + Sync>,
    counts: Mutex<std::collections::BTreeMap<String, u32>>,
    log: Box<dyn Fn(&str) + Send + Sync>,
}

impl RouteTally {
    pub fn new(
        inner: Arc<dyn ChunkTranscriber + Send + Sync>,
        log: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        RouteTally {
            inner,
            counts: Mutex::new(Default::default()),
            log: Box::new(log),
        }
    }

    /// Chunks per route so far.
    pub fn counts(&self) -> std::collections::BTreeMap<String, u32> {
        self.counts.lock().unwrap().clone()
    }
}

/// One chunk's route for the debug log: `route local`, `route remote
/// (server x, core y)`, `route cloud (openrouter-1, model z)`, plus the
/// fallback reason when the first route was not used.
pub fn route_line(r: &flow_core::CommitResult) -> (String, String) {
    let Some(ri) = r.route.as_ref() else {
        return ("none".into(), "route none".into());
    };
    let route = if ri.engine.is_empty() {
        "none".to_string()
    } else {
        ri.engine.clone()
    };
    let mut engine: Vec<String> = Vec::new();
    for (k, v) in [
        ("provider", &ri.provider),
        ("model", &ri.model),
        ("server", &ri.server),
        ("core", &ri.server_core),
        ("fallback", &ri.fallback),
    ] {
        if let Some(v) = v {
            engine.push(format!("{k} {v}"));
        }
    }
    let line = if engine.is_empty() {
        format!("route {route}")
    } else {
        format!("route {route} ({})", engine.join(", "))
    };
    (route, line)
}

impl ChunkTranscriber for RouteTally {
    fn transcribe_chunk(&self, pcm: &[i16], cfg: &SessionConfig) -> flow_core::CommitResult {
        let r = self.inner.transcribe_chunk(pcm, cfg);
        let (route, line) = route_line(&r);
        let n = {
            let mut c = self.counts.lock().unwrap();
            *c.entry(route).or_default() += 1;
            c.values().sum::<u32>()
        };
        (self.log)(&format!(
            "chunk {n}: {} ms audio, {line}, {:?}",
            pcm.len() as u64 * 1000 / 16_000,
            r.status
        ));
        r
    }

    fn abort(&self) {
        self.inner.abort()
    }
}

fn log_line(what: &str) {
    eprintln!("[meetings] {what}");
    crate::debug_log::event("meetings", what);
}

/// What the tray, the indicator and the Meetings page show.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct View {
    /// The meeting being recorded now.
    pub recording: Option<Recording>,
    /// The meeting being transcribed now.
    pub transcribing: Option<Progress>,
    /// Meetings waiting for transcription after the current one.
    pub queued: usize,
    /// The meeting whose summary is being made now (D53).
    pub summarising: Option<String>,
    /// The last capture problem of this recording ("" = none).
    pub warning: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recording {
    pub id: String,
    /// Wall clock (the page counts the elapsed time from it).
    pub started_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Progress {
    pub id: String,
    /// 0-100.
    pub percent: f32,
}

pub type Listener = Box<dyn Fn(&View) + Send + Sync>;

/// A queued piece of work: a meeting's transcription or its summary.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    Transcribe(String),
    Summarise(String),
}

impl Job {
    fn id(&self) -> &str {
        match self {
            Job::Transcribe(id) | Job::Summarise(id) => id,
        }
    }
}

struct Active {
    id: String,
    started_ms: u64,
    recorder: Recorder,
    capture: Box<dyn Send>,
}

struct Inner {
    store: MeetingStore,
    sources: Arc<dyn AudioSources>,
    engine: EngineFactory,
    session: SessionSource,
    wall: WallClock,
    mono: Clock,
    params: MeetingParams,
    keep_audio: usize,
    summaries: Option<Summaries>,
    listener: Mutex<Option<Listener>>,
    active: Mutex<Option<Active>>,
    job: Mutex<Option<Progress>>,
    /// The meeting being summarised and its job's cancel flag.
    summary_job: Mutex<Option<(String, Arc<AtomicBool>)>>,
    queue: Mutex<VecDeque<Job>>,
    warning: Mutex<String>,
    cancel: CancelSlot,
    /// Held while a meeting file is checked and written (delete vs. the
    /// job saving its result).
    io: Mutex<()>,
    jobs: Mutex<Sender<()>>,
}

impl Inner {
    fn view(&self) -> View {
        View {
            recording: self.active.lock().unwrap().as_ref().map(|a| Recording {
                id: a.id.clone(),
                started_ms: a.started_ms,
            }),
            transcribing: self.job.lock().unwrap().clone(),
            queued: self
                .queue
                .lock()
                .unwrap()
                .iter()
                .filter(|j| matches!(j, Job::Transcribe(_)))
                .count(),
            summarising: self
                .summary_job
                .lock()
                .unwrap()
                .as_ref()
                .map(|j| j.0.clone()),
            warning: self.warning.lock().unwrap().clone(),
        }
    }

    fn notify(&self) {
        let v = self.view();
        if let Some(l) = self.listener.lock().unwrap().as_ref() {
            l(&v);
        }
    }

    fn enqueue(&self, id: String) {
        self.push(Job::Transcribe(id));
    }

    fn push(&self, job: Job) {
        {
            let mut q = self.queue.lock().unwrap();
            if !q.contains(&job) {
                q.push_back(job);
            }
        }
        let _ = self.jobs.lock().unwrap().send(());
    }

    fn log(&self, what: &str) {
        log_line(what);
    }

    fn run_next(&self) -> bool {
        let Some(job) = self.queue.lock().unwrap().pop_front() else {
            return false;
        };
        match job {
            Job::Transcribe(id) => self.run_job(&id),
            Job::Summarise(id) => self.run_summary(&id),
        }
        true
    }

    /// D53: queue a summary of `m` (a done meeting) and mark it pending in
    /// `m` (the caller saves). No-op without summaries.
    fn want_summary(&self, m: &mut Meeting) -> bool {
        if self.summaries.is_none() {
            return false;
        }
        m.summary = Some(MeetingSummary::pending(m.summary.as_ref()));
        true
    }

    /// Make (or remake) a meeting's summary on the AI cleanup order.
    fn run_summary(&self, id: &str) {
        let Some(summaries) = self.summaries.clone() else {
            return;
        };
        let Some(m) = self.store.get(id) else { return };
        if m.status != Status::Done {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.summary_job.lock().unwrap() = Some((id.to_string(), cancel.clone()));
        self.notify();
        let t0 = Instant::now();
        let input = summary_input(&m.turns);
        let opts = flow_core::summary::Options {
            cancel,
            ..summaries.options.clone()
        };
        let result = flow_core::summary::summarise((summaries.engine)(), &input, &opts);
        {
            let _io = self.io.lock().unwrap();
            if let Some(mut cur) = self.store.get(id) {
                let mut s = MeetingSummary::pending(cur.summary.as_ref());
                match &result {
                    // Quit: stays pending, made at the next start.
                    Err(flow_core::summary::SummaryError::Cancelled) => {}
                    Ok(r) => {
                        s = MeetingSummary {
                            v: store::SUMMARY_VERSION,
                            status: SummaryStatus::Done,
                            text: r.text.clone(),
                            action_items: r.action_items.clone(),
                            language: r.language.clone(),
                            engine: Some(r.engine.clone()),
                            model: r.model.clone(),
                            error: None,
                            made_ms: (self.wall)(),
                        };
                    }
                    Err(e) => {
                        s.status = SummaryStatus::Failed;
                        s.error = Some(e.to_string());
                    }
                }
                cur.summary = Some(s);
                if let Err(e) = self.store.save(&cur) {
                    self.log(&format!("save {id}: {e}"));
                }
            }
        }
        // Sizes, engine and time only, never the text.
        self.log(&match &result {
            Ok(r) => format!(
                "summarised {id}: {} chars in {} calls on {} in {} ms, {} action items",
                input.chars().count(),
                r.calls,
                r.engine,
                t0.elapsed().as_millis(),
                r.action_items.len()
            ),
            Err(e) => format!("summary of {id} stopped: {e}"),
        });
        *self.summary_job.lock().unwrap() = None;
        self.notify();
    }

    fn run_job(&self, id: &str) {
        let Some(mut m) = self.store.get(id) else {
            return;
        };
        if m.status == Status::Done {
            return;
        }
        {
            let _io = self.io.lock().unwrap();
            if self.store.get(id).is_none() {
                return;
            }
            m.status = Status::Transcribing;
            m.error = None;
            if let Err(e) = self.store.save(&m) {
                self.log(&format!("save {id}: {e}"));
            }
        }
        *self.job.lock().unwrap() = Some(Progress {
            id: id.to_string(),
            percent: 0.0,
        });
        self.notify();
        let t0 = std::time::Instant::now();
        let paths = self.store.audio_paths(id);
        let mut last = -1i64;
        let mut tally: Option<Arc<RouteTally>> = None;
        let result = (self.engine)().and_then(|engine| {
            let jid = id.to_string();
            let t = Arc::new(RouteTally::new(engine, move |l| {
                log_line(&format!("{jid} {l}"))
            }));
            tally = Some(t.clone());
            transcript::transcribe_meeting(
                t,
                &(self.session)(),
                [&paths[0], &paths[1]],
                self.params,
                &self.cancel,
                |p| {
                    if p as i64 != last {
                        last = p as i64;
                        if let Some(j) = self.job.lock().unwrap().as_mut() {
                            j.percent = p;
                        }
                        self.notify();
                    }
                },
            )
        });
        {
            let _io = self.io.lock().unwrap();
            match (&result, self.store.get(id)) {
                // Deleted while it ran.
                (_, None) => {}
                // Quit (or deleted): stays "transcribing", resumes next start.
                (Err(e), Some(_)) if e == CANCELLED => {}
                (Ok(turns), Some(mut cur)) => {
                    cur.turns = turns.clone();
                    cur.routes = tally.as_ref().map(|t| t.counts()).unwrap_or_default();
                    cur.status = Status::Done;
                    cur.error = None;
                    // D53: a new transcript gets a new summary.
                    let summarise = self.want_summary(&mut cur);
                    match self.store.save(&cur) {
                        Ok(()) if summarise => self.push(Job::Summarise(id.to_string())),
                        Ok(()) => {}
                        Err(e) => self.log(&format!("save {id}: {e}")),
                    }
                }
                (Err(e), Some(mut cur)) => {
                    cur.status = Status::Failed;
                    cur.error = Some(e.clone());
                    if let Err(e) = self.store.save(&cur) {
                        self.log(&format!("save {id}: {e}"));
                    }
                }
            }
        }
        self.log(&match &result {
            Ok(t) => format!(
                "transcribed {id}: {} turns, {} ms audio in {} ms, chunks by route {:?}",
                t.len(),
                m.duration_ms,
                t0.elapsed().as_millis(),
                tally.as_ref().map(|t| t.counts()).unwrap_or_default()
            ),
            Err(e) => format!("transcription of {id} stopped: {e}"),
        });
        // Prune before the job is cleared, so a meeting seen done (no job
        // running) has had its old audio pruned too.
        self.prune();
        *self.job.lock().unwrap() = None;
        self.notify();
    }

    fn prune(&self) {
        let _io = self.io.lock().unwrap();
        let gone = self.store.prune_audio(self.keep_audio);
        if !gone.is_empty() {
            self.log(&format!("deleted {} old recording files", gone.len()));
        }
    }
}

/// Run the calling thread below normal priority (Windows; elsewhere a
/// no-op), so the scheduler favours a dictation over a meeting job (D45).
pub fn lower_thread_priority() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

fn worker(inner: Weak<Inner>, rx: Receiver<()>) {
    lower_thread_priority();
    while rx.recv().is_ok() {
        loop {
            let Some(i) = inner.upgrade() else { return };
            if !i.run_next() {
                break;
            }
        }
    }
}

/// The Notetaker service (module docs).
pub struct Meetings {
    inner: Arc<Inner>,
}

/// Construction options.
pub struct Options {
    pub dir: std::path::PathBuf,
    pub sources: Arc<dyn AudioSources>,
    pub engine: EngineFactory,
    pub session: SessionSource,
    pub wall: WallClock,
    pub mono: Clock,
    pub params: MeetingParams,
    pub keep_audio: usize,
    /// D53: summaries after transcription; `None` = none are made.
    pub summaries: Option<Summaries>,
}

impl Meetings {
    /// Starts the transcription worker. Call [`Meetings::recover`] once at
    /// startup to finish meetings the last run left unfinished.
    pub fn new(o: Options) -> Meetings {
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(Inner {
            store: MeetingStore::new(o.dir),
            sources: o.sources,
            engine: o.engine,
            session: o.session,
            wall: o.wall,
            mono: o.mono,
            params: o.params,
            keep_audio: o.keep_audio,
            summaries: o.summaries,
            listener: Mutex::new(None),
            active: Mutex::new(None),
            job: Mutex::new(None),
            summary_job: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            warning: Mutex::new(String::new()),
            cancel: Arc::new(Mutex::new(None)),
            io: Mutex::new(()),
            jobs: Mutex::new(tx),
        });
        let weak = Arc::downgrade(&inner);
        let _ = std::thread::Builder::new()
            .name("flow-meeting-jobs".into())
            .spawn(move || worker(weak, rx));
        Meetings { inner }
    }

    /// Called on every change (recording started / stopped, progress,
    /// transcript done), from any thread.
    pub fn on_change(&self, f: impl Fn(&View) + Send + Sync + 'static) {
        *self.inner.listener.lock().unwrap() = Some(Box::new(f));
    }

    pub fn view(&self) -> View {
        self.inner.view()
    }

    pub fn store(&self) -> &MeetingStore {
        &self.inner.store
    }

    pub fn is_recording(&self) -> bool {
        self.inner.active.lock().unwrap().is_some()
    }

    /// Start recording. `app` = the app in front (shown in the list).
    pub fn start(&self, app: Option<String>) -> Result<String, String> {
        let i = &self.inner;
        let mut active = i.active.lock().unwrap();
        if active.is_some() {
            return Err("a meeting is already being recorded".into());
        }
        let started_ms = (i.wall)();
        let id = i.store.new_id(started_ms);
        let m = Meeting {
            id: id.clone(),
            started_ms,
            app,
            status: Status::Recording,
            ..Meeting::default()
        };
        i.store.save(&m)?;
        let started = Recorder::start(i.store.audio_paths(&id), i.mono.clone()).and_then(|rec| {
            let weak = Arc::downgrade(&self.inner);
            let on_error: OnError = Arc::new(move |e: String| {
                if let Some(i) = weak.upgrade() {
                    i.log(&format!("capture: {e}"));
                    *i.warning.lock().unwrap() = e;
                    i.notify();
                }
            });
            let cap = i
                .sources
                .start(rec.sink(Source::Mic), rec.sink(Source::System), on_error)?;
            Ok((rec, cap))
        });
        let (recorder, capture) = match started {
            Ok(x) => x,
            Err(e) => {
                let _ = i.store.delete(&id);
                drop(active);
                i.log(&format!("could not start recording: {e}"));
                *i.warning.lock().unwrap() = format!("Could not start recording: {e}");
                i.notify();
                return Err(e);
            }
        };
        *active = Some(Active {
            id: id.clone(),
            started_ms,
            recorder,
            capture,
        });
        drop(active);
        i.warning.lock().unwrap().clear();
        i.log(&format!("recording {id}"));
        i.notify();
        Ok(id)
    }

    /// Stop recording and queue the transcription. Returns the meeting id.
    pub fn stop(&self) -> Result<String, String> {
        let i = &self.inner;
        let a = i
            .active
            .lock()
            .unwrap()
            .take()
            .ok_or("no meeting is being recorded")?;
        drop(a.capture);
        let rec = a.recorder.stop();
        let mut m = i.store.get(&a.id).unwrap_or(Meeting {
            id: a.id.clone(),
            started_ms: a.started_ms,
            ..Meeting::default()
        });
        match rec {
            Ok(r) => {
                m.duration_ms = r.duration_ms;
                m.system_silent = r.captured[1] == 0;
                m.status = Status::Transcribing;
                i.log(&format!(
                    "stopped {}: {} ms, mic {} / system {} samples",
                    a.id, r.duration_ms, r.captured[0], r.captured[1]
                ));
            }
            Err(e) => {
                m.status = Status::Failed;
                m.error = Some(e);
            }
        }
        i.store.save(&m)?;
        if m.status == Status::Transcribing {
            i.enqueue(a.id.clone());
        }
        i.notify();
        Ok(a.id)
    }

    /// Start, or stop a running recording. `Ok(true)` = recording now.
    pub fn toggle(&self, app: Option<String>) -> Result<bool, String> {
        if self.is_recording() {
            self.stop().map(|_| false)
        } else {
            self.start(app).map(|_| true)
        }
    }

    pub fn list(&self) -> Vec<MeetingMeta> {
        self.inner.store.list()
    }

    pub fn get(&self, id: &str) -> Option<Meeting> {
        self.inner.store.get(id)
    }

    /// The transcript as text (`[0:42] Me: …` per line).
    pub fn text(&self, id: &str) -> Result<String, String> {
        let m = self.get(id).ok_or("no such meeting")?;
        Ok(transcript::to_text(&m.turns))
    }

    /// Delete a meeting (transcript and audio). Not while it records; a
    /// running transcription of it is cancelled.
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let i = &self.inner;
        if i.active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.id == id)
        {
            return Err("stop the recording first".into());
        }
        i.queue.lock().unwrap().retain(|q| q.id() != id);
        let deleted = {
            let _io = i.io.lock().unwrap();
            i.store.delete(id)?
        };
        if i.job.lock().unwrap().as_ref().is_some_and(|j| j.id == id) {
            if let Some(c) = i.cancel.lock().unwrap().as_ref() {
                c.cancel();
            }
        }
        if let Some((_, c)) = i.summary_job.lock().unwrap().as_ref().filter(|j| j.0 == id) {
            c.store(true, Ordering::SeqCst);
        }
        i.notify();
        Ok(deleted)
    }

    /// D53 Regenerate: make the summary of a transcribed meeting again
    /// (or for the first time, for one transcribed before summaries). The
    /// old summary stays until the new one replaces it.
    pub fn summarise(&self, id: &str) -> Result<(), String> {
        let i = &self.inner;
        if i.summaries.is_none() {
            return Err(SUMMARIES_OFF_MSG.into());
        }
        if i.summary_job
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|j| j.0 == id)
            || i.queue
                .lock()
                .unwrap()
                .contains(&Job::Summarise(id.to_string()))
        {
            return Ok(());
        }
        {
            let _io = i.io.lock().unwrap();
            let mut m = i.store.get(id).ok_or("no such meeting")?;
            if m.status != Status::Done {
                return Err("the meeting has no transcript yet".into());
            }
            i.want_summary(&mut m);
            i.store.save(&m)?;
        }
        i.push(Job::Summarise(id.to_string()));
        i.notify();
        Ok(())
    }

    /// Transcribe a failed meeting again (its audio must still be there).
    pub fn retry(&self, id: &str) -> Result<(), String> {
        let i = &self.inner;
        let m = i.store.get(id).ok_or("no such meeting")?;
        if m.status != Status::Failed {
            return Err("only a failed meeting can be transcribed again".into());
        }
        if !i.store.has_audio(id) {
            return Err("its recording was deleted".into());
        }
        i.enqueue(id.to_string());
        i.notify();
        Ok(())
    }

    /// At startup: meetings the last run left recording or transcribing are
    /// transcribed now (their WAVs hold all but the last seconds); without
    /// audio they are marked failed. Old audio is pruned.
    pub fn recover(&self) {
        let i = &self.inner;
        for mut m in i.store.all().into_iter().rev() {
            // D53: a summary the last run did not finish.
            if m.status == Status::Done
                && i.summaries.is_some()
                && m.summary
                    .as_ref()
                    .is_some_and(|s| s.status == SummaryStatus::Pending)
            {
                i.push(Job::Summarise(m.id.clone()));
                continue;
            }
            if !matches!(m.status, Status::Recording | Status::Transcribing) {
                continue;
            }
            if !i.store.has_audio(&m.id) {
                m.status = Status::Failed;
                m.error = Some("the recording was interrupted".into());
                let _ = i.store.save(&m);
                continue;
            }
            if m.status == Status::Recording {
                let p = i.store.audio_path(&m.id, Source::Mic);
                if let Ok(r) = hound::WavReader::open(&p) {
                    m.duration_ms = r.duration() as u64 * 1000 / record::RATE;
                }
                m.status = Status::Transcribing;
                let _ = i.store.save(&m);
            }
            i.log(&format!("resuming {}", m.id));
            i.enqueue(m.id);
        }
        i.prune();
    }

    /// App quit: stop a recording (it is transcribed next start) and
    /// cancel a running transcription (resumed next start).
    pub fn shutdown(&self) {
        let i = &self.inner;
        i.queue.lock().unwrap().clear();
        if self.is_recording() {
            let _ = self.stop();
            i.queue.lock().unwrap().clear();
        }
        if let Some(c) = i.cancel.lock().unwrap().as_ref() {
            c.cancel();
        }
        if let Some((_, c)) = i.summary_job.lock().unwrap().as_ref() {
            c.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests;
