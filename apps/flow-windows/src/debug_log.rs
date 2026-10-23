//! Opt-in debug log (Settings → Developer → "Debug log (includes
//! transcripts)"; off by default). One JSON line per dictation, and one per
//! non-dictation error, appended to `%APPDATA%\FlowClone\debug\
//! dictations.jsonl` (last [`MAX_LINES`] kept) and echoed to stdout as
//! `[debug] {json}`. Device-local only: nothing here touches the network.
//!
//! Hard rule: logging never affects dictation. [`DebugLog::log`] only
//! pushes onto a bounded in-memory queue (the oldest entry is dropped when
//! it is full) and returns; a writer thread serialises and writes. Every
//! failure there (serialisation, I/O, disk full, a panicking sink) is
//! swallowed and counted ([`Stats`]), never reported to the caller.
//!
//! Never logged: the server token, cloud API keys (every line goes through
//! [`scrub`]), clipboard contents. Password fields: the target is
//! `"<redacted>"` and so are the transcripts.
//!
//! The line schema is shared with the Android app (docs/DECISIONS.md D21):
//! every key is always present (`null` when unknown); `kind` is
//! `dictation` or `event` (non-dictation: `ts, event, error, stack`).

use serde::Serialize;
use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub const DIR: &str = "debug";
pub const FILE_NAME: &str = "dictations.jsonl";
/// Lines kept in the file (older ones are rotated out).
pub const MAX_LINES: usize = 500;
/// Entries waiting for the writer; beyond this the oldest is dropped.
pub const QUEUE_CAP: usize = 256;
/// Target / transcript of a password field.
pub const REDACTED: &str = "<redacted>";

/// What happened to the text (`insert.outcome`; shared with Android).
pub mod outcome {
    pub const INSERTED: &str = "inserted";
    /// Paste/typing failed.
    pub const PASTE_FAILED: &str = "paste_failed";
    /// The focus moved to another window: copied to the clipboard instead.
    pub const TARGET_CHANGED: &str = "target_changed";
    /// A password field got the focus: nothing inserted.
    pub const TARGET_PROTECTED: &str = "target_protected";
    pub const EMPTY: &str = "empty";
    /// The commit failed (every route).
    pub const ERROR: &str = "error";
    /// No result within the processing timeout.
    pub const TIMEOUT: &str = "timeout";
    /// Mic or session start failed.
    pub const START_FAILED: &str = "start_failed";
    pub const CANCELLED: &str = "cancelled";
}

/// `insert`: `{"outcome": …, "steps": […]}`. Steps = the paste plan that
/// ran (`ctrl_v`, `shift_insert`, `type`, `key Enter`, …).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Insert {
    pub outcome: String,
    pub steps: Vec<String>,
}

/// A dictation line (`"kind": "dictation"`). Field names are the
/// cross-platform schema (docs/DECISIONS.md D21); do not rename.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// UTC, ISO 8601 with milliseconds (`2026-10-23T12:34:56.789Z`).
    pub ts: String,
    /// `hold` (push-to-talk) / `tap` (double-tap lock) / `handsfree`.
    pub mode: Option<String>,
    /// Engine that ran: `local` / `remote` (own server) / `cloud`.
    pub route: Option<String>,
    /// Why the preferred route was not used (`connect_failed`,
    /// `disconnected`, `circuit_open`, `timeout`, …).
    pub fallback: Option<String>,
    pub rtt_ms: Option<u32>,
    pub audio_ms: Option<u64>,
    /// Last speech → stop (shell-side [`SilenceTracker`]); `null` when no
    /// speech was heard.
    pub endpoint_wait_ms: Option<u64>,
    pub asr_ms: Option<u64>,
    /// D22: commit waited this long for a decode already in flight
    /// (preview or segment); part of `total_ms`. `null` without a result.
    pub wait_ms: Option<u64>,
    /// D22: pieces that went to ASR (0 = segmentation off).
    pub segments: Option<u64>,
    /// D22: ASR time spent while recording on pieces whose text was used
    /// (not part of `asr_ms`, which is the tail decoded in `commit`).
    pub pre_asr_ms: Option<u64>,
    /// Router wall time of the commit (release → result, network
    /// included for a server result); `null` when the result did not come
    /// through the router.
    pub client_ms: Option<u32>,
    /// LLM + post-processing after ASR: `total_ms - asr_ms`.
    pub post_ms: Option<u64>,
    /// Engine wall time of the commit.
    pub total_ms: Option<u64>,
    /// Stop / release → outcome (text inserted, or the error shown).
    pub perceived_ms: Option<u64>,
    /// Foreground exe name; `<redacted>` for a password field.
    pub target: Option<String>,
    pub insert: Insert,
    pub raw: Option<String>,
    pub text: Option<String>,
    pub error: Option<String>,
    /// Always `null` here (Rust errors carry no stack); kept for the schema.
    pub stack: Option<String>,
}

/// A non-dictation line (`"kind": "event"`): engine load, server probe,
/// hotkey hook, audio stream, paste-last.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Event {
    pub ts: String,
    pub event: String,
    pub error: Option<String>,
    pub stack: Option<String>,
}

/// One JSONL line; `kind` comes first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Line {
    Dictation(Box<Entry>),
    Event(Event),
}

impl Line {
    fn ts_mut(&mut self) -> &mut String {
        match self {
            Line::Dictation(e) => &mut e.ts,
            Line::Event(e) => &mut e.ts,
        }
    }
}

/// Wispr's head-silence rule (shared with Android): a chunk is speech when
/// mean(s²) > 33856 (RMS ≈ 184, ≈ −45 dBFS). Tracks the silence since the
/// last speech for `endpoint_wait_ms`. Lock-free; fed from the mic thread.
#[derive(Debug, Default)]
pub struct SilenceTracker {
    heard: AtomicBool,
    trailing_samples: AtomicU64,
}

impl SilenceTracker {
    pub const SIGNAL_MEAN_SQUARE: f64 = 33_856.0;

    pub fn update(&self, pcm: &[i16]) {
        if pcm.is_empty() {
            return;
        }
        let sum: f64 = pcm.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
        if sum / pcm.len() as f64 > Self::SIGNAL_MEAN_SQUARE {
            self.heard.store(true, Ordering::Relaxed);
            self.trailing_samples.store(0, Ordering::Relaxed);
        } else if self.heard.load(Ordering::Relaxed) {
            self.trailing_samples
                .fetch_add(pcm.len() as u64, Ordering::Relaxed);
        }
    }

    /// Silence since the last speech (16 kHz), `None` before any speech.
    pub fn trailing_silence_ms(&self) -> Option<u64> {
        self.heard
            .load(Ordering::Relaxed)
            .then(|| self.trailing_samples.load(Ordering::Relaxed) / 16)
    }
}

/// Where lines go. The file in production; tests inject failing ones.
pub trait Sink: Send {
    fn append(&mut self, line: &str) -> io::Result<()>;
    fn count(&mut self) -> io::Result<usize>;
    fn clear(&mut self) -> io::Result<()>;
}

/// `%APPDATA%\FlowClone\debug\dictations.jsonl`.
pub fn default_path(appdata: Option<&str>) -> PathBuf {
    let base = appdata.map(PathBuf::from).unwrap_or_default();
    base.join(crate::prefs::APP_DIR).join(DIR).join(FILE_NAME)
}

/// JSONL file keeping the last `max_lines` lines.
pub struct FileSink {
    path: PathBuf,
    max_lines: usize,
    /// Lines in the file (counted lazily once, then tracked).
    lines: Option<usize>,
}

impl FileSink {
    pub fn new(path: PathBuf) -> Self {
        Self::with_max(path, MAX_LINES)
    }

    pub fn with_max(path: PathBuf, max_lines: usize) -> Self {
        FileSink {
            path,
            max_lines: max_lines.max(1),
            lines: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn count_file(&self) -> io::Result<usize> {
        match std::fs::File::open(&self.path) {
            Ok(f) => Ok(io::BufReader::new(f).lines().count()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Keep the last `max_lines` lines (tmp + rename).
    fn rotate(&mut self) -> io::Result<()> {
        let text = std::fs::read_to_string(&self.path)?;
        let lines: Vec<&str> = text.lines().collect();
        let keep = &lines[lines.len().saturating_sub(self.max_lines)..];
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut out = String::with_capacity(text.len());
        for l in keep {
            out.push_str(l);
            out.push('\n');
        }
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path)?;
        self.lines = Some(keep.len());
        Ok(())
    }
}

impl Sink for FileSink {
    fn append(&mut self, line: &str) -> io::Result<()> {
        let n = match self.lines {
            Some(n) => n,
            None => self.count_file()?,
        };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        // One write per line so a crash leaves at most one partial line.
        f.write_all(format!("{line}\n").as_bytes())?;
        self.lines = Some(n + 1);
        if n + 1 > self.max_lines {
            // Unknown count after a failed rotation: recount next time.
            self.lines = None;
            self.rotate()?;
        }
        Ok(())
    }

    fn count(&mut self) -> io::Result<usize> {
        let n = self.count_file()?;
        self.lines = Some(n);
        Ok(n)
    }

    fn clear(&mut self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.lines = Some(0);
        Ok(())
    }
}

/// Counters for the Developer tab (and tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Stats {
    /// Lines written (this run).
    pub written: u64,
    /// Entries dropped because the queue was full.
    pub dropped: u64,
    /// Serialisation / I/O / sink failures (each swallowed).
    pub write_errors: u64,
}

struct Inner {
    queue: Mutex<VecDeque<Line>>,
    cv: Condvar,
    sink: Mutex<Box<dyn Sink>>,
    /// Queued + being written (flush waits for 0).
    pending: AtomicU64,
    written: AtomicU64,
    dropped: AtomicU64,
    write_errors: AtomicU64,
    closed: AtomicBool,
    echo: bool,
    secrets: Mutex<Vec<String>>,
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The debug log. Cheap to call when off (one atomic load).
pub struct DebugLog {
    enabled: AtomicBool,
    inner: Arc<Inner>,
}

impl DebugLog {
    /// Production log: the JSONL file, echoed to stdout.
    pub fn file(path: PathBuf, enabled: bool) -> Self {
        Self::new(Box::new(FileSink::new(path)), enabled, true)
    }

    /// `echo` = also print `[debug] {json}` to stdout.
    pub fn new(sink: Box<dyn Sink>, enabled: bool, echo: bool) -> Self {
        let inner = Arc::new(Inner {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            sink: Mutex::new(sink),
            pending: AtomicU64::new(0),
            written: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            echo,
            secrets: Mutex::new(Vec::new()),
        });
        let w = inner.clone();
        // No writer thread = entries just age out of the queue.
        let _ = std::thread::Builder::new()
            .name("flow-debug-log".into())
            .spawn(move || writer(w));
        DebugLog {
            enabled: AtomicBool::new(enabled),
            inner,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    /// Secrets that must never appear in a line (server token, cloud keys):
    /// [`scrub`] replaces them in every line. Kept in memory only.
    pub fn set_secrets(&self, secrets: Vec<String>) {
        *lock(&self.inner.secrets) = secrets;
    }

    /// Queue a dictation line; never blocks on I/O, never fails. `ts` is
    /// filled in when empty. Ignored while the log is off.
    pub fn log(&self, e: Entry) {
        self.push(Line::Dictation(Box::new(e)));
    }

    /// A non-dictation event (no-op while off).
    pub fn event(&self, event: &str, message: &str) {
        if self.is_enabled() {
            self.push(Line::Event(Event {
                event: event.to_string(),
                error: Some(message.to_string()),
                ..Event::default()
            }));
        }
    }

    fn push(&self, mut e: Line) {
        if !self.is_enabled() {
            return;
        }
        if e.ts_mut().is_empty() {
            *e.ts_mut() = iso8601_ms(now_unix_ms());
        }
        let mut q = lock(&self.inner.queue);
        if q.len() >= QUEUE_CAP {
            q.pop_front();
            self.inner.dropped.fetch_add(1, Ordering::Relaxed);
            self.inner.pending.fetch_sub(1, Ordering::SeqCst);
        }
        q.push_back(e);
        self.inner.pending.fetch_add(1, Ordering::SeqCst);
        drop(q);
        self.inner.cv.notify_one();
    }

    /// Lines in the file (`None` when it can't be read).
    pub fn count(&self) -> Option<usize> {
        lock(&self.inner.sink).count().ok()
    }

    /// Drop queued entries and delete the file.
    pub fn clear(&self) -> Result<(), String> {
        {
            let mut q = lock(&self.inner.queue);
            let n = q.len() as u64;
            q.clear();
            self.inner.pending.fetch_sub(n, Ordering::SeqCst);
        }
        lock(&self.inner.sink).clear().map_err(|e| e.to_string())
    }

    pub fn stats(&self) -> Stats {
        Stats {
            written: self.inner.written.load(Ordering::Relaxed),
            dropped: self.inner.dropped.load(Ordering::Relaxed),
            write_errors: self.inner.write_errors.load(Ordering::Relaxed),
        }
    }

    /// Wait until the writer has handled everything queued (tests, exit).
    pub fn flush(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while self.inner.pending.load(Ordering::SeqCst) > 0 {
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        true
    }
}

impl Drop for DebugLog {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.cv.notify_all();
    }
}

fn writer(inner: Arc<Inner>) {
    loop {
        let entry = {
            let mut q = lock(&inner.queue);
            loop {
                if let Some(e) = q.pop_front() {
                    break e;
                }
                if inner.closed.load(Ordering::SeqCst) {
                    return;
                }
                q = match inner.cv.wait_timeout(q, Duration::from_millis(500)) {
                    Ok((g, _)) => g,
                    Err(p) => p.into_inner().0,
                };
            }
        };
        let ok =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write_one(&inner, entry)))
                .unwrap_or(false);
        if ok {
            inner.written.fetch_add(1, Ordering::Relaxed);
        } else {
            inner.write_errors.fetch_add(1, Ordering::Relaxed);
        }
        inner.pending.fetch_sub(1, Ordering::SeqCst);
    }
}

fn write_one(inner: &Inner, e: Line) -> bool {
    let Ok(line) = serde_json::to_string(&e) else {
        return false;
    };
    let line = scrub(&line, &lock(&inner.secrets));
    if inner.echo {
        let _ = writeln!(io::stdout().lock(), "[debug] {line}");
    }
    lock(&inner.sink).append(&line).is_ok()
}

/// Remove credentials from a line: the given secrets, anything after
/// `Bearer `, and provider-style keys (`sk-…`, `gsk_…`) become
/// [`REDACTED`].
pub fn scrub(text: &str, secrets: &[String]) -> String {
    let mut s = text.to_string();
    for secret in secrets.iter().filter(|x| x.len() >= 6) {
        s = s.replace(secret.as_str(), REDACTED);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("Bearer ") {
        out.push_str(&rest[..i + "Bearer ".len()]);
        out.push_str(REDACTED);
        let tail = &rest[i + "Bearer ".len()..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out.split_inclusive(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .map(|w| {
            let (word, sep) = match w.char_indices().last() {
                Some((i, c)) if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') => {
                    (&w[..i], &w[i..])
                }
                _ => (w, ""),
            };
            let keyish = (word.starts_with("sk-") || word.starts_with("gsk_")) && word.len() >= 16;
            if keyish {
                format!("{REDACTED}{sep}")
            } else {
                w.to_string()
            }
        })
        .collect()
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `2026-10-23T12:34:56.789Z` (UTC) from Unix milliseconds.
pub fn iso8601_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        sod / 3600,
        (sod / 60) % 60,
        sod % 60,
        ms % 1000
    )
}

/// flow-router failure codes (`Failure::as_str`) worth naming in
/// `fallback` when only an error text is left (server-only failures).
pub const FAILURE_CODES: &[&str] = &[
    "connect_failed",
    "auth_failed",
    "proto_mismatch",
    "busy",
    "timeout",
    "disconnected",
    "circuit_open",
    "server_not_configured",
    "local_not_ready",
    "cloud_not_configured",
    "budget_exceeded",
    "quota_exceeded",
    "provider_error",
];

/// The first failure code named in `text`.
pub fn failure_code(text: &str) -> Option<String> {
    FAILURE_CODES
        .iter()
        .filter_map(|c| text.find(c).map(|i| (i, *c)))
        .min()
        .map(|(_, c)| c.to_string())
}

// ------------------------------------------------------------ app-wide

static GLOBAL: OnceLock<Arc<DebugLog>> = OnceLock::new();

/// The app's log, for code without a handle (engine load, hotkeys).
pub fn install(log: Arc<DebugLog>) {
    let _ = GLOBAL.set(log);
}

pub fn global() -> Option<&'static Arc<DebugLog>> {
    GLOBAL.get()
}

/// Log a non-dictation event to the installed log (no-op when off/none).
pub fn event(event: &str, message: &str) {
    if let Some(l) = GLOBAL.get() {
        l.event(event, message);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// In-memory sink (shared so the test can read it back).
    #[derive(Clone, Default)]
    pub struct MemSink(pub Arc<Mutex<Vec<String>>>);

    impl Sink for MemSink {
        fn append(&mut self, line: &str) -> io::Result<()> {
            self.0.lock().unwrap().push(line.to_string());
            Ok(())
        }
        fn count(&mut self) -> io::Result<usize> {
            Ok(self.0.lock().unwrap().len())
        }
        fn clear(&mut self) -> io::Result<()> {
            self.0.lock().unwrap().clear();
            Ok(())
        }
    }

    /// Fails every call (disk full); `panic` = panics instead.
    pub struct BrokenSink {
        pub panic: bool,
    }

    impl Sink for BrokenSink {
        fn append(&mut self, _line: &str) -> io::Result<()> {
            if self.panic {
                panic!("sink exploded");
            }
            Err(io::Error::other("disk full"))
        }
        fn count(&mut self) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }
        fn clear(&mut self) -> io::Result<()> {
            Err(io::Error::other("disk full"))
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "flow-debuglog-{name}-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d.join(FILE_NAME)
    }

    #[test]
    fn failure_codes_are_found_in_error_texts() {
        assert_eq!(
            failure_code("Server unreachable (connect_failed): refused").as_deref(),
            Some("connect_failed")
        );
        assert_eq!(
            failure_code("circuit_open after timeout").as_deref(),
            Some("circuit_open")
        );
        assert_eq!(failure_code("no mic"), None);
    }

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_ms(1_761_222_896_789), "2025-10-23T12:34:56.789Z");
        assert_eq!(iso8601_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn event_lines_have_kind_event_error_and_stack() {
        let v = serde_json::to_value(Line::Event(Event::default())).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["error", "event", "kind", "stack", "ts"]);
        assert_eq!(v["kind"], "event");
        let line = serde_json::to_string(&Line::Event(Event::default())).unwrap();
        assert!(line.starts_with("{\"kind\":\"event\""), "{line}");
    }

    #[test]
    fn silence_tracker_measures_last_speech_to_stop() {
        let t = SilenceTracker::default();
        t.update(&[0; 1600]);
        assert_eq!(t.trailing_silence_ms(), None, "no speech yet");
        t.update(&[1000; 1600]);
        assert_eq!(t.trailing_silence_ms(), Some(0));
        t.update(&[10; 8000]);
        t.update(&[0; 1600]);
        assert_eq!(t.trailing_silence_ms(), Some(600));
        t.update(&[-2000; 160]);
        assert_eq!(t.trailing_silence_ms(), Some(0));
    }

    #[test]
    fn every_schema_key_is_always_present() {
        let v = serde_json::to_value(Line::Dictation(Box::default())).unwrap();
        assert_eq!(v["kind"], "dictation");
        assert_eq!(v["insert"], serde_json::json!({"outcome": "", "steps": []}));
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        let mut want = vec![
            "kind",
            "stack",
            "ts",
            "mode",
            "route",
            "fallback",
            "rtt_ms",
            "audio_ms",
            "endpoint_wait_ms",
            "asr_ms",
            "wait_ms",
            "segments",
            "pre_asr_ms",
            "client_ms",
            "post_ms",
            "total_ms",
            "perceived_ms",
            "target",
            "insert",
            "raw",
            "text",
            "error",
        ];
        want.sort();
        assert_eq!(keys, want);
    }

    #[test]
    fn off_writes_nothing() {
        let mem = MemSink::default();
        let log = DebugLog::new(Box::new(mem.clone()), false, false);
        log.log(Entry::default());
        log.event("engine_load", "failed");
        assert!(log.flush(Duration::from_secs(2)));
        assert!(mem.0.lock().unwrap().is_empty());
        assert_eq!(log.stats(), Stats::default());
    }

    #[test]
    fn on_writes_one_line_per_entry_with_a_timestamp() {
        let mem = MemSink::default();
        let log = DebugLog::new(Box::new(mem.clone()), true, false);
        log.log(Entry {
            mode: Some("hold".into()),
            text: Some("hello".into()),
            ..Entry::default()
        });
        log.event("hotkeys", "hook failed");
        assert!(log.flush(Duration::from_secs(2)));
        let lines = mem.0.lock().unwrap().clone();
        assert_eq!(lines.len(), 2);
        let a: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(a["mode"], "hold");
        assert_eq!(a["text"], "hello");
        assert!(a["ts"].as_str().unwrap().ends_with('Z'));
        assert_eq!(a["kind"], "dictation");
        let b: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(b["kind"], "event");
        assert_eq!(b["event"], "hotkeys");
        assert_eq!(b["error"], "hook failed");
        assert!(b["ts"].as_str().unwrap().ends_with('Z'));
        assert_eq!(log.stats().written, 2);
    }

    #[test]
    fn write_errors_and_panics_are_swallowed_and_counted() {
        for panic in [false, true] {
            let log = DebugLog::new(Box::new(BrokenSink { panic }), true, false);
            for _ in 0..3 {
                log.log(Entry::default());
            }
            assert!(log.flush(Duration::from_secs(2)));
            assert_eq!(log.stats().write_errors, 3, "panic={panic}");
            assert_eq!(log.count(), None);
            assert!(log.clear().is_err());
            // Still accepting entries afterwards.
            log.log(Entry::default());
            assert!(log.flush(Duration::from_secs(2)));
            assert_eq!(log.stats().write_errors, 4);
        }
    }

    /// Blocks every append until released: the queue fills up.
    struct StuckSink(Arc<(Mutex<bool>, Condvar)>);

    impl Sink for StuckSink {
        fn append(&mut self, _l: &str) -> io::Result<()> {
            let (m, cv) = &*self.0;
            let g = m.lock().unwrap();
            let _g = cv.wait_while(g, |go| !*go).unwrap();
            Ok(())
        }
        fn count(&mut self) -> io::Result<usize> {
            Ok(0)
        }
        fn clear(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_stuck_writer_never_blocks_log_and_drops_the_oldest() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let log = DebugLog::new(Box::new(StuckSink(gate.clone())), true, false);
        let t = Instant::now();
        for i in 0..(QUEUE_CAP + 50) {
            log.log(Entry {
                raw: Some(i.to_string()),
                ..Entry::default()
            });
        }
        assert!(t.elapsed() < Duration::from_secs(1), "log() blocked");
        // The writer holds at most one entry; the rest overflowed.
        assert!(log.stats().dropped >= 49);
        {
            let q = lock(&log.inner.queue);
            assert!(q.len() <= QUEUE_CAP);
            let Some(Line::Dictation(last)) = q.back() else {
                panic!("not a dictation line")
            };
            assert_eq!(
                last.raw.as_deref(),
                Some((QUEUE_CAP + 49).to_string().as_str())
            );
        }
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert!(log.flush(Duration::from_secs(5)));
    }

    #[test]
    fn file_sink_keeps_the_last_lines() {
        let p = tmp("rotate");
        let mut s = FileSink::with_max(p.clone(), 5);
        for i in 0..12 {
            s.append(&format!("{{\"n\":{i}}}")).unwrap();
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "{\"n\":7}");
        assert_eq!(lines[4], "{\"n\":11}");
        assert_eq!(s.count().unwrap(), 5);
        // A new sink on the same file counts what is already there.
        let mut s2 = FileSink::with_max(p.clone(), 5);
        s2.append("{\"n\":12}").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 5);
        s2.clear().unwrap();
        assert!(!p.exists());
        assert_eq!(s2.count().unwrap(), 0);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn file_log_end_to_end_and_default_rotation_is_500() {
        let p = tmp("e2e");
        let log = DebugLog::new(Box::new(FileSink::new(p.clone())), true, false);
        for i in 0..(MAX_LINES + 3) {
            log.log(Entry::default());
            if i % 100 == 0 {
                assert!(log.flush(Duration::from_secs(10)));
            }
        }
        assert!(log.flush(Duration::from_secs(10)));
        assert_eq!(log.count(), Some(MAX_LINES));
        log.clear().unwrap();
        assert_eq!(log.count(), Some(0));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn default_path_is_under_appdata() {
        let p = default_path(Some("AD"));
        assert_eq!(
            p,
            PathBuf::from("AD")
                .join("FlowClone")
                .join("debug")
                .join("dictations.jsonl")
        );
    }

    #[test]
    fn scrub_removes_tokens_and_keys() {
        let secrets = vec!["s3cr3t-token-value".to_string()];
        let s = scrub(
            "auth_failed: token s3cr3t-token-value; header Bearer abc.def ok; key sk-abcdefghijklmnop1234, gsk_ABCDEFGHIJKLMNOPQR.",
            &secrets,
        );
        assert!(!s.contains("s3cr3t"), "{s}");
        assert!(!s.contains("abc.def"), "{s}");
        assert!(!s.contains("sk-abcdefghijklmnop1234"), "{s}");
        assert!(!s.contains("gsk_ABCDEFGHIJKLMNOPQR"), "{s}");
        assert!(s.starts_with("auth_failed: token <redacted>;"), "{s}");
        assert!(s.ends_with("<redacted>."), "{s}");
        // Ordinary text is untouched.
        assert_eq!(
            scrub("connect_failed: refused", &[]),
            "connect_failed: refused"
        );
    }

    #[test]
    fn secrets_are_scrubbed_from_every_line() {
        let mem = MemSink::default();
        let log = DebugLog::new(Box::new(mem.clone()), true, false);
        log.set_secrets(vec!["my-server-token-123".into()]);
        log.event("server", "rejected my-server-token-123");
        log.log(Entry {
            text: Some("my token is my-server-token-123".into()),
            ..Entry::default()
        });
        assert!(log.flush(Duration::from_secs(2)));
        for line in mem.0.lock().unwrap().iter() {
            assert!(!line.contains("my-server-token-123"), "{line}");
            assert!(line.contains(REDACTED), "{line}");
        }
    }
}
