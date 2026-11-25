//! How the real sherpa recognizers take hotword words that start with a
//! sherpa marker character (`#` threshold, `@` phrase text, as in sherpa's
//! keyword-file format). Env-gated: skips (passes with a note) unless
//! `MURMUR_MARKERS_ASR` (a model folder), `MURMUR_MARKERS_WAV` (a 16 kHz
//! mono clip) and `MURMUR_MARKERS_PHRASES` (`|`-separated screen phrases)
//! are set. `MURMUR_MARKERS_ENGINE=sherpa_online` uses the streaming
//! zipformer (language `he`), else Parakeet. One case per process, so a
//! decoder abort shows as a failed run, not a lost sweep. Prints the
//! hotword string sent and the text.
//!
//! `cargo test -p murmur-core --features wav,parakeet --release --test hotword_markers_e2e -- --nocapture`

use std::path::Path;

use murmur_core::asr::{AsrEngine, AsrRequest};
use murmur_core::audio;
use murmur_core::parakeet::ParakeetAsr;
use murmur_core::sherpa_online::SherpaOnlineAsr;

#[test]
fn marker_words_in_hotwords() {
    let (Ok(asr), Ok(wav), Ok(phrases)) = (
        std::env::var("MURMUR_MARKERS_ASR"),
        std::env::var("MURMUR_MARKERS_WAV"),
        std::env::var("MURMUR_MARKERS_PHRASES"),
    ) else {
        eprintln!("SKIP: set MURMUR_MARKERS_ASR, MURMUR_MARKERS_WAV, MURMUR_MARKERS_PHRASES");
        return;
    };
    let engine: Box<dyn AsrEngine> = match std::env::var("MURMUR_MARKERS_ENGINE").as_deref() {
        Ok("sherpa_online") => Box::new(SherpaOnlineAsr::load(&asr, 2, Some("he")).unwrap()),
        _ => Box::new(ParakeetAsr::load(&asr, 2).unwrap()),
    };
    let phrases: Vec<String> = phrases
        .split('|')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let req = AsrRequest {
        initial_prompt: phrases.join(", "),
        screen_terms: phrases,
        ..Default::default()
    };
    let pcm16 = audio::read_wav_16k_mono(Path::new(&wav)).unwrap();
    let pcm: Vec<f32> = pcm16.iter().map(|&s| s as f32 / 32768.0).collect();
    let pcm = murmur_core::vad::asr_level(&pcm).into_owned();
    let arg = murmur_asr_sherpa::hotwords_arg(&murmur_core::asr::sherpa_hotwords(&req));
    let text = engine.transcribe(&pcm, &req).unwrap().text;
    eprintln!("HOTWORDS {arg:?}\nTEXT {text:?}");
}
