//! whisper.cpp backend via `whisper-rs` (feature `whisper`).
//!
//! One warm `WhisperContext` + reusable state per core. Bias comes from
//! `initial_prompt`; language is forced or auto-detected (English-only
//! `.en` models are always forced to `en`).

use std::sync::{Mutex, Once};

use regex::Regex;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use crate::config::SAMPLE_RATE;

/// whisper.cpp refuses (< 1 s) input; pad with silence.
pub const MIN_SAMPLES: usize = SAMPLE_RATE + SAMPLE_RATE / 10;
/// RMS below this is treated as no speech (avoids silence hallucinations).
const SILENCE_RMS: f32 = 0.003;

/// Encoder positions per second of audio (1500 per 30 s window).
const CTX_PER_SEC: usize = 50;
const FULL_CTX: usize = 1500;

/// The CPU features this whisper.cpp/ggml build was compiled for, e.g.
/// `"... CPU : SSE3 = 1 | AVX2 = 1 | ..."`. Compile-time flags, not what the
/// running CPU supports: a feature listed here that the CPU lacks is an
/// illegal-instruction crash on the first decode.
pub fn build_cpu_features() -> &'static str {
    whisper_rs::print_system_info()
}

/// `audio_ctx` for `samples` of (padded) audio: its length plus 1 s, in
/// encoder positions; 0 (= the full 30 s window) when that is no smaller.
pub fn audio_ctx_for(samples: usize) -> i32 {
    let ctx = (samples * CTX_PER_SEC).div_ceil(SAMPLE_RATE) + CTX_PER_SEC;
    if ctx >= FULL_CTX {
        0
    } else {
        ctx as i32
    }
}

/// Repetition/length guard for a shrunken encoder window (the benchmark
/// saw `-ac` loop a sentence 3× on base and garble tiny): more than
/// 4 words per second (+3), or a repeated phrase (below), means "decode
/// again with the full window". A false alarm only costs the re-decode.
pub fn looks_degenerate(text: &str, secs: f32) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() as f32 > 4.0 * secs + 3.0 {
        return true;
    }
    // A phrase of 3+ words twice in a row ("And so my fellow Americans.
    // And so my fellow Americans." on a 3 s clip), or a 1-2 word one three
    // times (not "no no no" / "very very very": short single words).
    let rep = |k: usize, times: usize| {
        (0..(words.len() + 1).saturating_sub(times * k))
            .any(|i| (1..times).all(|t| words[i..i + k] == words[i + t * k..i + (t + 1) * k]))
    };
    (3..=12).any(|k| rep(k, 2))
        || rep(2, 3)
        || (0..words.len().saturating_sub(2))
            .any(|i| words[i].len() > 4 && words[i] == words[i + 1] && words[i] == words[i + 2])
}

pub fn pad(pcm: &[f32]) -> Vec<f32> {
    let mut v = pcm.to_vec();
    if v.len() < MIN_SAMPLES {
        v.resize(MIN_SAMPLES, 0.0);
    }
    v
}

pub fn is_silent(pcm: &[f32]) -> bool {
    if pcm.is_empty() {
        return true;
    }
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt();
    rms < SILENCE_RMS
}

/// Drop Whisper's non-speech annotations (`[BLANK_AUDIO]`, `(music)`, ...)
/// and normalise whitespace.
pub fn clean_text(s: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\s*(\[[^\]]*\]|\([^)]*\))").unwrap());
    let t = re.replace_all(s, "");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct WhisperEngine {
    state: Mutex<WhisperState>,
    multilingual: bool,
    threads: usize,
    /// Silero VAD model (ggml) for whisper.cpp's built-in VAD, if set.
    silero: Option<String>,
    /// Shrink the encoder window to the clip (`audio_ctx`), with a
    /// repetition guard that re-decodes with the full window.
    fit_ctx: bool,
    /// Full-window re-decodes caused by the guard (tests, benchmarks).
    redecodes: std::sync::atomic::AtomicU64,
    // Keep the context alive for the state's lifetime.
    _ctx: WhisperContext,
}

impl WhisperEngine {
    pub fn load(path: &str, threads: usize) -> Result<Self, AsrError> {
        static HOOK: Once = Once::new();
        HOOK.call_once(whisper_rs::install_logging_hooks);
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| AsrError::Load(format!("{path}: {e}")))?;
        let state = ctx
            .create_state()
            .map_err(|e| AsrError::Load(e.to_string()))?;
        Ok(Self {
            multilingual: ctx.is_multilingual(),
            state: Mutex::new(state),
            threads: threads.max(1),
            silero: None,
            fit_ctx: false,
            redecodes: Default::default(),
            _ctx: ctx,
        })
    }

    /// Enable whisper.cpp's built-in Silero VAD (model file such as
    /// `ggml-silero-v5.1.2.bin`): whisper then decodes only the speech
    /// segments it finds. Runs in addition to flow-core's energy VAD.
    pub fn with_silero_vad(mut self, model_path: &str) -> Self {
        self.silero = Some(model_path.to_string());
        self
    }

    /// Size the encoder window to the clip (`ModelConfig.whisper_fit_ctx`).
    pub fn with_fit_ctx(mut self, on: bool) -> Self {
        self.fit_ctx = on;
        self
    }

    /// How many decodes the repetition guard re-ran with the full window.
    pub fn redecodes(&self) -> u64 {
        self.redecodes.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl AsrEngine for WhisperEngine {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let audio = pad(pcm);
        let ctx = if self.fit_ctx {
            audio_ctx_for(audio.len())
        } else {
            0
        };
        let out = self.decode(pcm, &audio, req, ctx)?;
        if ctx > 0 && looks_degenerate(&out.text, audio.len() as f32 / SAMPLE_RATE as f32) {
            self.redecodes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return self.decode(pcm, &audio, req, 0);
        }
        Ok(out)
    }
}

impl WhisperEngine {
    /// One whisper.cpp run; `pcm` is the unpadded input (silence gate),
    /// `audio` the padded one; `audio_ctx` 0 = full window.
    fn decode(
        &self,
        pcm: &[f32],
        audio: &[f32],
        req: &AsrRequest,
        audio_ctx: i32,
    ) -> Result<AsrOutput, AsrError> {
        let lang = if self.multilingual {
            req.language.clone()
        } else {
            Some("en".into())
        };
        if is_silent(pcm) {
            return Ok(AsrOutput {
                text: String::new(),
                language: lang,
                no_speech_prob: None,
            });
        }
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_n_threads(self.threads as i32);
        p.set_language(Some(lang.as_deref().unwrap_or("auto")));
        p.set_no_context(true);
        p.set_suppress_blank(true);
        p.set_suppress_nst(true);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_special(false);
        p.set_print_timestamps(false);
        if audio_ctx > 0 {
            p.set_audio_ctx(audio_ctx);
        }
        // D22: the previous segments' last words follow the dictionary.
        let prompt = crate::asr::compose_prompt(&req.initial_prompt, req.prior_text.as_deref());
        if !prompt.is_empty() {
            p.set_initial_prompt(&prompt);
        }
        if let Some(v) = &self.silero {
            p.set_vad_model_path(Some(v));
            p.enable_vad(true);
        }
        // whisper-rs 0.16's `set_abort_callback_safe` boxes the closure as
        // `Box<dyn FnMut>` but its trampoline reads it back as the concrete
        // closure type, so the callback read garbage and aborted decodes at
        // random (whisper error -6). Pass the flag itself instead; `flag`
        // (an `Arc`) outlives `st.full` below.
        let flag = req.abort.0.clone();
        if let Some(f) = &flag {
            unsafe extern "C" fn abort_cb(user_data: *mut std::ffi::c_void) -> bool {
                // SAFETY: user_data is `Arc::as_ptr` of a live AtomicBool.
                unsafe {
                    (*(user_data as *const std::sync::atomic::AtomicBool))
                        .load(std::sync::atomic::Ordering::Relaxed)
                }
            }
            // SAFETY: the pointer stays valid for the whole `full` call.
            unsafe {
                p.set_abort_callback(Some(abort_cb));
                p.set_abort_callback_user_data(std::sync::Arc::as_ptr(f) as *mut std::ffi::c_void);
            }
        }
        let mut st = self.state.lock().unwrap();
        let full = st.full(p, audio);
        drop(flag);
        full.map_err(|e| AsrError::Transcribe(e.to_string()))?;
        let mut text = String::new();
        let mut no_speech: Option<f32> = None;
        for seg in st.as_iter() {
            let p = seg.no_speech_probability();
            no_speech = Some(no_speech.map_or(p, |q| q.max(p)));
            text.push_str(
                &seg.to_str_lossy()
                    .map_err(|e| AsrError::Transcribe(e.to_string()))?,
            );
        }
        let language = match lang {
            Some(l) => Some(l),
            None => whisper_rs::get_lang_str(st.full_lang_id_from_state()).map(str::to_string),
        };
        Ok(AsrOutput {
            text: clean_text(&text),
            language,
            no_speech_prob: no_speech,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_non_speech_annotations() {
        assert_eq!(clean_text(" [BLANK_AUDIO] "), "");
        assert_eq!(
            clean_text(" Hello (music) world [Laughter]."),
            "Hello world."
        );
        assert_eq!(
            clean_text(" And so, my fellow Americans"),
            "And so, my fellow Americans"
        );
    }

    #[test]
    fn pads_short_audio_to_minimum() {
        assert_eq!(pad(&[0.1; 100]).len(), MIN_SAMPLES);
        assert_eq!(pad(&vec![0.1; 40_000]).len(), 40_000);
    }

    #[test]
    fn audio_ctx_fits_the_clip_plus_a_second() {
        assert_eq!(audio_ctx_for(MIN_SAMPLES), 55 + 50);
        assert_eq!(audio_ctx_for(SAMPLE_RATE * 5), 300);
        assert_eq!(audio_ctx_for(SAMPLE_RATE * 29), 0);
        assert_eq!(audio_ctx_for(SAMPLE_RATE * 40), 0);
    }

    #[test]
    fn repetition_guard() {
        let jfk = "And so, my fellow Americans, ask not what your country can do for you, ask what you can do for your country.";
        assert!(!looks_degenerate(jfk, 11.0));
        let looped = "I went home. I went home. I went home. I went home.";
        assert!(looks_degenerate(looped, 5.0));
        assert!(looks_degenerate(&format!("{jfk} {jfk} {jfk}"), 11.0));
        // Too many words for the audio.
        assert!(looks_degenerate(&"word ".repeat(30), 5.0));
        // Natural repeats stay.
        assert!(!looks_degenerate("No no no, that is not it.", 2.0));
        assert!(!looks_degenerate("It is very very very good.", 2.0));
        // Short clip: one sentence twice is already a loop.
        assert!(looks_degenerate(
            "And so my fellow Americans. And so my fellow Americans.",
            3.0
        ));
        assert!(!looks_degenerate("I think, I think we should go.", 3.0));
    }

    #[test]
    fn rms_gate_detects_silence() {
        assert!(is_silent(&[0.0; 16000]));
        assert!(!is_silent(&[0.2; 16000]));
    }
}
