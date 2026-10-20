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
            _ctx: ctx,
        })
    }
}

impl AsrEngine for WhisperEngine {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let lang = if self.multilingual {
            req.language.clone()
        } else {
            Some("en".into())
        };
        if is_silent(pcm) {
            return Ok(AsrOutput {
                text: String::new(),
                language: lang,
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
        if !req.initial_prompt.is_empty() {
            p.set_initial_prompt(&req.initial_prompt);
        }
        if req.abort.0.is_some() {
            let flag = req.abort.clone();
            p.set_abort_callback_safe(move || flag.is_set());
        }
        let audio = pad(pcm);
        let mut st = self.state.lock().unwrap();
        st.full(p, &audio)
            .map_err(|e| AsrError::Transcribe(e.to_string()))?;
        let mut text = String::new();
        for seg in st.as_iter() {
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
    fn rms_gate_detects_silence() {
        assert!(is_silent(&[0.0; 16000]));
        assert!(!is_silent(&[0.2; 16000]));
    }
}
