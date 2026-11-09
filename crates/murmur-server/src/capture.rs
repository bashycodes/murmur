//! `[debug] capture`: opt-in, server-local copy of each finished session
//! (beta replay tool; default off). Mirrors the Android app's
//! `DebugAudioStore` / `dictations.jsonl` so a replay harness treats both
//! alike:
//!
//! * `<dir>/audio/<ts>.wav`: the session's 16 kHz mono PCM16 as received,
//!   named by the session's start time (`2026-10-27T10-15-30.123Z.wav`,
//!   the D21 `ts` with colons as dashes).
//! * `<dir>/dictations.jsonl`: one JSON line per captured session (`ts`,
//!   `audio`, `raw` transcript, `text` after AI cleanup, timings, `route`).
//!
//! The newest `max_files` WAVs are kept, and none older than `max_age_h`;
//! lines whose WAV is gone are dropped with it. Pruning runs on every save
//! and at server start. Files are mode 0600, directories 0700. The line is
//! built from an allowlist of fields: never the token, the client's
//! `Authorization` header, an API key, the context or the dictionary.
//! Nothing is uploaded.

use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use murmur_core::config::SAMPLE_RATE;
use murmur_core::CommitResult;
use serde::Deserialize;
use serde_json::{json, Value};

pub const MAX_FILES: usize = 30;
pub const MAX_AGE_H: u64 = 72;
pub const LOG_NAME: &str = "dictations.jsonl";
pub const AUDIO_DIR: &str = "audio";
const REDACTED: &str = "[redacted]";

/// `[debug]` in config.toml.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DebugConfig {
    /// Save each session's audio and result. Off by default; startup warns.
    pub capture: bool,
    pub max_files: usize,
    pub max_age_h: u64,
    /// Where to write; default `<state dir>/debug` (see [`state_dir`]).
    pub dir: Option<PathBuf>,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            capture: false,
            max_files: MAX_FILES,
            max_age_h: MAX_AGE_H,
            dir: None,
        }
    }
}

impl DebugConfig {
    /// The capture directory: `dir`, else `<state dir>/debug`.
    pub fn resolve_dir(&self) -> Result<PathBuf, String> {
        if let Some(d) = &self.dir {
            if !d.is_absolute() {
                return Err(format!("[debug] dir {} must be absolute", d.display()));
            }
            return Ok(d.clone());
        }
        state_dir()
            .map(|d| d.join("debug"))
            .ok_or_else(|| "[debug] capture: no state directory (set [debug] dir)".into())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.max_files == 0 {
            return Err("[debug] max_files must be >= 1".into());
        }
        if self.max_age_h == 0 {
            return Err("[debug] max_age_h must be >= 1".into());
        }
        if self.capture {
            self.resolve_dir()?;
        }
        Ok(())
    }
}

/// The server's state directory: systemd's `$STATE_DIRECTORY` (the unit's
/// `StateDirectory=murmur-server`, first entry), else
/// `$XDG_STATE_HOME/murmur-server`, else `$HOME/.local/state/murmur-server`;
/// the earlier install's `flow-server` dir while only it has the keys (D83).
pub fn state_dir() -> Option<PathBuf> {
    let (dir, note) = crate::legacy::state_dir(default_state_dir()?);
    if let Some(n) = note {
        crate::legacy::notice(&n);
    }
    Some(dir)
}

fn default_state_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    if let Some(s) = env("STATE_DIRECTORY") {
        let first = s
            .to_string_lossy()
            .split(':')
            .next()
            .unwrap_or("")
            .to_string();
        if !first.is_empty() {
            return Some(first.into());
        }
    }
    if let Some(x) = env("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Some(x.join("murmur-server"));
    }
    env("HOME").map(|h| PathBuf::from(h).join(".local/state/murmur-server"))
}

/// What one finished session leaves for the JSON line.
pub struct Record<'a> {
    pub sid: u64,
    pub peer: SocketAddr,
    pub result: &'a CommitResult,
    pub asr: &'a str,
    /// ASR model folder / file name.
    pub model: &'a str,
    pub queue_ms: u64,
    pub start_to_commit_ms: Option<u64>,
}

/// The capture store (one per server; writes are serialised).
pub struct CaptureStore {
    dir: PathBuf,
    max_files: usize,
    max_age: Duration,
    /// Strings that must never reach disk (the bearer token).
    secrets: Vec<String>,
    lock: Mutex<()>,
}

impl CaptureStore {
    pub fn new(dir: PathBuf, max_files: usize, max_age: Duration, secrets: Vec<String>) -> Self {
        Self {
            dir,
            max_files: max_files.max(1),
            max_age,
            secrets: secrets.into_iter().filter(|s| s.len() >= 8).collect(),
            lock: Mutex::new(()),
        }
    }

    pub fn from_config(c: &DebugConfig, token: Option<&str>) -> Result<Self, String> {
        Ok(Self::new(
            c.resolve_dir()?,
            c.max_files,
            Duration::from_secs(c.max_age_h.saturating_mul(3600)),
            token.map(str::to_string).into_iter().collect(),
        ))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn audio_dir(&self) -> PathBuf {
        self.dir.join(AUDIO_DIR)
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir.join(LOG_NAME)
    }

    /// Write the WAV and the JSON line, then prune. Returns the WAV name.
    pub fn save(&self, started: SystemTime, pcm: &[i16], rec: &Record) -> io::Result<String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let audio = self.audio_dir();
        make_private_dir(&self.dir)?;
        make_private_dir(&audio)?;
        let ts = ts(started);
        let base = ts.replace(':', "-");
        let mut name = format!("{base}.wav");
        let mut n = 2;
        while audio.join(&name).exists() {
            name = format!("{base}-{n}.wav");
            n += 1;
        }
        let tmp = audio.join(format!("{name}.tmp"));
        write_private(&tmp, &wav(pcm))?;
        fs::rename(&tmp, audio.join(&name))?;

        let mut line = self.scrub(&line(&ts, &name, pcm.len(), rec).to_string());
        line.push('\n');
        let mut o = fs::OpenOptions::new();
        o.append(true).create(true);
        private(&mut o);
        o.open(self.log_path())?.write_all(line.as_bytes())?;
        self.prune_locked(SystemTime::now())?;
        Ok(name)
    }

    /// Newest `max_files` WAVs kept, none older than `max_age` (by
    /// modification time); JSON lines whose WAV is gone are dropped.
    pub fn prune(&self, now: SystemTime) -> io::Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.prune_locked(now)
    }

    fn prune_locked(&self, now: SystemTime) -> io::Result<()> {
        let cutoff = now.checked_sub(self.max_age).unwrap_or(UNIX_EPOCH);
        let mut kept = std::collections::HashSet::new();
        for (i, (name, mtime)) in self.wavs().into_iter().enumerate() {
            if i >= self.max_files || mtime < cutoff {
                let _ = fs::remove_file(self.audio_dir().join(&name));
            } else {
                kept.insert(name);
            }
        }
        let log = self.log_path();
        let Ok(s) = fs::read_to_string(&log) else {
            return Ok(());
        };
        let keep: Vec<&str> = s
            .lines()
            .filter(|l| {
                serde_json::from_str::<Value>(l)
                    .ok()
                    .and_then(|v| v.get("audio")?.as_str().map(|a| kept.contains(a)))
                    .unwrap_or(false)
            })
            .collect();
        if keep.len() == s.lines().count() {
            return Ok(());
        }
        let mut out = keep.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        let tmp = self.dir.join(format!("{LOG_NAME}.tmp"));
        write_private(&tmp, out.as_bytes())?;
        fs::rename(tmp, log)
    }

    /// WAV names with their modification time, newest name first.
    fn wavs(&self) -> Vec<(String, SystemTime)> {
        let Ok(rd) = fs::read_dir(self.audio_dir()) else {
            return Vec::new();
        };
        let mut v: Vec<(String, SystemTime)> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| {
                let n = e.file_name().into_string().ok()?;
                n.ends_with(".wav").then_some(())?;
                Some((n, e.metadata().ok()?.modified().ok()?))
            })
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0));
        v
    }

    /// WAV names, newest first.
    pub fn list(&self) -> Vec<String> {
        self.wavs().into_iter().map(|(n, _)| n).collect()
    }

    fn scrub(&self, s: &str) -> String {
        let mut out = s.to_string();
        for x in &self.secrets {
            out = out.replace(x.as_str(), REDACTED);
        }
        out
    }
}

/// The JSON line: an allowlist of result fields, D21 names where they exist.
fn line(ts: &str, audio: &str, samples: usize, r: &Record) -> Value {
    let res = r.result;
    let t = &res.timings;
    json!({
        "ts": ts,
        "kind": "server_session",
        "build": murmur_build_info::FULL,
        "sid": r.sid,
        "peer": r.peer.ip().to_string(),
        "route": "server",
        "engine": r.asr,
        "model": r.model,
        "audio": audio,
        "recv_audio_ms": (samples as u64) * 1000 / SAMPLE_RATE as u64,
        "status": res.status,
        "empty_reason": res.empty_reason,
        "fallback_reason": res.fallback_reason,
        "cleanup_route": res.cleanup_route,
        "cloud_fallback": res.cloud_fallback,
        "llm_engine": res.llm.as_ref().and_then(|l| l.engine.clone()),
        "llm_model": res.llm.as_ref().and_then(|l| l.model.clone()),
        "language": res.language,
        "truncated": res.truncated,
        "press_enter": res.press_enter,
        "error": res.error,
        "audio_ms": t.audio_ms,
        "asr_ms": t.asr_ms,
        "asr_audio_ms": t.asr_audio_ms,
        "llm_ms": t.llm_ms,
        "post_ms": t.llm_ms + t.postprocess_ms,
        "total_ms": t.total_ms,
        "wait_ms": t.wait_ms,
        "segments": t.segments,
        "pre_asr_ms": t.pre_asr_ms,
        "speech_ms": t.speech_ms,
        "peak_dbfs": t.peak_dbfs,
        "dropped_samples": t.dropped_samples,
        "queue_ms": r.queue_ms,
        "start_to_commit_ms": r.start_to_commit_ms,
        "raw": res.asr_text,
        "text": res.final_text,
    })
}

/// D21 `ts`: UTC `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn ts(t: SystemTime) -> String {
    let ms = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s, milli) = (
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000,
    );
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// 16 kHz mono PCM16, canonical 44-byte header (same as Android `Wav`).
pub fn wav(pcm: &[i16]) -> Vec<u8> {
    let rate = SAMPLE_RATE as u32;
    let data = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[cfg_attr(not(unix), allow(unused_variables))]
fn private(o: &mut fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
}

fn write_private(p: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    private(&mut o);
    let mut f = o.open(p)?;
    f.write_all(bytes)?;
    set_mode(p, 0o600)
}

fn make_private_dir(p: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(p)?;
    set_mode(p, 0o700)
}

fn set_mode(p: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (p, mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str) -> CommitResult {
        serde_json::from_value(json!({
            "status": "FORMATTED", "asr_text": text, "final_text": format!("{text}."),
            "language": "en",
            "timings": serde_json::to_value(murmur_core::Timings::default()).unwrap(),
            "press_enter": false, "fallback_reason": null,
            "truncated": false, "error": null
        }))
        .unwrap()
    }

    fn rec(r: &CommitResult) -> Record<'_> {
        Record {
            sid: 1,
            peer: "100.64.0.2:5000".parse().unwrap(),
            result: r,
            asr: "mock",
            model: "mock",
            queue_ms: 0,
            start_to_commit_ms: Some(900),
        }
    }

    fn store(d: &Path, n: usize, age_h: u64) -> CaptureStore {
        CaptureStore::new(
            d.to_path_buf(),
            n,
            Duration::from_secs(age_h * 3600),
            vec!["secret-token-0123456789".into()],
        )
    }

    fn at(ms: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(ms)
    }

    fn lines(s: &CaptureStore) -> Vec<Value> {
        fs::read_to_string(s.log_path())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn ts_matches_the_android_format() {
        assert_eq!(ts(at(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(ts(at(1_793_096_130_123)), "2026-10-27T10:15:30.123Z");
    }

    #[test]
    fn wav_header_is_canonical() {
        let w = wav(&[1, -1, 300]);
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 6);
        assert_eq!(&w[44..46], &1i16.to_le_bytes());
    }

    #[test]
    fn save_writes_wav_and_line_with_private_modes() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("debug");
        let s = store(&dir, 30, 72);
        let r = result("hello world");
        let name = s
            .save(at(1_793_096_130_123), &[0i16; 1600], &rec(&r))
            .unwrap();
        assert_eq!(name, "2026-10-27T10-15-30.123Z.wav");
        assert_eq!(
            fs::read(s.audio_dir().join(&name)).unwrap().len(),
            44 + 3200
        );
        let l = lines(&s);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0]["audio"], name.as_str());
        assert_eq!(l[0]["ts"], "2026-10-27T10:15:30.123Z");
        assert_eq!(l[0]["raw"], "hello world");
        assert_eq!(l[0]["text"], "hello world.");
        assert_eq!(l[0]["route"], "server");
        assert_eq!(l[0]["recv_audio_ms"], 100);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&s.audio_dir()), 0o700);
            assert_eq!(mode(&s.log_path()), 0o600);
            assert_eq!(mode(&s.audio_dir().join(&name)), 0o600);
        }
    }

    #[test]
    fn same_millisecond_does_not_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), 30, 72);
        let r = result("a");
        let a = s.save(at(5), &[1], &rec(&r)).unwrap();
        let b = s.save(at(5), &[2], &rec(&r)).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn prunes_by_count_newest_kept() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), 3, 72);
        let r = result("x");
        for i in 0..5u64 {
            s.save(at(1_793_096_130_000 + i * 1000), &[0; 16], &rec(&r))
                .unwrap();
        }
        let names = s.list();
        assert_eq!(names.len(), 3);
        assert!(names[0].starts_with("2026-10-27T10-15-34"), "{names:?}");
        assert!(names[2].starts_with("2026-10-27T10-15-32"), "{names:?}");
        let l = lines(&s);
        assert_eq!(l.len(), 3, "lines follow their WAVs");
        assert!(l
            .iter()
            .all(|v| names.contains(&v["audio"].as_str().unwrap().to_string())));
    }

    #[test]
    fn prunes_by_age_on_save_and_at_start() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), 30, 72);
        let r = result("x");
        let old = s.save(at(1_000), &[0; 16], &rec(&r)).unwrap();
        let new = s.save(at(2_000), &[0; 16], &rec(&r)).unwrap();
        let f = fs::File::options()
            .write(true)
            .open(s.audio_dir().join(&old))
            .unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(73 * 3600))
            .unwrap();
        drop(f);
        // "At start": a fresh store over the same directory.
        store(d.path(), 30, 72).prune(SystemTime::now()).unwrap();
        assert_eq!(s.list(), vec![new.clone()]);
        assert_eq!(lines(&s).len(), 1);
        // 71 h old survives.
        let f = fs::File::options()
            .write(true)
            .open(s.audio_dir().join(&new))
            .unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(71 * 3600))
            .unwrap();
        s.prune(SystemTime::now()).unwrap();
        assert_eq!(s.list(), vec![new]);
    }

    #[test]
    fn line_never_carries_the_token() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), 30, 72);
        // Even a transcript that contains the token is scrubbed.
        let r = result("my token is secret-token-0123456789");
        s.save(at(9), &[0; 16], &rec(&r)).unwrap();
        let log = fs::read_to_string(s.log_path()).unwrap();
        assert!(!log.contains("secret-token-0123456789"), "{log}");
        assert!(log.contains(REDACTED), "{log}");
    }

    #[test]
    fn config_defaults_and_validation() {
        let c = DebugConfig::default();
        assert!(!c.capture);
        assert_eq!((c.max_files, c.max_age_h), (30, 72));
        assert!(c.validate().is_ok());
        let bad = DebugConfig {
            max_files: 0,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let bad = DebugConfig {
            max_age_h: 0,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let rel = DebugConfig {
            capture: true,
            dir: Some("debug".into()),
            ..Default::default()
        };
        assert!(rel.validate().unwrap_err().contains("absolute"));
    }
}
