//! Streaming (online) transducer through sherpa-onnx's `OnlineRecognizer`,
//! for icefall zipformer2 models such as `ivrit-ai/zipformer2-66m-streaming-he`
//! (docs/proposals/hebrew-stt.md decision 3).
//!
//! First version decodes a whole utterance at commit: the buffer is fed in
//! one go, followed by [`DEFAULT_TAIL_PADDING_S`] of silence (the streaming
//! encoder holds back its right context until it sees more frames) and
//! `input_finished()`, then decoded while ready. Measured RTF ≈ 0.05 on the
//! i7-6700 (docs/findings/2026-10-28-hebrew-stt-models.md). Feeding chunks
//! live while the user speaks is a later, app-wide change.
//!
//! The model type comes from the encoder's ONNX metadata
//! ([`crate::onnx_model_type`]) and is passed to sherpa: left empty, sherpa
//! creates a whole extra encoder session just to read that metadata, which
//! was half of the zipformer-he load (2026-11-05,
//! docs/findings/2026-11-05-phone-model-load-time.md).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineTransducerModelConfig};

use crate::{
    create_pair, hotwords_arg, is_silent, onnx_model_type, timed, vocab_path, LoadPhase, Make,
    ModelFiles, Search, SherpaError, SherpaTranscriber, DEFAULT_BEAM, DEFAULT_HOTWORDS_SCORE,
    SAMPLE_RATE,
};

/// Silence appended after the utterance before `input_finished()`. The
/// model card's sherpa example and the 2026-10-28 probe use 0.8-1.5 s; the
/// last word is lost without it. 1.5 s costs ≈ 75 ms at RTF 0.05.
pub const DEFAULT_TAIL_PADDING_S: f32 = 1.5;

/// Online transducer types sherpa-onnx 1.13 takes as `model_type` (the
/// encoder metadata's own values). Anything else is left to sherpa's
/// detection, as before.
const KNOWN_MODEL_TYPES: [&str; 4] = ["conformer", "lstm", "zipformer", "zipformer2"];

#[derive(Debug, Clone)]
pub struct OnlineConfig {
    /// Directory with `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and
    /// `tokens.txt` (int8 preferred), as in [`ModelFiles::find`].
    pub model_dir: PathBuf,
    pub threads: usize,
    /// Language the model writes (BCP-47 base code, e.g. `he`), reported by
    /// [`SherpaTranscriber::language`]. `None` = unknown.
    pub language: Option<String>,
    /// Modified beam search with per-decode hotwords; `false` = greedy only.
    /// sherpa's online beam search adds the hotword bonus after picking the
    /// top `max_active_paths` candidates, so hotwords only re-rank what the
    /// model already considers (fixes spellings and word joins, never
    /// inserts): docs/findings/2026-10-30-hebrew-model-pins.md.
    pub hotwords: bool,
    pub hotwords_score: f32,
    pub max_active_paths: usize,
    /// With `hotwords`, also load a greedy recognizer for decodes without
    /// hotwords (greedy is faster; the zipformer is 72 MB, so the second
    /// set of sessions is cheap in RAM, and [`OnlineConfig::parallel_sessions`]
    /// keeps it off the load time).
    pub greedy_fallback: bool,
    /// Seconds of silence fed after the audio (see [`DEFAULT_TAIL_PADDING_S`]).
    pub tail_padding_s: f32,
    /// Where to write a derived `bpe.vocab` (see `ParakeetConfig`).
    pub scratch_dir: Option<PathBuf>,
    /// Create the beam and greedy recognizers at once (see
    /// `ParakeetConfig::parallel_sessions`). Default true.
    pub parallel_sessions: bool,
}

impl OnlineConfig {
    pub fn new(model_dir: impl Into<PathBuf>, threads: usize, language: Option<&str>) -> Self {
        Self {
            model_dir: model_dir.into(),
            threads,
            language: language.map(str::to_string),
            hotwords: true,
            hotwords_score: DEFAULT_HOTWORDS_SCORE,
            max_active_paths: DEFAULT_BEAM,
            greedy_fallback: true,
            tail_padding_s: DEFAULT_TAIL_PADDING_S,
            scratch_dir: None,
            parallel_sessions: true,
        }
    }
}

pub struct OnlineEngine {
    beam: Option<Mutex<OnlineRecognizer>>,
    greedy: Option<Mutex<OnlineRecognizer>>,
    language: Option<String>,
    tail: Vec<f32>,
    phases: Vec<LoadPhase>,
}

impl OnlineEngine {
    /// Create the onnxruntime sessions (≈ 1-5 s); call once and keep.
    pub fn load(cfg: &OnlineConfig) -> Result<Self, SherpaError> {
        let mut phases = Vec::new();
        let files = timed(&mut phases, "files", || ModelFiles::find(&cfg.model_dir))?;
        let model_type = timed(&mut phases, "model_type", || {
            onnx_model_type(&files.encoder).filter(|t| KNOWN_MODEL_TYPES.contains(&t.as_str()))
        });
        let s = |p: &Path| Some(p.to_string_lossy().into_owned());
        let dir = cfg.model_dir.display().to_string();
        let created = move |c: OnlineRecognizerConfig| {
            OnlineRecognizer::create(&c).map(Mutex::new).ok_or_else(|| {
                SherpaError::Load(format!(
                    "{dir}: sherpa-onnx could not create the online recognizer (see stderr)"
                ))
            })
        };
        let mut c = OnlineRecognizerConfig::default();
        c.model_config.transducer = OnlineTransducerModelConfig {
            encoder: s(&files.encoder),
            decoder: s(&files.decoder),
            joiner: s(&files.joiner),
        };
        c.model_config.tokens = s(&files.tokens);
        c.model_config.model_type = model_type;
        c.model_config.num_threads = cfg.threads.max(1) as i32;
        c.model_config.provider = Some("cpu".into());
        // One utterance per stream: no endpointing inside it.
        c.enable_endpoint = false;
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
                Err(e) => eprintln!("flow-asr-sherpa: online hotwords disabled: {e}"),
            }
        }
        let want_greedy = beam_cfg.is_none() || cfg.greedy_fallback;
        let created_b = created.clone();
        let made = create_pair(
            &mut phases,
            cfg.parallel_sessions,
            beam_cfg.map(|b| Box::new(move || created_b(b)) as Make<'_, _>),
            want_greedy.then(|| Box::new(move || created(c)) as Make<'_, _>),
        );
        if let Some(p) = temp_vocab {
            let _ = std::fs::remove_file(p);
        }
        let (beam, greedy) = made?;
        let tail_len = (cfg.tail_padding_s.max(0.0) * SAMPLE_RATE as f32) as usize;
        Ok(Self {
            beam,
            greedy,
            language: cfg.language.clone(),
            tail: vec![0.0; tail_len],
            phases,
        })
    }

    /// How long each step of [`OnlineEngine::load`] took.
    pub fn load_phases(&self) -> &[LoadPhase] {
        &self.phases
    }

    pub fn supports_hotwords(&self) -> bool {
        self.beam.is_some()
    }

    pub fn has_greedy(&self) -> bool {
        self.greedy.is_some()
    }

    /// Beam with hotwords (or when it is the only recognizer), else greedy.
    pub fn default_search<S: AsRef<str>>(&self, hotwords: &[S]) -> Search {
        if self.beam.is_some() && (self.greedy.is_none() || !hotwords_arg(hotwords).is_empty()) {
            Search::Beam
        } else {
            Search::Greedy
        }
    }

    pub fn transcribe<S: AsRef<str>>(
        &self,
        pcm: &[f32],
        hotwords: &[S],
    ) -> Result<String, SherpaError> {
        self.transcribe_with(pcm, hotwords, self.default_search(hotwords))
    }

    /// Feed the whole utterance + tail silence, finish the stream, decode
    /// while ready. Falls back to the loaded recognizer when the requested
    /// one is not; hotwords reach only beam search.
    pub fn transcribe_with<S: AsRef<str>>(
        &self,
        pcm: &[f32],
        hotwords: &[S],
        search: Search,
    ) -> Result<String, SherpaError> {
        if is_silent(pcm) {
            return Ok(String::new());
        }
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
        if !self.tail.is_empty() {
            stream.accept_waveform(SAMPLE_RATE, &self.tail);
        }
        stream.input_finished();
        while rec.is_ready(&stream) {
            rec.decode(&stream);
        }
        let r = rec
            .get_result(&stream)
            .ok_or_else(|| SherpaError::Transcribe("sherpa-onnx returned no result".into()))?;
        Ok(r.text.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

impl SherpaTranscriber for OnlineEngine {
    fn supports_hotwords(&self) -> bool {
        OnlineEngine::supports_hotwords(self)
    }
    fn has_greedy(&self) -> bool {
        OnlineEngine::has_greedy(self)
    }
    fn default_search(&self, hotwords: &[String]) -> Search {
        OnlineEngine::default_search(self, hotwords)
    }
    fn transcribe_with(
        &self,
        pcm: &[f32],
        hotwords: &[String],
        search: Search,
    ) -> Result<String, SherpaError> {
        OnlineEngine::transcribe_with(self, pcm, hotwords, search)
    }
    fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let c = OnlineConfig::new("/m", 2, Some("he"));
        assert_eq!(c.language.as_deref(), Some("he"));
        assert!(c.hotwords && c.greedy_fallback);
        assert_eq!(c.tail_padding_s, DEFAULT_TAIL_PADDING_S);
        assert_eq!(OnlineConfig::new("/m", 2, None).language, None);
    }

    #[test]
    fn missing_dir_is_a_load_error() {
        let e = OnlineEngine::load(&OnlineConfig::new("/nonexistent/zipformer", 1, None));
        assert!(matches!(e, Err(SherpaError::Load(_))));
    }
}
