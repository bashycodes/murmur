//! Long test dictations built from LibriSpeech clips, exactly as the
//! 2026-10-23 probe builds them (`tools/probes/prototypes/
//! segmented_asr_2026_10_23.py::build_clips`): consecutive utterances of
//! `tools/probes/bench/asr_manifest_extended.json` that exist in the audio
//! dir, joined with 700 ms of digital silence until each target length is
//! reached, targets consumed in order. `long(40)` is the 41.7 s clip on
//! which Parakeet's beam search returned nothing.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const TARGETS: [usize; 9] = [5, 10, 15, 20, 25, 28, 32, 40, 60];
pub const GAP_MS: usize = 700;

pub struct Clip {
    pub target_s: usize,
    pub pcm: Vec<i16>,
    pub reference: String,
}

impl Clip {
    pub fn secs(&self) -> f32 {
        self.pcm.len() as f32 / 16_000.0
    }
}

pub fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/probes/bench/asr_manifest_extended.json")
}

/// `FLOW_AUDIO_DIR` (LibriSpeech WAVs named `<id>.wav`).
pub fn audio_dir() -> Option<PathBuf> {
    std::env::var_os("FLOW_AUDIO_DIR").map(PathBuf::from)
}

/// All probe clips, or `None` (with a SKIP note) when the audio is missing.
pub fn build(audio: &Path) -> Option<Vec<Clip>> {
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest()).ok()?).ok()?;
    let utts: Vec<(PathBuf, String)> = m["utterances"]
        .as_array()?
        .iter()
        .filter_map(|u| {
            let p = audio.join(format!("{}.wav", u["id"].as_str()?));
            p.is_file()
                .then(|| (p, u["ref"].as_str().unwrap_or("").to_string()))
        })
        .collect();
    if utts.len() < 10 {
        eprintln!(
            "SKIP: only {} manifest clips in {}",
            utts.len(),
            audio.display()
        );
        return None;
    }
    let gap = vec![0i16; 16 * GAP_MS];
    let mut out = Vec::new();
    let mut i = 0;
    for t in TARGETS {
        let (mut pcm, mut refs) = (Vec::new(), Vec::new());
        while pcm.len() < t * 16_000 {
            let (p, r) = &utts[i % utts.len()];
            i += 1;
            if !pcm.is_empty() {
                pcm.extend_from_slice(&gap);
            }
            pcm.extend(read_wav_i16(p));
            refs.push(r.clone());
        }
        out.push(Clip {
            target_s: t,
            pcm,
            reference: refs.join(" "),
        });
    }
    Some(out)
}

fn read_wav_i16(p: &Path) -> Vec<i16> {
    flow_core::audio::read_wav_16k_mono(p).expect("16 kHz mono wav")
}

fn norm(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// (word errors, reference words), lowercase, punctuation dropped.
pub fn wer_counts(reference: &str, hyp: &str) -> (usize, usize) {
    let (r, h) = (norm(reference), norm(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(rw != hw))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    (prev[h.len()], r.len())
}

/// The STT survey's long clips (docs/proposals/stt-model-survey.md): the
/// first `n` manifest utterances joined with 300 ms of silence. n = 3 is
/// the 25.1 s clip and n = 4 the 42.8 s clip on which Parakeet v2 beam 4
/// returned "" (and greedy dropped a 22-word utterance at 42.8 s).
pub fn survey(audio: &Path, n: usize) -> Option<Clip> {
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest()).ok()?).ok()?;
    let mut pcm = Vec::new();
    let mut refs = Vec::new();
    for u in m["utterances"].as_array()?.iter().take(n) {
        let p = audio.join(format!("{}.wav", u["id"].as_str()?));
        if !p.is_file() {
            eprintln!("SKIP: {} missing", p.display());
            return None;
        }
        if !pcm.is_empty() {
            pcm.extend(std::iter::repeat_n(0i16, 16 * 300));
        }
        pcm.extend(read_wav_i16(&p));
        refs.push(u["ref"].as_str().unwrap_or("").to_string());
    }
    Some(Clip {
        target_s: pcm.len() / 16_000,
        pcm,
        reference: refs.join(" "),
    })
}
