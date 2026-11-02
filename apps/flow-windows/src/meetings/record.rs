//! The two-stream meeting recorder: the microphone ("Me") and the system
//! audio ("Others", WASAPI loopback) written as two 16 kHz mono WAVs of
//! the same length, on one clock.
//!
//! The capture callbacks only stamp each block with its arrival time and
//! hand it to a writer thread, so they stay cheap. The writer places every
//! block on the recording clock with an [`Aligner`]:
//!
//! * a stream that starts late (the loopback device opens after the mic)
//!   gets silence in front;
//! * a gap in a stream gets silence. WASAPI loopback delivers **no** blocks
//!   while nothing plays, so without this every pause on the call would
//!   pull the "Others" track earlier than the mic;
//! * on stop both files are padded to the same end.
//!
//! A block is never dropped and never moved earlier: small jitter (under
//! [`GAP_TOLERANCE_MS`]) is absorbed, so a steady stream is written as is.
//! The headers are rewritten every few seconds ([`FLUSH_EVERY`]), so a
//! crash leaves valid WAVs with all but the last seconds.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};

/// Sample rate of both files.
pub const RATE: u64 = 16_000;
/// A block that arrives this much later than its stream's position is
/// placed after a gap of silence; less is jitter and is ignored.
pub const GAP_TOLERANCE_MS: u64 = 200;
/// Rewrite the WAV headers after this many samples per stream (5 s).
pub const FLUSH_EVERY: u64 = 5 * RATE;

/// Milliseconds on a monotonic clock (the recorder only uses differences).
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// A capture callback's sink: 16 kHz mono blocks, in order.
pub type Sink = Box<dyn FnMut(&[i16]) + Send + 'static>;

/// Which stream: the microphone, or what the PC plays (loopback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Mic,
    System,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::Mic, Source::System];

    /// File-name part (`<id>.mic.wav`, `<id>.system.wav`).
    pub fn file_tag(self) -> &'static str {
        match self {
            Source::Mic => "mic",
            Source::System => "system",
        }
    }
}

/// Places the blocks of one stream on the recording clock (module docs).
#[derive(Debug, Clone, Default)]
pub struct Aligner {
    written: u64,
}

impl Aligner {
    /// A block of `len` samples whose last sample arrived `at_ms` after the
    /// recording started: the number of silent samples to write before it.
    pub fn place(&mut self, at_ms: u64, len: usize) -> u64 {
        let end = at_ms * RATE / 1000;
        let start = end.saturating_sub(len as u64);
        let tolerance = GAP_TOLERANCE_MS * RATE / 1000;
        let pad = if start > self.written + tolerance {
            start - self.written
        } else {
            0
        };
        self.written += pad + len as u64;
        pad
    }

    /// Silent samples that bring the stream to `total`.
    pub fn pad_to(&mut self, total: u64) -> u64 {
        let pad = total.saturating_sub(self.written);
        self.written += pad;
        pad
    }

    /// Samples placed so far.
    pub fn written(&self) -> u64 {
        self.written
    }
}

/// What a finished recording holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// Samples per file (both files have this many).
    pub samples: u64,
    pub duration_ms: u64,
    /// Samples that came from each device (the rest is padding): 0 for the
    /// system audio = nothing played during the meeting.
    pub captured: [u64; 2],
}

enum Msg {
    Block(Source, u64, Vec<i16>),
    Stop(u64),
}

type Writer = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

fn spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

struct Track {
    writer: Writer,
    align: Aligner,
    captured: u64,
    since_flush: u64,
}

impl Track {
    fn create(path: &Path) -> Result<Track, String> {
        let writer = hound::WavWriter::create(path, spec())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Track {
            writer,
            align: Aligner::default(),
            captured: 0,
            since_flush: 0,
        })
    }

    fn zeros(&mut self, n: u64) -> Result<(), String> {
        for _ in 0..n {
            self.writer.write_sample(0i16).map_err(|e| e.to_string())?;
        }
        self.since_flush += n;
        Ok(())
    }

    fn block(&mut self, at_ms: u64, pcm: &[i16]) -> Result<(), String> {
        let pad = self.align.place(at_ms, pcm.len());
        self.zeros(pad)?;
        for &s in pcm {
            self.writer.write_sample(s).map_err(|e| e.to_string())?;
        }
        self.captured += pcm.len() as u64;
        self.since_flush += pcm.len() as u64;
        if self.since_flush >= FLUSH_EVERY {
            self.since_flush = 0;
            self.writer.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn write_loop(rx: Receiver<Msg>, mut tracks: [Track; 2]) -> Result<Recorded, String> {
    let idx = |s: Source| match s {
        Source::Mic => 0,
        Source::System => 1,
    };
    let mut end_ms = None;
    let mut err = None;
    for m in rx.iter() {
        match m {
            Msg::Block(src, at, pcm) => {
                if err.is_none() {
                    err = tracks[idx(src)].block(at, &pcm).err();
                }
            }
            Msg::Stop(at) => {
                end_ms = Some(at);
                break;
            }
        }
    }
    if let Some(e) = err {
        return Err(e);
    }
    let end = end_ms.unwrap_or(0) * RATE / 1000;
    let total = tracks
        .iter()
        .map(|t| t.align.written())
        .max()
        .unwrap_or(0)
        .max(end);
    let captured = [tracks[0].captured, tracks[1].captured];
    for mut t in tracks {
        let pad = t.align.pad_to(total);
        t.zeros(pad)?;
        t.writer.finalize().map_err(|e| e.to_string())?;
    }
    Ok(Recorded {
        samples: total,
        duration_ms: total * 1000 / RATE,
        captured,
    })
}

/// One recording in progress: two WAVs, one writer thread.
pub struct Recorder {
    tx: Sender<Msg>,
    join: Option<JoinHandle<Result<Recorded, String>>>,
    clock: Clock,
    start_ms: u64,
}

impl Recorder {
    /// Create both files (`paths` = mic, system) and start the writer.
    /// Time 0 of the recording is `clock()` now.
    pub fn start(paths: [PathBuf; 2], clock: Clock) -> Result<Recorder, String> {
        if let Some(dir) = paths[0].parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tracks = [Track::create(&paths[0])?, Track::create(&paths[1])?];
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("flow-meeting-writer".into())
            .spawn(move || write_loop(rx, tracks))
            .map_err(|e| e.to_string())?;
        let start_ms = clock();
        Ok(Recorder {
            tx,
            join: Some(join),
            clock,
            start_ms,
        })
    }

    /// Milliseconds since the recording started.
    pub fn elapsed_ms(&self) -> u64 {
        (self.clock)().saturating_sub(self.start_ms)
    }

    /// A block of `src` that arrived `at_ms` after the start (its last
    /// sample). The capture sinks call this; tests call it directly.
    pub fn feed(&self, src: Source, at_ms: u64, pcm: &[i16]) {
        let _ = self.tx.send(Msg::Block(src, at_ms, pcm.to_vec()));
    }

    /// A capture sink for `src`: stamps every block with the clock.
    pub fn sink(&self, src: Source) -> Sink {
        let tx = self.tx.clone();
        let clock = self.clock.clone();
        let start = self.start_ms;
        Box::new(move |pcm: &[i16]| {
            if pcm.is_empty() {
                return;
            }
            let at = clock().saturating_sub(start);
            let _ = tx.send(Msg::Block(src, at, pcm.to_vec()));
        })
    }

    /// End the recording at `at_ms` (both files padded to it, or to the
    /// longer stream) and close the files. Stop the captures first.
    pub fn stop_at(mut self, at_ms: u64) -> Result<Recorded, String> {
        let _ = self.tx.send(Msg::Stop(at_ms));
        match self.join.take().map(|j| j.join()) {
            Some(Ok(r)) => r,
            Some(Err(_)) => Err("the meeting writer thread panicked".into()),
            None => Err("already stopped".into()),
        }
    }

    /// End the recording now.
    pub fn stop(self) -> Result<Recorded, String> {
        let at = self.elapsed_ms();
        self.stop_at(at)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(j) = self.join.take() {
            let _ = self.tx.send(Msg::Stop(0));
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "flow-meet-rec-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read(p: &Path) -> Vec<i16> {
        hound::WavReader::open(p)
            .unwrap()
            .samples::<i16>()
            .map(|s| s.unwrap())
            .collect()
    }

    fn fixed_clock() -> Clock {
        Arc::new(|| 0)
    }

    #[test]
    fn aligner_writes_a_steady_stream_as_is() {
        let mut a = Aligner::default();
        // 10 ms blocks arriving with up to 30 ms of jitter.
        for i in 1..=100u64 {
            let jitter = (i * 7) % 30;
            assert_eq!(a.place(i * 10 + jitter, 160), 0, "block {i}");
        }
        assert_eq!(a.written(), 16_000);
    }

    #[test]
    fn aligner_fills_a_late_start_and_a_gap_with_silence() {
        let mut a = Aligner::default();
        // First block's last sample at 1.01 s: it covers 1.00-1.01 s.
        assert_eq!(a.place(1010, 160), 16_000);
        assert_eq!(a.place(1020, 160), 0);
        // Nothing for 3 s (loopback while nothing plays).
        assert_eq!(a.place(4030, 160), 4020 * 16 - 1020 * 16);
        assert_eq!(a.written(), 4030 * 16);
        assert_eq!(a.pad_to(5000 * 16), 970 * 16);
        assert_eq!(a.pad_to(10), 0, "never shortens");
    }

    /// The acceptance fixture: two sources that start at different times,
    /// arrive in differently sized blocks with jitter, and the system one
    /// goes silent (no blocks) in the middle. A click played at known
    /// times on both must land on the same sample in both files.
    #[test]
    fn two_sources_end_up_in_sync() {
        let d = tmp("sync");
        let paths = [d.join("a.mic.wav"), d.join("a.system.wav")];
        let rec = Recorder::start(paths.clone(), fixed_clock()).unwrap();
        // 20 s at 16 kHz; clicks at 2 s, 9 s and 17 s on both sources.
        let clicks_ms = [2_000u64, 9_000, 17_000];
        let signal = |src_ms: u64| -> i16 {
            if clicks_ms.iter().any(|&c| src_ms >= c && src_ms < c + 5) {
                20_000
            } else {
                0
            }
        };
        let total_ms = 20_000u64;
        // Mic: starts at 0, 10 ms blocks, 0-15 ms late.
        // System: opens 350 ms late, 7 ms blocks, 0-40 ms late, and sends
        // nothing from 5 s to 8.5 s.
        let mut events: Vec<(u64, Source, Vec<i16>)> = Vec::new();
        // A device delivers its blocks in order: arrival never goes back.
        let (mut t, mut last) = (0, 0);
        while t < total_ms {
            let block: Vec<i16> = (0..160).map(|i| signal(t + i / 16)).collect();
            last = (t + 10 + (t / 10 * 13) % 16).max(last);
            events.push((last, Source::Mic, block));
            t += 10;
        }
        let (mut t, mut last) = (350, 0);
        while t < total_ms - 7 {
            if !(5_000..8_500).contains(&t) {
                let block: Vec<i16> = (0..112).map(|i| signal(t + i / 16)).collect();
                last = (t + 7 + (t * 31) % 41).max(last);
                events.push((last, Source::System, block));
            }
            t += 7;
        }
        // Interleave the two sources by arrival (stable: in order per source).
        events.sort_by_key(|e| e.0);
        for (at, src, pcm) in &events {
            rec.feed(*src, *at, pcm);
        }
        let r = rec.stop_at(total_ms).unwrap();
        let mic = read(&paths[0]);
        let sys = read(&paths[1]);
        assert_eq!(mic.len(), sys.len(), "same length");
        assert_eq!(mic.len() as u64, r.samples);
        assert!(r.samples >= total_ms * 16);
        assert!(r.captured[1] < r.captured[0], "system had a gap");
        let onsets = |x: &[i16]| -> Vec<usize> {
            (0..x.len())
                .filter(|&i| x[i] != 0 && (i == 0 || x[i - 1] == 0))
                .collect()
        };
        let (a, b) = (onsets(&mic), onsets(&sys));
        assert_eq!(a.len(), 3, "{a:?}");
        assert_eq!(b.len(), 3, "{b:?}");
        for (i, c) in clicks_ms.iter().enumerate() {
            let want = (*c * 16) as i64;
            // Within the jitter tolerance of the true time, and of each other.
            assert!(
                (a[i] as i64 - want).abs() <= 200 * 16,
                "mic click {i}: {} vs {want}",
                a[i]
            );
            assert!(
                (b[i] as i64 - want).abs() <= 200 * 16,
                "sys click {i}: {} vs {want}",
                b[i]
            );
            assert!(
                (a[i] as i64 - b[i] as i64).abs() <= 200 * 16,
                "click {i}: mic {} sys {}",
                a[i],
                b[i]
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sinks_stamp_blocks_with_the_clock() {
        let d = tmp("sink");
        let now = Arc::new(std::sync::atomic::AtomicU64::new(1_000));
        let n = now.clone();
        let clock: Clock = Arc::new(move || n.load(std::sync::atomic::Ordering::SeqCst));
        let paths = [d.join("b.mic.wav"), d.join("b.system.wav")];
        let rec = Recorder::start(paths.clone(), clock).unwrap();
        let mut mic = rec.sink(Source::Mic);
        let mut sys = rec.sink(Source::System);
        now.store(1_100, std::sync::atomic::Ordering::SeqCst);
        mic(&[7; 1600]);
        // System audio first plays 2 s in.
        now.store(3_000, std::sync::atomic::Ordering::SeqCst);
        sys(&[9; 160]);
        now.store(4_000, std::sync::atomic::Ordering::SeqCst);
        let r = rec.stop().unwrap();
        assert_eq!(r.duration_ms, 3_000);
        let s = read(&paths[1]);
        let first = s.iter().position(|&x| x == 9).unwrap();
        assert_eq!(first, 2_000 * 16 - 160);
        assert_eq!(read(&paths[0]).len(), s.len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn headers_are_valid_while_recording() {
        let d = tmp("flush");
        let paths = [d.join("c.mic.wav"), d.join("c.system.wav")];
        let rec = Recorder::start(paths.clone(), fixed_clock()).unwrap();
        for i in 0..70u64 {
            rec.feed(Source::Mic, (i + 1) * 100, &[1; 1600]);
        }
        // Wait for the writer to pass the first flush.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Ok(r) = hound::WavReader::open(&paths[0]) {
                if r.duration() as u64 >= FLUSH_EVERY {
                    break;
                }
            }
            assert!(std::time::Instant::now() < deadline, "no flush");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(rec);
        let _ = std::fs::remove_dir_all(&d);
    }
}
