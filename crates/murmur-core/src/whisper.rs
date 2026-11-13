//! whisper.cpp backend via `whisper-rs` (feature `whisper`).
//!
//! One warm `WhisperContext` + reusable state per core. Bias comes from
//! `initial_prompt`; language is forced or auto-detected (English-only
//! `.en` models are always forced to `en`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use regex::Regex;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use crate::config::SAMPLE_RATE;
use crate::recital::{self, Recital};

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

/// A phrase this long seen twice anywhere in one decode is a loop, even
/// with other words between the copies (D66). A shrunken window on a
/// short segment piece repeated a 14-word span with 4 words between
/// (parity eval 2026-11-07,
/// `docs/findings/2026-11-07-whisper-segment-joins.md`), which the
/// back-to-back checks in [`looks_degenerate`] miss.
const LOOP_PHRASE_WORDS: usize = 6;

/// Repetition/length guard for a shrunken encoder window (the benchmark
/// saw `-ac` loop a sentence 3× on base and garble tiny): more than
/// 4 words per second (+3), or a repeated phrase (below), means "decode
/// again with the full window". A false alarm only costs the re-decode.
///
/// Words are split on whitespace, so scripts written without spaces
/// (Chinese, Japanese, Thai) get no protection from the phrase checks: a
/// whole clause is one "word", so only the words/s cap and back-to-back
/// repeats of whole clauses can fire.
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
        || repeats_phrase(&words, LOOP_PHRASE_WORDS)
        || (0..words.len().saturating_sub(2))
            .any(|i| words[i].len() > 4 && words[i] == words[i + 1] && words[i] == words[i + 2])
}

/// Whether some run of `k` words occurs twice without overlapping.
fn repeats_phrase(words: &[String], k: usize) -> bool {
    debug_assert!(k > 0);
    if words.len() < 2 * k {
        return false;
    }
    let mut first: HashMap<&[String], usize> = HashMap::new();
    for i in 0..=words.len() - k {
        let g = &words[i..i + k];
        match first.get(g) {
            Some(&j) if i >= j + k => return true,
            Some(_) => {}
            None => {
                first.insert(g, i);
            }
        }
    }
    false
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

/// Where whisper.cpp runs a model. Only a GPU build of whisper.cpp (the
/// Windows `murmur_whisper_vulkan.dll` plugin, crate `murmur-whisper`) has a GPU
/// to use; in a CPU build `Gpu` loads on the CPU, as whisper.cpp finds no
/// GPU device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperDevice {
    Cpu,
    /// whisper.cpp's `gpu_device`: the index among the GPU devices ggml
    /// lists (discrete and integrated).
    Gpu(i32),
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
    /// `<|startoftranscript|>` and `<|nospeech|>` (D91).
    sot: i32,
    nosp: i32,
    /// Re-decodes by [`decode_guarded`] (tests, benchmarks).
    redecodes: std::sync::atomic::AtomicU64,
    // Keep the context alive for the state's lifetime.
    _ctx: WhisperContext,
}

impl WhisperEngine {
    pub fn load(path: &str, threads: usize) -> Result<Self, AsrError> {
        Self::load_with(path, threads, WhisperContextParameters::default())
    }

    /// [`WhisperEngine::load`] on a chosen device.
    pub fn load_on(path: &str, threads: usize, device: WhisperDevice) -> Result<Self, AsrError> {
        let mut p = WhisperContextParameters::default();
        match device {
            WhisperDevice::Cpu => p.use_gpu(false),
            WhisperDevice::Gpu(i) => p.use_gpu(true).gpu_device(i),
        };
        Self::load_with(path, threads, p)
    }

    fn load_with(
        path: &str,
        threads: usize,
        params: WhisperContextParameters,
    ) -> Result<Self, AsrError> {
        static HOOK: Once = Once::new();
        HOOK.call_once(whisper_rs::install_logging_hooks);
        let ctx = WhisperContext::new_with_params(path, params)
            .map_err(|e| AsrError::Load(format!("{path}: {e}")))?;
        let state = ctx
            .create_state()
            .map_err(|e| AsrError::Load(e.to_string()))?;
        Ok(Self {
            multilingual: ctx.is_multilingual(),
            sot: ctx.token_sot(),
            nosp: ctx.token_nosp(),
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
    /// segments it finds. Runs in addition to murmur-core's energy VAD.
    pub fn with_silero_vad(mut self, model_path: &str) -> Self {
        self.silero = Some(model_path.to_string());
        self
    }

    /// Size the encoder window to the clip (`ModelConfig.whisper_fit_ctx`).
    pub fn with_fit_ctx(mut self, on: bool) -> Self {
        self.fit_ctx = on;
        self
    }

    /// How many decodes were run again: with the full window (repetition
    /// guard, D20) or without the prompt (recital, D87).
    pub fn redecodes(&self) -> u64 {
        self.redecodes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The model is multilingual (not a `.en` model).
    pub fn is_multilingual(&self) -> bool {
        self.multilingual
    }

    /// [`AsrEngine::transcribe`] with the stop flag passed as a plain
    /// reference instead of `req.abort` (which is ignored): the
    /// `murmur-whisper` plugin gets its flag as a pointer from the host.
    pub fn transcribe_abortable(
        &self,
        pcm: &[f32],
        req: &AsrRequest,
        abort: Option<&AtomicBool>,
    ) -> Result<AsrOutput, AsrError> {
        let audio = pad(pcm);
        let ctx = if self.fit_ctx {
            audio_ctx_for(audio.len())
        } else {
            0
        };
        let secs = audio.len() as f32 / SAMPLE_RATE as f32;
        decode_guarded(ctx, secs, &req.initial_prompt, &self.redecodes, |pass| {
            self.decode(pcm, &audio, req, pass, abort)
        })
    }
}

/// whisper.cpp's decode settings for one `WhisperEngine` run: greedy, no
/// carried context, `audio_ctx` 0 = full window, `prompt` as
/// `initial_prompt` when not empty. The engine adds its Silero VAD, abort
/// flag and no-speech probe (D91) on top. Public so tests can run
/// whisper.cpp alone with the same settings.
pub fn full_params<'a>(
    threads: usize,
    language: &'a str,
    audio_ctx: i32,
    prompt: &str,
) -> FullParams<'a, 'a> {
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_n_threads(threads as i32);
    p.set_language(Some(language));
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
    if !prompt.is_empty() {
        p.set_initial_prompt(prompt);
    }
    p
}

/// One whisper.cpp run of [`decode_guarded`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pass {
    /// Encoder window; 0 = the full 30 s.
    audio_ctx: i32,
    /// Whether `initial_prompt` (the dictionary and screen words) is used;
    /// D22's `prior_text` always is.
    use_prompt: bool,
}

/// Decode with encoder window `ctx` (0 = full) and `prompt`, then at most
/// once more (counted in `redecodes`):
/// - D87: without the prompt when the result recites it
///   ([`recital::check`]), with the full window if it also
///   [`looks_degenerate`];
/// - D20: else with the full window when a fitted result
///   [`looks_degenerate`] over `secs` of audio.
///
/// A recited list after real speech is cut off, on either decode. The
/// second result is otherwise used as is.
fn decode_guarded(
    ctx: i32,
    secs: f32,
    prompt: &str,
    redecodes: &std::sync::atomic::AtomicU64,
    mut decode: impl FnMut(Pass) -> Result<AsrOutput, AsrError>,
) -> Result<AsrOutput, AsrError> {
    let strip_tail = |mut out: AsrOutput| {
        if let Recital::Tail(speech) = recital::check(&out.text, prompt) {
            out.text = speech;
        }
        out
    };
    let out = decode(Pass {
        audio_ctx: ctx,
        use_prompt: true,
    })?;
    if recital::check(&out.text, prompt) == Recital::Whole {
        redecodes.fetch_add(1, Ordering::Relaxed);
        let degenerate = ctx > 0 && looks_degenerate(&out.text, secs);
        return decode(Pass {
            audio_ctx: if degenerate { 0 } else { ctx },
            use_prompt: false,
        });
    }
    let out = strip_tail(out);
    if ctx > 0 && looks_degenerate(&out.text, secs) {
        redecodes.fetch_add(1, Ordering::Relaxed);
        return decode(Pass {
            audio_ctx: 0,
            use_prompt: true,
        })
        .map(strip_tail);
    }
    Ok(out)
}

impl AsrEngine for WhisperEngine {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        self.transcribe_abortable(pcm, req, req.abort.0.as_deref())
    }
}

impl WhisperEngine {
    /// One whisper.cpp run; `pcm` is the unpadded input (silence gate),
    /// `audio` the padded one.
    fn decode(
        &self,
        pcm: &[f32],
        audio: &[f32],
        req: &AsrRequest,
        pass: Pass,
        abort: Option<&AtomicBool>,
    ) -> Result<AsrOutput, AsrError> {
        let Pass {
            audio_ctx,
            use_prompt,
        } = pass;
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
                avg_logprob: None,
            });
        }
        // D22: the previous segments' last words follow the dictionary.
        let initial = if use_prompt {
            req.initial_prompt.as_str()
        } else {
            ""
        };
        let prompt = crate::asr::compose_prompt(initial, req.prior_text.as_deref());
        let mut p = full_params(
            self.threads,
            lang.as_deref().unwrap_or("auto"),
            audio_ctx,
            &prompt,
        );
        if let Some(v) = &self.silero {
            p.set_vad_model_path(Some(v));
            p.enable_vad(true);
        }
        // whisper-rs 0.16's `set_abort_callback_safe` boxes the closure as
        // `Box<dyn FnMut>` but its trampoline reads it back as the concrete
        // closure type, so the callback read garbage and aborted decodes at
        // random (whisper error -6). Pass the flag itself instead; `abort`
        // outlives `st.full` below.
        if let Some(f) = abort {
            unsafe extern "C" fn abort_cb(user_data: *mut std::ffi::c_void) -> bool {
                // SAFETY: user_data points at a live AtomicBool.
                unsafe { (*(user_data as *const AtomicBool)).load(Ordering::Relaxed) }
            }
            // SAFETY: the pointer stays valid for the whole `full` call.
            unsafe {
                p.set_abort_callback(Some(abort_cb));
                p.set_abort_callback_user_data(f as *const AtomicBool as *mut std::ffi::c_void);
            }
        }
        // D91: Whisper's no-speech probability, read per window by us
        // (see `NoSpeechProbe`); `probe` outlives `st.full` below.
        let mut probe = NoSpeechProbe::new(self.sot, self.nosp, self.threads);
        // SAFETY: the callbacks only touch `probe` and the state whisper.cpp
        // passes in, between two decodes (see `on_new_segment`).
        unsafe {
            let ud = &mut probe as *mut NoSpeechProbe as *mut std::ffi::c_void;
            p.set_start_encoder_callback(Some(on_encoder_begin));
            p.set_start_encoder_callback_user_data(ud);
            p.set_new_segment_callback(Some(on_new_segment));
            p.set_new_segment_callback_user_data(ud);
        }
        let mut st = self.state.lock().unwrap();
        let full = st.full(p, audio);
        full.map_err(|e| AsrError::Transcribe(e.to_string()))?;
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
        let (no_speech_prob, avg_logprob) = probe.result(st.full_n_segments(), |segs| {
            segs.filter_map(|i| st.get_segment(i))
                .flat_map(|seg| {
                    (0..seg.n_tokens())
                        .filter_map(|j| seg.get_token(j).map(|t| t.token_data().plog))
                        .collect::<Vec<_>>()
                })
                .collect()
        });
        Ok(AsrOutput {
            text: clean_text(&text),
            language,
            no_speech_prob,
            avg_logprob,
        })
    }
}

/// D91: Whisper's no-speech probability, read at `<|startoftranscript|>`
/// once per encoder window, as OpenAI Whisper reads it.
///
/// whisper.cpp 1.8.3's own value (`whisper_full_get_segment_no_speech_prob`)
/// is unusable: `whisper_full_with_state` decodes the whole prompt
/// ([`prev`, `initial_prompt`…], `sot`, language, task) but keeps the
/// logits of the last prompt token only, in row `prompt.size() - 1`, and
/// then reads `<|nospeech|>` from row 0. Row 0 is the `sot` row only when
/// the prompt is `sot` alone (an English-only model with no prompt).
/// Otherwise it holds stale logits from an earlier decode step and the
/// value is about 0: on every multilingual model, and on English-only
/// models given a dictionary prompt.
///
/// So when a window gives its first segment (whisper.cpp's new-segment
/// callback, while that window's encoder output is still in the state),
/// `sot` alone is decoded against it: one decoder step. This differs from
/// OpenAI Whisper in one way: OpenAI reads at `sot` after the prompt
/// tokens (`prev` and the initial prompt come first in its sequence), so
/// its value is conditioned on the prompt; ours decodes `sot` with no
/// prompt before it, so a dictionary prompt cannot move it. With no
/// prompt the two are the same value. A window without text (a
/// near-empty tail window after the last timestamp) is not read.
///
/// Each read window also gets OpenAI's `avg_logprob`: the sum of its
/// sampled tokens' log-probabilities over (tokens + 1) (`decoding.py`
/// counts the end-of-text token in both; whisper.cpp does not keep that
/// token, so only the count has it). The engine reports the pair of the
/// window with the highest no-speech probability; the guard needs both
/// (`hallucination::whisper_says_no_speech`).
///
/// The probe leaves its logits in row 0, where whisper.cpp's broken read
/// of the next decode would find them and could then drop a segment as
/// "no speech" on the strength of an earlier window. So the row is zeroed
/// afterwards: whisper.cpp then reads 1/n_vocab, about 0, as it did
/// before, and its own no-speech drop stays as it was (it works only for
/// an English-only model with no prompt, whose row 0 is its own `sot`).
struct NoSpeechProbe {
    sot: i32,
    nosp: i32,
    threads: i32,
    windows: Vec<ProbeWindow>,
}

/// One encoder window as the probe saw it.
struct ProbeWindow {
    /// Segments before this window.
    first_seg: i32,
    /// Its no-speech probability was read (or the read failed).
    read: bool,
    no_speech: Option<f32>,
}

impl NoSpeechProbe {
    fn new(sot: i32, nosp: i32, threads: usize) -> Self {
        Self {
            sot,
            nosp,
            threads: threads as i32,
            windows: Vec::new(),
        }
    }

    /// A new encoder window starts after `segs` segments.
    fn window(&mut self, segs: i32) {
        self.windows.push(ProbeWindow {
            first_seg: segs,
            read: false,
            no_speech: None,
        });
    }

    /// The current window gave a segment; `read` reads its no-speech
    /// probability (once per window).
    fn segment(&mut self, read: impl FnOnce() -> Option<f32>) {
        if self.windows.is_empty() {
            self.window(0);
        }
        let w = self.windows.last_mut().expect("a window");
        if !std::mem::replace(&mut w.read, true) {
            w.no_speech = read();
        }
    }

    /// `(no_speech_prob, avg_logprob)` of the read window with the highest
    /// no-speech probability, after `full` made `n_segs` segments in all.
    /// `plogs` gives the token log-probabilities of a range of segments.
    fn result(
        &self,
        n_segs: i32,
        mut plogs: impl FnMut(std::ops::Range<i32>) -> Vec<f32>,
    ) -> (Option<f32>, Option<f32>) {
        let ends = self.windows.iter().skip(1).map(|w| w.first_seg);
        self.windows
            .iter()
            .zip(ends.chain(std::iter::once(n_segs)))
            .filter_map(|(w, end)| Some((w.no_speech?, avg_logprob(&plogs(w.first_seg..end)))))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map_or((None, None), |(p, l)| (Some(p), Some(l)))
    }
}

/// OpenAI's `avg_logprob` for a window's sampled tokens: the sum over
/// (count + 1), the `+ 1` being the end-of-text token.
fn avg_logprob(plogs: &[f32]) -> f32 {
    plogs.iter().sum::<f32>() / (plogs.len() + 1) as f32
}

/// whisper.cpp's encoder-begin callback.
unsafe extern "C" fn on_encoder_begin(
    _ctx: *mut whisper_rs::whisper_rs_sys::whisper_context,
    st: *mut whisper_rs::whisper_rs_sys::whisper_state,
    user_data: *mut std::ffi::c_void,
) -> bool {
    // SAFETY: user_data is the `NoSpeechProbe` set in `decode`, alive for
    // the whole `full` call; st is whisper.cpp's own.
    unsafe {
        let segs = whisper_rs::whisper_rs_sys::whisper_full_n_segments_from_state(st);
        (*(user_data as *mut NoSpeechProbe)).window(segs);
    }
    true
}

/// whisper.cpp's new-segment callback: reads the window's no-speech
/// probability. whisper.cpp calls it once the window is decoded and before
/// the next encode; `decode_with_state` only rewrites the decoder batch,
/// self-attention cache and logits, which whisper.cpp resets before its
/// next decode (row 0 aside, zeroed here).
unsafe extern "C" fn on_new_segment(
    ctx: *mut whisper_rs::whisper_rs_sys::whisper_context,
    st: *mut whisper_rs::whisper_rs_sys::whisper_state,
    _n_new: std::ffi::c_int,
    user_data: *mut std::ffi::c_void,
) {
    use whisper_rs::whisper_rs_sys as sys;
    // SAFETY: user_data is the `NoSpeechProbe` set in `decode`, alive for
    // the whole `full` call; ctx and st are whisper.cpp's own, and a
    // one-token decode fills logits row 0 (n_vocab floats).
    unsafe {
        let probe = &mut *(user_data as *mut NoSpeechProbe);
        let (sot, nosp, threads) = (probe.sot, probe.nosp, probe.threads);
        probe.segment(|| {
            if sys::whisper_decode_with_state(ctx, st, &sot, 1, 0, threads) != 0 {
                return None;
            }
            let row = std::slice::from_raw_parts_mut(
                sys::whisper_get_logits_from_state(st),
                sys::whisper_n_vocab(ctx) as usize,
            );
            let p = softmax_at(row, nosp as usize);
            row.fill(0.0);
            Some(p)
        });
    }
}

/// `softmax(logits)[i]`. `logits` is one row of `whisper_n_vocab` floats,
/// the same count whisper.cpp's own softmax uses (`id_to_token.size()`,
/// which `whisper_process_logits` asserts equals `n_vocab`).
fn softmax_at(logits: &[f32], i: usize) -> f32 {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = logits.iter().map(|l| (l - max).exp()).sum();
    (logits[i] - max).exp() / sum
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
    fn a_loop_with_words_between_is_decoded_again_with_the_full_window() {
        // Fake decoder: the fitted window loops (synthetic text), the full
        // window does not.
        let looped = "we moved the meeting to the blue room on the third floor \
                      and then we said we moved the meeting to the blue room on the third floor";
        let clean = "we moved the meeting to the blue room on the third floor";
        let n = std::sync::atomic::AtomicU64::new(0);
        let mut seen = Vec::new();
        let out = decode_guarded(500, 9.0, "", &n, |p| {
            seen.push(p.audio_ctx);
            Ok(AsrOutput {
                text: if p.audio_ctx > 0 { looped } else { clean }.into(),
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!(seen, [500, 0]);
        assert_eq!(out.text, clean);
        assert_eq!(n.load(Ordering::Relaxed), 1);
        // A clean fitted result is kept: one decode, no re-decode.
        seen.clear();
        let out = decode_guarded(500, 9.0, "", &n, |p| {
            seen.push(p.audio_ctx);
            Ok(AsrOutput {
                text: clean.into(),
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!((seen.as_slice(), out.text.as_str()), (&[500][..], clean));
        // The full window (ctx 0) is never re-decoded, looped or not.
        seen.clear();
        decode_guarded(0, 9.0, "", &n, |p| {
            seen.push(p.audio_ctx);
            Ok(AsrOutput {
                text: looped.into(),
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!(seen, [0]);
        assert_eq!(n.load(Ordering::Relaxed), 1);
    }

    /// Fake decoder for the recital guard: `with` while the prompt is
    /// used, `without` once it is dropped; records each pass.
    fn fake<'a>(
        seen: &'a mut Vec<Pass>,
        with: &'a str,
        without: &'a str,
    ) -> impl FnMut(Pass) -> Result<AsrOutput, AsrError> + 'a {
        move |p| {
            seen.push(p);
            Ok(AsrOutput {
                text: if p.use_prompt { with } else { without }.into(),
                ..Default::default()
            })
        }
    }

    #[test]
    fn a_recited_prompt_is_decoded_again_without_it() {
        // Synthetic dictionary; Whisper recites it on a near-silent clip.
        let prompt = "Quorvel, Tamsik, Brennolt, Vexmoor, Lintrava.";
        let recited = "Quorvel, Tamsik, Brennolt, Vexmoor.";
        let n = std::sync::atomic::AtomicU64::new(0);
        let mut seen = Vec::new();
        let out = decode_guarded(0, 9.0, prompt, &n, fake(&mut seen, recited, "")).unwrap();
        let pass = |audio_ctx, use_prompt| Pass {
            audio_ctx,
            use_prompt,
        };
        assert_eq!(seen, [pass(0, true), pass(0, false)]);
        assert_eq!(out.text, "");
        assert_eq!(n.load(Ordering::Relaxed), 1);
        // On a fitted window the retry keeps it, unless the recital also
        // looks degenerate (4 list words on a 0.2 s clip).
        seen.clear();
        decode_guarded(500, 9.0, prompt, &n, fake(&mut seen, recited, "")).unwrap();
        assert_eq!(seen, [pass(500, true), pass(500, false)]);
        seen.clear();
        decode_guarded(500, 0.2, prompt, &n, fake(&mut seen, recited, "")).unwrap();
        assert_eq!(seen, [pass(500, true), pass(0, false)]);
        assert_eq!(n.load(Ordering::Relaxed), 3);
        // Speech that uses the list is kept, in one decode.
        seen.clear();
        let said = "Ask Quorvel and Tamsik about the draft.";
        let out = decode_guarded(500, 9.0, prompt, &n, fake(&mut seen, said, "x")).unwrap();
        assert_eq!((seen.len(), out.text.as_str()), (1, said));
        assert_eq!(n.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_recited_list_after_speech_is_cut_without_a_redecode() {
        let prompt = "Pellish, Orrin, Quorvel, Tamsik, Brennolt, Vexmoor.";
        let n = std::sync::atomic::AtomicU64::new(0);
        let mut seen = Vec::new();
        let out = decode_guarded(
            500,
            9.0,
            prompt,
            &n,
            fake(
                &mut seen,
                "Move the review to Friday. Pellish, Orrin, Quorvel, Tamsik, Brennolt, Vexmoor.",
                "x",
            ),
        )
        .unwrap();
        assert_eq!(out.text, "Move the review to Friday.");
        assert_eq!((seen.len(), n.load(Ordering::Relaxed)), (1, 0));
    }

    #[test]
    fn repetition_guard_catches_a_loop_with_words_between() {
        // Synthetic: a long phrase, a few other words, the phrase again,
        // at a word rate the length check accepts.
        let looped = "we moved the meeting to the blue room on the third floor \
                      and then we said we moved the meeting to the blue room on the third floor";
        assert!(looks_degenerate(looped, 9.0));
        // A short phrase said twice is normal speech.
        assert!(!looks_degenerate(
            "Send it to the team. Then later send it to the client as well.",
            5.0
        ));
        // Five words twice is still below the bar.
        assert!(!looks_degenerate(
            "Put the box on the table, then put the box on the shelf.",
            5.0
        ));
    }

    #[test]
    fn softmax_at_is_a_probability() {
        let l = [1.0f32, 2.0, 3.0, 1000.0];
        assert!((softmax_at(&l, 3) - 1.0).abs() < 1e-6);
        assert!(softmax_at(&l, 0) < 1e-6);
        let flat = [0.5f32; 4];
        assert!((softmax_at(&flat, 2) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn no_speech_is_read_once_per_window_and_paired_with_its_logprob() {
        let mut p = NoSpeechProbe::new(0, 1, 1);
        // Window 1 gives segments 0-1: read once.
        p.window(0);
        p.segment(|| Some(0.2));
        p.segment(|| panic!("read twice in one window"));
        // Window 2 gives none (a near-empty tail window): not read.
        p.window(2);
        // Window 3 gives segment 2 and has the highest value.
        p.window(2);
        p.segment(|| Some(0.7));
        // Window 4 gives segment 3; a failed read leaves no value.
        p.window(3);
        p.segment(|| None);
        let mut asked = Vec::new();
        let (nsp, lp) = p.result(4, |r| {
            asked.push(r.clone());
            // Two tokens per segment, -0.3 each.
            vec![-0.3; 2 * r.len()]
        });
        assert_eq!(asked, [0..2, 2..3]);
        assert_eq!(nsp, Some(0.7));
        // Window 3: two tokens, (-0.6) / 3.
        assert!((lp.unwrap() + 0.2).abs() < 1e-6);
        // No text, no value.
        let mut q = NoSpeechProbe::new(0, 1, 1);
        q.window(0);
        assert_eq!(q.result(0, |_| vec![]), (None, None));
    }

    #[test]
    fn avg_logprob_counts_the_end_token() {
        assert!((avg_logprob(&[-1.0, -2.0, -3.0]) + 1.5).abs() < 1e-6);
        assert_eq!(avg_logprob(&[]), 0.0);
    }

    #[test]
    fn rms_gate_detects_silence() {
        assert!(is_silent(&[0.0; 16000]));
        assert!(!is_silent(&[0.2; 16000]));
    }
}
