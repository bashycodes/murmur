//! Meetings on disk, next to the history: `%APPDATA%\FlowClone\meetings\`
//! holds one `<id>.json` per meeting (times, app, status, transcript) and,
//! for the [`KEEP_AUDIO`] most recent meetings only, its two recordings
//! `<id>.mic.wav` / `<id>.system.wav`. Older audio is deleted
//! ([`MeetingStore::prune_audio`]); transcripts stay until the user deletes
//! them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::record::Source;
use super::transcript::Turn;

/// Folder of the meetings under `%APPDATA%\FlowClone`.
pub const MEETINGS_DIR: &str = "meetings";
/// Meetings whose audio is kept (debugging); older ones keep only text.
pub const KEEP_AUDIO: usize = 3;

/// `%APPDATA%\FlowClone\meetings`.
pub fn default_dir(appdata: Option<&str>) -> PathBuf {
    crate::notes::app_dir(appdata).join(MEETINGS_DIR)
}

/// Ids become file names: lowercase ASCII letters, digits and `-`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Being recorded now (or the app stopped mid-recording).
    #[default]
    Recording,
    /// Recorded; waiting for or in transcription.
    Transcribing,
    Done,
    /// Transcription failed (`error` says why); the audio is kept.
    Failed,
}

/// Version of [`MeetingSummary`] this build writes (`v`).
pub const SUMMARY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryStatus {
    /// Waiting for, or being made by, a summary job (also after a quit
    /// mid-job: it is made at the next start).
    #[default]
    Pending,
    Done,
    /// `error` says why; an earlier summary's text, if any, is kept.
    Failed,
}

/// A meeting's AI summary and action items (D53), made from its
/// transcript by the AI cleanup order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeetingSummary {
    /// Format version ([`SUMMARY_VERSION`]); 0 = written before versions.
    pub v: u32,
    pub status: SummaryStatus,
    pub text: String,
    pub action_items: Vec<String>,
    /// The language asked for (`"he"`, `"en"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Who wrote it: `local` (this PC), `server`, `cloud` (OpenRouter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Wall clock (ms since the Unix epoch) when it was last finished.
    pub made_ms: u64,
}

impl MeetingSummary {
    /// Queued for (re)generation; an earlier summary's text stays shown
    /// until the new one replaces it.
    pub fn pending(prev: Option<&MeetingSummary>) -> MeetingSummary {
        MeetingSummary {
            v: SUMMARY_VERSION,
            status: SummaryStatus::Pending,
            error: None,
            ..prev.cloned().unwrap_or_default()
        }
    }
}

/// One meeting (`<id>.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meeting {
    pub id: String,
    /// Wall clock, ms since the Unix epoch.
    pub started_ms: u64,
    pub duration_ms: u64,
    /// Exe name of the app in front when the recording started, when it
    /// was not this app or the desktop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// No sound came from the PC during the meeting (headphones on another
    /// device, or a muted call): the transcript has only "Me".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub system_silent: bool,
    pub turns: Vec<Turn>,
    /// Which route transcribed the meeting's chunks, as chunk counts by
    /// route (`local` = this PC, `remote` = the user's server, `cloud` =
    /// OpenRouter; `none` = a chunk no route served). Set by the last
    /// finished transcription job; empty before one.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub routes: std::collections::BTreeMap<String, u32>,
    /// D53: the AI summary; absent in files written before it, and until
    /// the first summary job of this meeting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<MeetingSummary>,
}

/// A row of the Meetings list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingMeta {
    pub id: String,
    pub started_ms: u64,
    pub duration_ms: u64,
    pub app: Option<String>,
    pub status: Status,
    pub error: Option<String>,
    pub turns: usize,
    /// The first turn's text (cut).
    pub preview: String,
    pub has_audio: bool,
}

const PREVIEW_MAX: usize = 120;

fn preview(m: &Meeting) -> String {
    let t = m
        .turns
        .iter()
        .find(|t| !t.text.trim().is_empty())
        .map(|t| t.text.trim())
        .unwrap_or("");
    if t.chars().count() > PREVIEW_MAX {
        let cut: String = t.chars().take(PREVIEW_MAX - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        t.to_string()
    }
}

pub struct MeetingStore {
    dir: PathBuf,
}

impl MeetingStore {
    pub fn new(dir: PathBuf) -> Self {
        MeetingStore { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn json(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// The recording of `src` for meeting `id`.
    pub fn audio_path(&self, id: &str, src: Source) -> PathBuf {
        self.dir.join(format!("{id}.{}.wav", src.file_tag()))
    }

    pub fn audio_paths(&self, id: &str) -> [PathBuf; 2] {
        [
            self.audio_path(id, Source::Mic),
            self.audio_path(id, Source::System),
        ]
    }

    pub fn has_audio(&self, id: &str) -> bool {
        self.audio_paths(id).iter().any(|p| p.is_file())
    }

    /// A fresh id for a meeting started at `started_ms` (unique in the
    /// folder).
    pub fn new_id(&self, started_ms: u64) -> String {
        let base = format!("m{started_ms}");
        let mut id = base.clone();
        let mut n = 1;
        while self.json(&id).exists() || self.has_audio(&id) {
            n += 1;
            id = format!("{base}-{n}");
        }
        id
    }

    /// Write `m` (temp file + rename, so a crash never leaves half a file).
    pub fn save(&self, m: &Meeting) -> Result<(), String> {
        if !valid_id(&m.id) {
            return Err(format!("bad meeting id {:?}", m.id));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let path = self.json(&m.id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, id: &str) -> Option<Meeting> {
        if !valid_id(id) {
            return None;
        }
        let body = std::fs::read(self.json(id)).ok()?;
        let mut m: Meeting = serde_json::from_slice(&body).ok()?;
        m.id = id.to_string();
        Some(m)
    }

    /// Every meeting, newest first.
    pub fn all(&self) -> Vec<Meeting> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut v: Vec<Meeting> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let id = name.strip_suffix(".json")?.to_string();
                self.get(&id)
            })
            .collect();
        v.sort_by(|a, b| b.started_ms.cmp(&a.started_ms).then(b.id.cmp(&a.id)));
        v
    }

    /// The Meetings list, newest first.
    pub fn list(&self) -> Vec<MeetingMeta> {
        self.all()
            .into_iter()
            .map(|m| MeetingMeta {
                has_audio: self.has_audio(&m.id),
                preview: preview(&m),
                turns: m.turns.len(),
                id: m.id,
                started_ms: m.started_ms,
                duration_ms: m.duration_ms,
                app: m.app,
                status: m.status,
                error: m.error,
            })
            .collect()
    }

    /// Delete a meeting: its transcript and its audio. `Ok(false)` = there
    /// was nothing to delete.
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        if !valid_id(id) {
            return Err(format!("bad meeting id {id:?}"));
        }
        let mut any = false;
        let mut paths = vec![self.json(id)];
        paths.extend(self.audio_paths(id));
        for p in paths {
            match std::fs::remove_file(&p) {
                Ok(()) => any = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", p.display())),
            }
        }
        Ok(any)
    }

    /// Keep the audio of the `keep` most recent meetings and delete the
    /// rest, plus any recording whose meeting is gone. A meeting still
    /// being recorded or transcribed always keeps its audio (it needs it)
    /// and counts towards `keep`. Returns the files deleted.
    pub fn prune_audio(&self, keep: usize) -> Vec<PathBuf> {
        self.prune_listed(&self.all(), keep)
    }

    /// [`Self::prune_audio`] given the meetings listed at its start. A
    /// recording can begin after that listing (`Meetings::start` writes
    /// the meeting file, then its WAVs), so an orphan is a WAV whose
    /// meeting file is missing *now*, not one missing from `listed`;
    /// otherwise the newest recording could be deleted as an orphan.
    fn prune_listed(&self, listed: &[Meeting], keep: usize) -> Vec<PathBuf> {
        let mut deleted = Vec::new();
        for (i, m) in listed.iter().enumerate() {
            let busy = matches!(m.status, Status::Recording | Status::Transcribing);
            if i < keep || busy {
                continue;
            }
            for p in self.audio_paths(&m.id) {
                if std::fs::remove_file(&p).is_ok() {
                    deleted.push(p);
                }
            }
        }
        // Orphans: recordings without a meeting file. A meeting file that
        // exists but doesn't parse keeps its WAVs: never delete audio we
        // can't attribute.
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(stem) = name.strip_suffix(".wav") else {
                    continue;
                };
                let id = stem.rsplit_once('.').map_or(stem, |(id, _)| id);
                if !self.json(id).exists() && std::fs::remove_file(e.path()).is_ok() {
                    deleted.push(e.path());
                }
            }
        }
        deleted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::transcript::Speaker;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "flow-meet-store-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn meeting(s: &MeetingStore, started: u64, status: Status) -> Meeting {
        let m = Meeting {
            id: s.new_id(started),
            started_ms: started,
            duration_ms: 60_000,
            status,
            turns: vec![Turn {
                speaker: Speaker::Me,
                start_ms: 0,
                end_ms: 1_000,
                text: format!("meeting at {started}"),
                error: None,
            }],
            ..Meeting::default()
        };
        s.save(&m).unwrap();
        for p in s.audio_paths(&m.id) {
            std::fs::write(p, b"RIFF").unwrap();
        }
        m
    }

    #[test]
    fn save_list_get_delete() {
        let d = tmp("crud");
        let s = MeetingStore::new(d.clone());
        assert!(s.list().is_empty(), "no folder yet");
        let a = meeting(&s, 1_000, Status::Done);
        let mut b = meeting(&s, 2_000, Status::Done);
        b.app = Some("zoom.exe".into());
        s.save(&b).unwrap();
        let l = s.list();
        assert_eq!(
            l.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            [&b.id, &a.id]
        );
        assert_eq!(l[0].app.as_deref(), Some("zoom.exe"));
        assert_eq!(l[0].preview, "meeting at 2000");
        assert!(l[0].has_audio);
        assert_eq!(s.get(&b.id).unwrap(), b);
        assert_eq!(s.delete(&a.id), Ok(true));
        assert!(!s.has_audio(&a.id));
        assert_eq!(s.delete(&a.id), Ok(false));
        assert_eq!(s.list().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn files_without_or_with_a_summary_load() {
        let d = tmp("summary");
        let s = MeetingStore::new(d.clone());
        std::fs::create_dir_all(&d).unwrap();
        // Written by 1.1.1: no summary field.
        let old = r#"{"id":"m9","started_ms":9,"duration_ms":1000,"status":"done",
            "turns":[{"speaker":"me","start_ms":0,"end_ms":900,"text":"made up line"}]}"#;
        std::fs::write(d.join("m9.json"), old).unwrap();
        let m = s.get("m9").unwrap();
        assert_eq!((m.status, m.summary.as_ref()), (Status::Done, None));
        assert!(!serde_json::to_string(&m).unwrap().contains("summary"));
        // With one, and a field from a later version that this build skips.
        let new = r#"{"id":"m10","started_ms":10,"status":"done","turns":[],
            "summary":{"v":2,"status":"done","text":"A short sync.","action_items":["Me: book it"],
            "engine":"server","made_ms":5,"later_field":true}}"#;
        std::fs::write(d.join("m10.json"), new).unwrap();
        let sm = s.get("m10").unwrap().summary.unwrap();
        assert_eq!(sm.status, SummaryStatus::Done);
        assert_eq!(sm.action_items, ["Me: book it"]);
        assert_eq!(sm.engine.as_deref(), Some("server"));
        // Regenerate keeps the old text while pending.
        let p = MeetingSummary::pending(Some(&sm));
        assert_eq!(
            (p.status, p.text.as_str(), p.v),
            (SummaryStatus::Pending, "A short sync.", 1)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ids_are_unique_and_safe() {
        let d = tmp("ids");
        let s = MeetingStore::new(d.clone());
        let a = meeting(&s, 5, Status::Done);
        assert_eq!(a.id, "m5");
        assert_eq!(s.new_id(5), "m5-2");
        assert!(s.get("../prefs").is_none());
        assert!(s.delete("..\\x").is_err());
        assert!(!valid_id("M5") && !valid_id("") && valid_id("m5-2"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn exactly_the_last_three_recordings_are_kept() {
        let d = tmp("prune");
        let s = MeetingStore::new(d.clone());
        for i in 1..=6 {
            meeting(&s, i * 1_000, Status::Done);
        }
        std::fs::write(d.join("m77.mic.wav"), b"orphan").unwrap();
        let deleted = s.prune_audio(KEEP_AUDIO);
        assert_eq!(deleted.len(), 3 * 2 + 1);
        let with_audio: Vec<String> = s
            .list()
            .into_iter()
            .filter(|m| m.has_audio)
            .map(|m| m.id)
            .collect();
        assert_eq!(with_audio, ["m6000", "m5000", "m4000"]);
        assert_eq!(s.list().len(), 6, "transcripts stay");
        assert!(!d.join("m77.mic.wav").exists());
        // A fourth recording pushes the oldest kept one out.
        meeting(&s, 7_000, Status::Done);
        s.prune_audio(KEEP_AUDIO);
        let n = s.list().iter().filter(|m| m.has_audio).count();
        assert_eq!(n, 3);
        assert!(!s.has_audio("m4000"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_recording_started_during_a_prune_is_not_an_orphan() {
        let d = tmp("race");
        let s = MeetingStore::new(d.clone());
        for i in 1..=4 {
            meeting(&s, i * 1_000, Status::Done);
        }
        // The prune lists the meetings, then a new recording writes its
        // meeting file and WAVs before the prune looks for orphans.
        let listed = s.all();
        meeting(&s, 5_000, Status::Recording);
        std::fs::write(d.join("m77.mic.wav"), b"orphan").unwrap();
        s.prune_listed(&listed, KEEP_AUDIO);
        assert!(s.has_audio("m5000"), "the new recording keeps its audio");
        assert!(!s.has_audio("m1000"));
        assert!(!d.join("m77.mic.wav").exists(), "real orphans still go");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_meeting_still_in_work_keeps_its_audio() {
        let d = tmp("busy");
        let s = MeetingStore::new(d.clone());
        meeting(&s, 1_000, Status::Transcribing);
        for i in 2..=5 {
            meeting(&s, i * 1_000, Status::Done);
        }
        s.prune_audio(KEEP_AUDIO);
        assert!(s.has_audio("m1000"), "still needed");
        assert!(!s.has_audio("m2000"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
