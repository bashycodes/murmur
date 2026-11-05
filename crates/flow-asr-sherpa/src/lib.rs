//! Parakeet-TDT ASR through sherpa-onnx (official `sherpa-onnx` crate).
//!
//! One warm [`ParakeetEngine`] holds the onnxruntime sessions for the
//! encoder, decoder and joiner of a NeMo TDT transducer
//! (`sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`). Session creation is the
//! expensive part (seconds, see `docs/model-benchmarks.md`), so callers load
//! once and keep the engine resident; a decode is RTF ≈ 0.14 on a 4-core CPU.
//!
//! Contextual biasing: sherpa-onnx's modified beam search accepts per-stream
//! hotwords for NeMo transducers, TDT included. The hotwords are
//! sentencepiece-encoded with a `bpe.vocab` that this crate derives from the
//! model's `tokens.txt` (see [`bpe_vocab_from_tokens`]).
//!
//! [`OnlineEngine`] does the same for streaming (online) transducers such
//! as ivrit-ai's zipformer2 Hebrew model; both implement
//! [`SherpaTranscriber`].
//!
//! The crate does not depend on flow-core; flow-core's `parakeet` module
//! adapts it to the `AsrEngine` trait.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

mod online;
pub use online::{OnlineConfig, OnlineEngine, DEFAULT_TAIL_PADDING_S};

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};

/// The model's native rate; callers pass 16 kHz mono f32 in [-1, 1].
pub const SAMPLE_RATE: i32 = 16_000;
/// Inputs shorter than this are padded with silence (the FastConformer
/// subsampling needs a few frames; 1 s matches the whisper backend).
pub const MIN_SAMPLES: usize = SAMPLE_RATE as usize;
/// Per-token bonus for hotwords. Measured (docs/core-architecture.md,
/// "Parakeet hotwords"): on 3 clips with an unrelated hotword in the list,
/// 1.5-2.5 fixed the names without inserting it, 3.5 inserted it and 5
/// produced garbage. On the 40-clip set with 22 hotwords (2 relevant),
/// 0.5, 1.0 and 1.5 all cut WER 1.2% → 0.9%; higher scores re-cased more
/// words (12 / 17 / 22 surface-only changes), so 1.0 is the compromise.
pub const DEFAULT_HOTWORDS_SCORE: f32 = 1.0;
/// Cap on hotword phrases per decode (the context graph grows with them,
/// and so does the chance of a false insertion).
pub const MAX_HOTWORDS: usize = 100;
/// Beam width. 2 paths with hotwords raised WER 1.2% → 2.1% (split
/// apostrophes, mis-segmented words); 4 paths gave 0.9%.
pub const DEFAULT_BEAM: usize = 4;
/// RMS below this is treated as no speech (same gate as the whisper backend).
const SILENCE_RMS: f32 = 0.003;

#[derive(Debug, thiserror::Error)]
pub enum SherpaError {
    #[error("model load failed: {0}")]
    Load(String),
    #[error("transcription failed: {0}")]
    Transcribe(String),
}

#[derive(Debug, Clone)]
pub struct ParakeetConfig {
    /// Directory with `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and
    /// `tokens.txt` (int8 files are preferred when both exist).
    pub model_dir: PathBuf,
    pub threads: usize,
    /// Use modified beam search so per-decode hotwords work. `false` = greedy
    /// search, hotwords ignored.
    pub hotwords: bool,
    pub hotwords_score: f32,
    /// Beam width for modified beam search.
    pub max_active_paths: usize,
    /// With `hotwords`, also create a greedy-search recognizer: used for
    /// decodes without hotwords and to retry a beam decode that came back
    /// empty. Parakeet v2 int8 with beam 4 returns "" on some long clips
    /// (25.1 s 6/6, 42.8 s; beam 8 too) where greedy is fine
    /// (docs/proposals/stt-model-survey.md). Costs a second set of
    /// onnxruntime sessions (≈ +0.6 GB RSS). Default true.
    pub greedy_fallback: bool,
    /// Where to write the derived `bpe.vocab` when the model directory has
    /// none. `None` = `std::env::temp_dir()`. The file is deleted once the
    /// recognizer has read it. Android: pass the app's cache dir.
    pub scratch_dir: Option<PathBuf>,
    /// Create the beam and greedy recognizers on two threads at once
    /// (onnxruntime session setup is mostly one core per session), so a
    /// load takes about as long as the slower one instead of both in turn.
    /// Default false: two Parakeet encoder sessions (hundreds of MB each)
    /// initialising at once raise the load's peak RAM, unmeasured on the
    /// phone (the zipformer, ~70 MB, does it by default:
    /// [`OnlineConfig::parallel_sessions`]).
    pub parallel_sessions: bool,
}

impl ParakeetConfig {
    pub fn new(model_dir: impl Into<PathBuf>, threads: usize) -> Self {
        Self {
            model_dir: model_dir.into(),
            threads,
            hotwords: true,
            hotwords_score: DEFAULT_HOTWORDS_SCORE,
            max_active_paths: DEFAULT_BEAM,
            greedy_fallback: true,
            scratch_dir: None,
            parallel_sessions: false,
        }
    }
}

/// One timed step of a model load (milliseconds), for the shells' load log
/// (`docs/findings/2026-11-05-phone-model-load-time.md`). Names: `files`
/// (find the model files), `vocab` (derive `bpe.vocab` for hotwords),
/// `beam_sessions` / `greedy_sessions` (one recognizer's onnxruntime
/// sessions, timed on its own thread), `sessions` (wall time of all
/// recognizers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadPhase {
    pub name: &'static str,
    pub ms: u64,
}

fn ms_since(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

/// Run `f`, recording how long it took as phase `name`.
fn timed<T>(phases: &mut Vec<LoadPhase>, name: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    phases.push(LoadPhase {
        name,
        ms: ms_since(t),
    });
    r
}

/// A deferred recognizer creation for [`create_pair`].
type MakeRecognizer<'a, R> = Box<dyn FnOnce() -> Result<R, SherpaError> + Send + 'a>;

/// Create an optional beam and an optional greedy recognizer, on two threads
/// when `parallel` and both are wanted. Records `beam_sessions`,
/// `greedy_sessions` (each one's own time) and `sessions` (wall time).
/// An error from either is returned after both have finished.
fn create_pair<R: Send>(
    phases: &mut Vec<LoadPhase>,
    parallel: bool,
    beam: Option<MakeRecognizer<'_, R>>,
    greedy: Option<MakeRecognizer<'_, R>>,
) -> Result<(Option<R>, Option<R>), SherpaError> {
    type TimedRun<R> = Option<(Result<R, SherpaError>, u64)>;
    fn run<R>(f: Option<MakeRecognizer<'_, R>>) -> TimedRun<R> {
        f.map(|f| {
            let t = Instant::now();
            (f(), ms_since(t))
        })
    }
    let wall = Instant::now();
    let (b, g): (TimedRun<R>, TimedRun<R>) = match (beam, greedy) {
        (Some(b), Some(g)) if parallel => std::thread::scope(|s| {
            let h = s.spawn(move || run(Some(g)));
            let b = run(Some(b));
            (b, h.join().expect("recognizer thread panicked"))
        }),
        (b, g) => (run(b), run(g)),
    };
    for (name, r) in [("beam_sessions", &b), ("greedy_sessions", &g)] {
        if let Some((_, ms)) = r {
            phases.push(LoadPhase { name, ms: *ms });
        }
    }
    phases.push(LoadPhase {
        name: "sessions",
        ms: ms_since(wall),
    });
    Ok((
        b.map(|(r, _)| r).transpose()?,
        g.map(|(r, _)| r).transpose()?,
    ))
}

/// The four files of a sherpa-onnx NeMo transducer directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

impl ModelFiles {
    pub fn find(dir: &Path) -> Result<Self, SherpaError> {
        if !dir.is_dir() {
            return Err(SherpaError::Load(format!(
                "{}: not a directory (expected a sherpa-onnx Parakeet model dir)",
                dir.display()
            )));
        }
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map_err(|e| SherpaError::Load(format!("{}: {e}", dir.display())))?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .collect();
        names.sort();
        let pick = |part: &str| -> Result<PathBuf, SherpaError> {
            let cands: Vec<&String> = names
                .iter()
                .filter(|n| n.starts_with(part) && n.ends_with(".onnx"))
                .collect();
            cands
                .iter()
                .find(|n| n.contains(".int8."))
                .or(cands.first())
                .map(|n| dir.join(n))
                .ok_or_else(|| SherpaError::Load(format!("{}: no {part}*.onnx", dir.display())))
        };
        let tokens = dir.join("tokens.txt");
        if !tokens.is_file() {
            return Err(SherpaError::Load(format!(
                "{}: no tokens.txt",
                dir.display()
            )));
        }
        Ok(Self {
            encoder: pick("encoder")?,
            decoder: pick("decoder")?,
            joiner: pick("joiner")?,
            tokens,
        })
    }
}

/// Derive a sentencepiece `bpe.vocab` (`piece \t score`) from sherpa's
/// `tokens.txt` (`piece id`). The NeMo exports do not ship the vocab, and
/// sherpa needs it only to encode hotwords. Score = -id: sentencepiece BPE
/// assigns scores in merge order, which is id order, so this reproduces the
/// model's own segmentation closely. A flat score (all 0) was measured to be
/// harmful: words split into many single-letter pieces, each piece earned the
/// bonus, and an unrelated hotword was inserted all over the transcript.
pub fn bpe_vocab_from_tokens(tokens_txt: &str) -> String {
    let mut out = String::with_capacity(tokens_txt.len() + tokens_txt.len() / 4);
    for line in tokens_txt.lines() {
        let mut it = line.rsplitn(2, char::is_whitespace);
        let (Some(id), Some(piece)) = (it.next(), it.next()) else {
            continue;
        };
        let Ok(id) = id.parse::<i64>() else { continue };
        if piece.is_empty() {
            continue;
        }
        out.push_str(&format!("{piece}\t{}\n", -id));
    }
    out
}

/// Render phrases as sherpa's per-stream hotword string: `/`-separated
/// phrases. Separators and control characters inside a phrase become spaces;
/// blanks and case-insensitive duplicates are dropped; at most
/// [`MAX_HOTWORDS`] phrases are kept, in order.
pub fn hotwords_arg<S: AsRef<str>>(phrases: &[S]) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    for p in phrases {
        let clean: String = p
            .as_ref()
            .chars()
            .map(|c| if c == '/' || c.is_control() { ' ' } else { c })
            .collect();
        let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.is_empty() || seen.contains(&clean.to_lowercase()) {
            continue;
        }
        seen.push(clean.to_lowercase());
        out.push(clean);
        if out.len() == MAX_HOTWORDS {
            break;
        }
    }
    out.join("/")
}

pub fn is_silent(pcm: &[f32]) -> bool {
    if pcm.is_empty() {
        return true;
    }
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt();
    rms < SILENCE_RMS
}

/// Decoding method of one decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Search {
    /// Modified beam search (hotwords honoured).
    Beam,
    Greedy,
}

pub struct ParakeetEngine {
    /// Modified beam search with hotwords (when enabled and the vocab loaded).
    beam: Option<Mutex<OfflineRecognizer>>,
    /// Greedy search (always when `beam` is `None`).
    greedy: Option<Mutex<OfflineRecognizer>>,
    phases: Vec<LoadPhase>,
}

impl ParakeetEngine {
    /// Create the onnxruntime sessions. Expensive (≈ 1-4 s warm, tens of
    /// seconds from a cold HDD); call once and keep the engine.
    pub fn load(cfg: &ParakeetConfig) -> Result<Self, SherpaError> {
        let mut phases = Vec::new();
        let files = timed(&mut phases, "files", || ModelFiles::find(&cfg.model_dir))?;
        let s = |p: &Path| Some(p.to_string_lossy().into_owned());
        let dir = cfg.model_dir.display().to_string();
        let created = move |c: OfflineRecognizerConfig| {
            OfflineRecognizer::create(&c)
                .map(Mutex::new)
                .ok_or_else(|| {
                    SherpaError::Load(format!(
                        "{dir}: sherpa-onnx could not create the recognizer (see stderr)"
                    ))
                })
        };
        let mut c = OfflineRecognizerConfig::default();
        c.model_config.transducer = OfflineTransducerModelConfig {
            encoder: s(&files.encoder),
            decoder: s(&files.decoder),
            joiner: s(&files.joiner),
        };
        c.model_config.tokens = s(&files.tokens);
        c.model_config.model_type = Some("nemo_transducer".into());
        c.model_config.num_threads = cfg.threads.max(1) as i32;
        c.model_config.provider = Some("cpu".into());
        c.decoding_method = Some("greedy_search".into());

        let mut temp_vocab = None;
        let mut beam_cfg = None;
        if cfg.hotwords {
            let vocab = timed(&mut phases, "vocab", || {
                vocab_path(&cfg.model_dir, cfg.scratch_dir.as_deref(), &files.tokens)
            });
            match vocab {
                Ok((path, is_temp)) => {
                    let mut b = c.clone();
                    b.decoding_method = Some("modified_beam_search".into());
                    b.max_active_paths = cfg.max_active_paths.max(1) as i32;
                    b.hotwords_score = cfg.hotwords_score;
                    b.model_config.modeling_unit = Some("bpe".into());
                    b.model_config.bpe_vocab = s(&path);
                    if is_temp {
                        temp_vocab = Some(path);
                    }
                    beam_cfg = Some(b);
                }
                // No writable place for the vocab: still transcribe, unbiased.
                Err(e) => eprintln!("flow-asr-sherpa: hotwords disabled: {e}"),
            }
        }
        let want_greedy = beam_cfg.is_none() || cfg.greedy_fallback;
        let created_b = created.clone();
        let made = create_pair(
            &mut phases,
            cfg.parallel_sessions,
            beam_cfg.map(|b| Box::new(move || created_b(b)) as MakeRecognizer<'_, _>),
            want_greedy.then(|| Box::new(move || created(c)) as MakeRecognizer<'_, _>),
        );
        if let Some(p) = temp_vocab {
            let _ = std::fs::remove_file(p);
        }
        let (beam, greedy) = made?;
        Ok(Self {
            beam,
            greedy,
            phases,
        })
    }

    /// How long each step of [`ParakeetEngine::load`] took.
    pub fn load_phases(&self) -> &[LoadPhase] {
        &self.phases
    }

    /// Whether per-decode hotwords are honoured (beam search + vocab loaded).
    pub fn supports_hotwords(&self) -> bool {
        self.beam.is_some()
    }

    /// Whether a greedy recognizer is loaded (see `greedy_fallback`).
    pub fn has_greedy(&self) -> bool {
        self.greedy.is_some()
    }

    /// The search [`ParakeetEngine::transcribe`] uses for these hotwords:
    /// beam with hotwords, otherwise greedy when loaded.
    pub fn default_search<S: AsRef<str>>(&self, hotwords: &[S]) -> Search {
        if self.beam.is_some() && (self.greedy.is_none() || !hotwords_arg(hotwords).is_empty()) {
            Search::Beam
        } else {
            Search::Greedy
        }
    }

    /// Decode a whole utterance (16 kHz mono f32). `hotwords` biases the
    /// decode toward those phrases (ignored when unsupported). Beam search
    /// only with hotwords; greedy otherwise (when loaded).
    pub fn transcribe<S: AsRef<str>>(
        &self,
        pcm: &[f32],
        hotwords: &[S],
    ) -> Result<String, SherpaError> {
        self.transcribe_with(pcm, hotwords, self.default_search(hotwords))
    }

    /// Decode with an explicit search; falls back to the loaded one when
    /// the requested recognizer is not loaded. Hotwords only reach beam.
    pub fn transcribe_with<S: AsRef<str>>(
        &self,
        pcm: &[f32],
        hotwords: &[S],
        search: Search,
    ) -> Result<String, SherpaError> {
        if is_silent(pcm) {
            return Ok(String::new());
        }
        let mut audio;
        let pcm = if pcm.len() < MIN_SAMPLES {
            audio = pcm.to_vec();
            audio.resize(MIN_SAMPLES, 0.0);
            &audio[..]
        } else {
            pcm
        };
        let (rec, is_beam) = match (search, &self.beam, &self.greedy) {
            (Search::Beam, Some(b), _) | (Search::Greedy, Some(b), None) => (b, true),
            (_, _, Some(g)) => (g, false),
            (_, None, None) => unreachable!("load creates at least one recognizer"),
        };
        let rec = rec.lock().unwrap_or_else(|p| p.into_inner());
        let hw = if is_beam {
            hotwords_arg(hotwords)
        } else {
            String::new()
        };
        let stream = if hw.is_empty() {
            rec.create_stream()
        } else {
            rec.create_stream_with_hotwords(&hw)
        };
        stream.accept_waveform(SAMPLE_RATE, pcm);
        rec.decode(&stream);
        let r = stream
            .get_result()
            .ok_or_else(|| SherpaError::Transcribe("sherpa-onnx returned no result".into()))?;
        Ok(r.text.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

/// The `model_type` metadata of an ONNX file (`zipformer2` for icefall's
/// streaming zipformer), read from the top level of the `ModelProto`
/// without parsing the graph (field 7 is skipped by its length, so this
/// reads a few hundred bytes). `None` when absent or unreadable.
///
/// sherpa-onnx's online transducer creates a whole extra encoder session
/// just to read this when `model_type` is not configured (measured
/// 2026-11-05: half of the zipformer-he load).
pub fn onnx_model_type(path: &Path) -> Option<String> {
    use std::io::{BufReader, Read, Seek, SeekFrom};
    fn varint(r: &mut impl Read) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let mut b = [0u8];
            r.read_exact(&mut b).ok()?;
            v |= u64::from(b[0] & 0x7f) << shift;
            if b[0] & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    /// `key`/`value` of a `StringStringEntryProto`.
    fn entry(b: &[u8]) -> Option<(String, String)> {
        let mut r = b;
        let (mut k, mut v) = (String::new(), String::new());
        while !r.is_empty() {
            let tag = varint(&mut r)?;
            if tag & 7 != 2 {
                return None;
            }
            let len = usize::try_from(varint(&mut r)?).ok()?;
            let (field, rest) = (r.get(..len)?, &r[len..]);
            let text = String::from_utf8(field.to_vec()).ok()?;
            match tag >> 3 {
                1 => k = text,
                2 => v = text,
                _ => {}
            }
            r = rest;
        }
        Some((k, v))
    }
    const METADATA_PROPS: u64 = 14;
    /// Metadata entries are short; anything longer is not one.
    const MAX_ENTRY: u64 = 1 << 16;
    let mut r = BufReader::new(std::fs::File::open(path).ok()?);
    loop {
        let tag = varint(&mut r)?;
        match tag & 7 {
            0 => {
                varint(&mut r)?;
            }
            1 => {
                r.seek_relative(8).ok()?;
            }
            5 => {
                r.seek_relative(4).ok()?;
            }
            2 => {
                let len = varint(&mut r)?;
                if tag >> 3 == METADATA_PROPS && len <= MAX_ENTRY {
                    let mut b = vec![0u8; len as usize];
                    r.read_exact(&mut b).ok()?;
                    if let Some((k, v)) = entry(&b) {
                        if k == "model_type" {
                            return Some(v);
                        }
                    }
                } else {
                    r.seek(SeekFrom::Current(i64::try_from(len).ok()?)).ok()?;
                }
            }
            _ => return None,
        }
    }
}

/// `bpe.vocab` shipped in the model dir, else one derived from `tokens.txt`
/// in the scratch dir (returned with `is_temp = true`).
fn vocab_path(
    model_dir: &Path,
    scratch_dir: Option<&Path>,
    tokens: &Path,
) -> Result<(PathBuf, bool), String> {
    let shipped = model_dir.join("bpe.vocab");
    if shipped.is_file() {
        return Ok((shipped, false));
    }
    let tokens =
        std::fs::read_to_string(tokens).map_err(|e| format!("{}: {e}", tokens.display()))?;
    let dir = scratch_dir.map_or_else(std::env::temp_dir, Path::to_path_buf);
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("flow-sherpa-{}-{n}.vocab", std::process::id()));
    std::fs::write(&path, bpe_vocab_from_tokens(&tokens))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((path, true))
}

/// The decode interface both engines share, so flow-core's sherpa adapter
/// (beam/greedy retry, piecewise fallback) can drive either one. Object
/// safe; phrases are `String`s because that is what core passes.
pub trait SherpaTranscriber: Send + Sync {
    /// Whether per-decode hotwords are honoured (beam search + vocab loaded).
    fn supports_hotwords(&self) -> bool;
    /// Whether a greedy recognizer is loaded.
    fn has_greedy(&self) -> bool;
    /// The search a plain decode with these hotwords uses.
    fn default_search(&self, hotwords: &[String]) -> Search;
    /// Decode one utterance (16 kHz mono f32) with an explicit search.
    fn transcribe_with(
        &self,
        pcm: &[f32],
        hotwords: &[String],
        search: Search,
    ) -> Result<String, SherpaError>;
    /// The language this model writes, when it is fixed by configuration
    /// (`None` = the caller knows, e.g. the English Parakeet).
    fn language(&self) -> Option<&str>;
}

impl SherpaTranscriber for ParakeetEngine {
    fn supports_hotwords(&self) -> bool {
        ParakeetEngine::supports_hotwords(self)
    }
    fn has_greedy(&self) -> bool {
        ParakeetEngine::has_greedy(self)
    }
    fn default_search(&self, hotwords: &[String]) -> Search {
        ParakeetEngine::default_search(self, hotwords)
    }
    fn transcribe_with(
        &self,
        pcm: &[f32],
        hotwords: &[String],
        search: Search,
    ) -> Result<String, SherpaError> {
        ParakeetEngine::transcribe_with(self, pcm, hotwords, search)
    }
    fn language(&self) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocab_scores_follow_token_ids() {
        let v = bpe_vocab_from_tokens("<unk> 0\n▁t 1\n▁th 2\nbad\n\nx y\n");
        assert_eq!(v, "<unk>\t0\n▁t\t-1\n▁th\t-2\n");
    }

    #[test]
    fn hotwords_are_sanitised_deduped_and_capped() {
        assert_eq!(
            hotwords_arg(&["Kubernetes", " kubernetes ", "", "TCP/IP", "a\nb"]),
            "Kubernetes/TCP IP/a b"
        );
        let many: Vec<String> = (0..500).map(|i| format!("w{i}")).collect();
        assert_eq!(hotwords_arg(&many).split('/').count(), MAX_HOTWORDS);
        assert_eq!(hotwords_arg::<&str>(&[]), "");
    }

    #[test]
    fn model_files_prefer_int8_and_report_missing() {
        let d = std::env::temp_dir().join(format!("flow-sherpa-files-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for f in [
            "encoder.onnx",
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.onnx",
        ] {
            std::fs::write(d.join(f), b"").unwrap();
        }
        assert!(
            matches!(ModelFiles::find(&d), Err(SherpaError::Load(m)) if m.contains("tokens.txt"))
        );
        std::fs::write(d.join("tokens.txt"), b"").unwrap();
        let f = ModelFiles::find(&d).unwrap();
        assert_eq!(f.encoder, d.join("encoder.int8.onnx"));
        assert_eq!(f.joiner, d.join("joiner.onnx"));
        std::fs::remove_dir_all(&d).unwrap();
        assert!(ModelFiles::find(&d).is_err());
    }

    /// Protobuf length-delimited field.
    fn len_field(field: u8, body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 128);
        [&[field << 3 | 2, body.len() as u8][..], body].concat()
    }

    #[test]
    fn model_type_is_read_from_the_metadata_without_the_graph() {
        let d = std::env::temp_dir().join(format!("flow-sherpa-meta-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let entry =
            |k: &[u8], v: &[u8]| len_field(14, &[len_field(1, k), len_field(2, v)].concat());
        let model = [
            vec![1 << 3, 8],                          // ir_version: 8
            len_field(2, b"made-up-exporter"),        // producer_name
            len_field(7, &[0xff; 100]),               // graph: skipped, not parsed
            vec![5 << 3 | 1, 0, 0, 0, 0, 0, 0, 0, 0], // a fixed64 field
            entry(b"version", b"1"),
            entry(b"model_type", b"zipformer2"),
        ]
        .concat();
        let f = d.join("encoder.onnx");
        std::fs::write(&f, &model).unwrap();
        assert_eq!(onnx_model_type(&f).as_deref(), Some("zipformer2"));
        // No such entry, a truncated file, no file.
        std::fs::write(&f, &model[..model.len() - 16]).unwrap();
        assert_eq!(onnx_model_type(&f), None);
        std::fs::write(&f, [1 << 3, 8]).unwrap();
        assert_eq!(onnx_model_type(&f), None);
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(onnx_model_type(&f), None);
    }

    #[test]
    fn silence_gate() {
        assert!(is_silent(&[]));
        assert!(is_silent(&[0.0; 1600]));
        assert!(!is_silent(&[0.2; 1600]));
    }
}
