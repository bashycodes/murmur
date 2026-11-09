//! End-to-end: real Parakeet-TDT model through the full FlowCore pipeline.
//! Skips (passes with a note) without the model: `tools/fetch-models.sh
//! --parakeet`, or FLOW_PARAKEET_DIR. The hotword case also needs the
//! LibriSpeech clip 5142-36377-0008.wav in FLOW_AUDIO_DIR (default models/).

use std::path::PathBuf;
use std::time::Instant;

use flow_core::config::{AsrBackend, ModelConfig, SessionConfig};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{audio, CommitStatus, FlowCore};

mod common;
use common::long_clips;

fn models() -> PathBuf {
    std::env::var_os("FLOW_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn load() -> Option<FlowCore> {
    let dir = std::env::var_os("FLOW_PARAKEET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| models().join("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"));
    if !dir.join("tokens.txt").is_file() || !models().join("jfk.wav").is_file() {
        eprintln!("SKIP: {} or jfk.wav missing", dir.display());
        return None;
    }
    let t = Instant::now();
    let cfg: ModelConfig = serde_json::from_value(serde_json::json!({
        "asr_engine": "parakeet",
        "asr_path": dir,
        "threads": 4,
    }))
    .unwrap();
    assert_eq!(cfg.asr_backend, AsrBackend::Parakeet);
    let core = FlowCore::load_models(&cfg).expect("load parakeet");
    eprintln!("parakeet load_models: {} ms", t.elapsed().as_millis());
    Some(core)
}

fn dictate(core: &FlowCore, wav: &std::path::Path, cfg: SessionConfig) -> flow_core::CommitResult {
    let pcm = audio::read_wav_16k_mono(wav).unwrap();
    core.start_session(cfg).unwrap();
    for c in pcm.chunks(640) {
        assert!(core.push_audio(c));
    }
    let r = core.commit();
    eprintln!(
        "audio {} ms | asr {} ms | {:?}: {}",
        r.timings.audio_ms, r.timings.asr_ms, r.status, r.final_text
    );
    r
}

#[test]
fn parakeet_jfk_end_to_end_with_dictionary_and_hotwords() {
    let Some(core) = load() else { return };
    let r = dictate(
        &core,
        &models().join("jfk.wav"),
        SessionConfig {
            dictionary: Dictionary {
                entries: vec![DictEntry::replacement(
                    "fellow americans",
                    "fellow Americans!",
                )],
            },
            ..Default::default()
        },
    );
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.language.as_deref(), Some("en"));
    assert!(
        r.asr_text
            .starts_with("And so, my fellow Americans, ask not what your country can do for you"),
        "{}",
        r.asr_text
    );
    assert!(
        r.final_text.contains("fellow Americans!"),
        "{}",
        r.final_text
    );
    // RTF guard, loose: the bench measured 0.12-0.14 on a quiet box; allow a
    // heavily shared one.
    assert!(r.timings.asr_ms < 11_000, "asr {} ms", r.timings.asr_ms);

    // Contextual biasing: the dictionary word reaches sherpa as a hotword.
    let clip = std::env::var_os("FLOW_AUDIO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(models)
        .join("5142-36377-0008.wav");
    if !clip.is_file() {
        eprintln!("SKIP hotword case: {} missing", clip.display());
        return;
    }
    let plain = dictate(&core, &clip, SessionConfig::default());
    assert!(plain.asr_text.contains("Meadocroft"), "{}", plain.asr_text);
    let biased = dictate(
        &core,
        &clip,
        SessionConfig {
            dictionary: Dictionary {
                entries: vec![DictEntry::word("Meadowcroft")],
            },
            ..Default::default()
        },
    );
    assert!(
        biased.asr_text.contains("Meadowcroft"),
        "{}",
        biased.asr_text
    );
}

#[test]
fn parakeet_silence_is_empty() {
    let Some(core) = load() else { return };
    core.start_session(SessionConfig::default()).unwrap();
    core.push_audio(&[0i16; 32_000]);
    assert_eq!(core.commit().status, CommitStatus::Empty);
}

#[test]
fn parakeet_bad_dir_fails_load() {
    let r = FlowCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Parakeet,
        asr_path: "/nonexistent/parakeet".into(),
        ..Default::default()
    });
    match r {
        Err(flow_core::CoreError::ModelNotFound { path }) => {
            assert_eq!(path, "/nonexistent/parakeet")
        }
        Err(e) => panic!("wrong error: {e:?}"),
        Ok(_) => panic!("loaded a missing model dir"),
    }
}

/// Regression (data loss, docs/proposals/streaming-asr-latency.md §1.3):
/// beam search returned "" for the 41.7 s probe clip 3/3 runs, an EMPTY
/// `blank_asr` dictation. The engine now re-decodes in pieces cut at
/// pauses. Needs FLOW_AUDIO_DIR with the LibriSpeech manifest clips.
#[test]
fn parakeet_long_clip_is_not_empty() {
    let Some(core) = load() else { return };
    let Some(clips) = long_clips::audio_dir().and_then(|d| long_clips::build(&d)) else {
        eprintln!("SKIP: FLOW_AUDIO_DIR with the manifest clips not set");
        return;
    };
    for clip in clips.iter().filter(|c| c.target_s >= 32) {
        core.start_session(SessionConfig {
            // Pushed at once: the ring must hold the whole clip.
            ring_capacity_samples: clip.pcm.len() + 16_000,
            ..Default::default()
        })
        .unwrap();
        for c in clip.pcm.chunks(640) {
            assert!(core.push_audio(c));
        }
        let r = core.commit();
        let (e, n) = long_clips::wer_counts(&clip.reference, &r.asr_text);
        eprintln!(
            "{:.1} s: {:?} asr {} ms, WER {:.1}%",
            clip.secs(),
            r.status,
            r.timings.asr_ms,
            100.0 * e as f32 / n as f32
        );
        assert_eq!(
            r.status,
            CommitStatus::RawTranscript,
            "{:.1} s: {r:?}",
            clip.secs()
        );
        assert!(
            e * 10 < n,
            "{:.1} s: WER {e}/{n}: {}",
            clip.secs(),
            r.asr_text
        );
    }
}

/// Survey regression: beam 4 returned "" on long clips (6/6), and one-shot
/// greedy decodes dropped whole utterances (here: the 17 s third sentence
/// of the 30 s clip, WER 58%). Clips over 20 s are now decoded in pieces
/// cut at pauses, greedy without hotwords, beam (+ greedy retry) with: no
/// sentence may go missing, with or without a dictionary.
#[test]
fn parakeet_survey_long_clips_are_not_empty_with_or_without_hotwords() {
    let Some(core) = load() else { return };
    let Some(audio) = long_clips::audio_dir() else {
        eprintln!("SKIP: FLOW_AUDIO_DIR not set");
        return;
    };
    for n in [3, 4] {
        let Some(clip) = long_clips::survey(&audio, n) else {
            return;
        };
        for dict in [vec![], vec![DictEntry::word("Meadowcroft")]] {
            let biased = !dict.is_empty();
            core.start_session(SessionConfig {
                ring_capacity_samples: clip.pcm.len() + 16_000,
                dictionary: Dictionary { entries: dict },
                ..Default::default()
            })
            .unwrap();
            for c in clip.pcm.chunks(640) {
                assert!(core.push_audio(c));
            }
            let r = core.commit();
            let (e, w) = long_clips::wer_counts(&clip.reference, &r.asr_text);
            eprintln!(
                "{:.1} s hotwords={biased}: {:?} asr {} ms WER {:.1}%",
                clip.secs(),
                r.status,
                r.timings.asr_ms,
                100.0 * e as f32 / w as f32
            );
            assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
            assert!(e * 10 < w, "WER {e}/{w}: {}", r.asr_text);
        }
    }
}

/// D22 with the real engine: the probe's 32-62 s clips dictated with
/// `segment_pause_ms` (pushed at once; the closed pieces decode at
/// commit, which gives the same text as decoding them while recording).
/// No sentence may go missing (survey: one-shot decodes dropped 22-29
/// words at 43-56 s).
#[test]
fn parakeet_segmented_long_clips_lose_no_sentence() {
    let Some(core) = load() else { return };
    let Some(clips) = long_clips::audio_dir().and_then(|d| long_clips::build(&d)) else {
        eprintln!("SKIP: FLOW_AUDIO_DIR with the manifest clips not set");
        return;
    };
    for clip in clips.iter().filter(|c| c.target_s >= 32) {
        core.start_session(SessionConfig {
            ring_capacity_samples: clip.pcm.len() + 16_000,
            segment_pause_ms: Some(800),
            ..Default::default()
        })
        .unwrap();
        for c in clip.pcm.chunks(640) {
            assert!(core.push_audio(c));
        }
        let r = core.commit();
        let (e, n) = long_clips::wer_counts(&clip.reference, &r.asr_text);
        eprintln!(
            "{:.1} s segmented: {} pieces, asr {} ms, WER {:.1}%",
            clip.secs(),
            r.timings.segments,
            r.timings.asr_ms,
            100.0 * e as f32 / n as f32
        );
        assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
        assert!(r.timings.segments >= 2, "{:?}", r.timings);
        assert!(
            e * 20 < n,
            "{:.1} s: WER {e}/{n}: {}",
            clip.secs(),
            r.asr_text
        );
    }
}

/// Regression (agent W, feat/win-streaming 64d8e6b): pieces were
/// punctuated on their own, so jfk's mid-sentence pause became "ask not.
/// What your country". Segmented output must equal the whole-clip one,
/// punctuation and case included.
#[test]
fn parakeet_segmented_jfk_equals_whole_clip() {
    let Some(core) = load() else { return };
    let whole = dictate(&core, &models().join("jfk.wav"), SessionConfig::default());
    let seg = dictate(
        &core,
        &models().join("jfk.wav"),
        SessionConfig {
            segment_pause_ms: Some(800),
            ..Default::default()
        },
    );
    assert!(seg.timings.segments >= 2, "no cut: {:?}", seg.timings);
    assert_eq!(seg.asr_text, whole.asr_text);
    assert_eq!(seg.final_text, whole.final_text);
}

/// Words after a sentence end that start lower case (abbreviations and
/// ellipses aside).
fn lowercase_after_stop(t: &str) -> Vec<String> {
    let w: Vec<&str> = t.split_whitespace().collect();
    w.windows(2)
        .filter(|p| {
            let a = p[0].to_lowercase();
            a.ends_with(['.', '?', '!'])
                && !a.ends_with("..")
                && !["mr.", "mrs.", "dr.", "st.", "e.g.", "i.e."].contains(&a.as_str())
                && p[1].starts_with(char::is_lowercase)
        })
        .map(|p| p.join(" "))
        .collect()
}

/// Regression (cliptest 2026-10-23, Parakeet-unified int8 at 800 ms):
/// "capacity. the Englishman" (long_028s; a bridge's edge word),
/// "figures. that is" (ls_1188_M; the splice was refused), the dropped
/// period in "tender flowers the pain" (long_040s; a 4 s left bridge
/// heard no sentence end), and dictations ending with no period. Every
/// sentence end in the segmented output is followed by a capital, and
/// `final_text` is closed. With the unified model the long_040s break
/// must survive the cut.
#[test]
fn parakeet_segmented_joins_keep_sentence_case_and_close() {
    let Some(core) = load() else { return };
    let Some(audio) = long_clips::audio_dir() else {
        eprintln!("SKIP: FLOW_AUDIO_DIR not set");
        return;
    };
    let Some(clips) = long_clips::build(&audio) else {
        return;
    };
    let mut set: Vec<(String, Vec<i16>)> = clips
        .into_iter()
        .filter(|c| [10, 28, 40].contains(&c.target_s))
        .map(|c| (format!("long_{}s", c.target_s), c.pcm))
        .collect();
    if let Ok(p) = audio::read_wav_16k_mono(&audio.join("1188-133604-0019.wav")) {
        set.push(("ls_1188_M".into(), p));
    }
    let unified = std::env::var("FLOW_PARAKEET_DIR").is_ok_and(|d| d.contains("unified"));
    for (name, pcm) in &set {
        core.start_session(SessionConfig {
            ring_capacity_samples: pcm.len() + 16_000,
            segment_pause_ms: Some(800),
            ..Default::default()
        })
        .unwrap();
        for c in pcm.chunks(640) {
            assert!(core.push_audio(c));
        }
        let r = core.commit();
        eprintln!("{name}: {} pieces: {:?}", r.timings.segments, r.final_text);
        assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
        assert!(r.timings.segments >= 2, "{name}: no cut: {:?}", r.timings);
        assert!(
            lowercase_after_stop(&r.final_text).is_empty(),
            "{name}: {:?} in {:?}",
            lowercase_after_stop(&r.final_text),
            r.final_text
        );
        assert!(
            r.final_text.trim_end().ends_with(['.', '?', '!']),
            "{name}: open ending: {:?}",
            r.final_text
        );
        if unified && name == "long_40s" {
            assert!(
                r.final_text.contains("flowers. The pain"),
                "{:?}",
                r.final_text
            );
        }
    }
}
