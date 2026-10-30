//! Per-language models on the server (D33, `[model.languages.<code>]`): a
//! session whose single forced language has a model decodes on it, the
//! rest on the default model, and `hello_ok.languages` lists the keys.
//!
//! The real-model test (feature `parakeet`) runs the streaming Hebrew
//! zipformer on a FLEURS clip and skips (passes with a note) unless both
//! exist: FLOW_ZIPFORMER_HE_DIR (default models/zipformer2-66m-streaming-
//! he-int8, then ~/.cache/flow-clone-models/zipformer2-66m-streaming-he-int8;
//! fetch with `tools/fetch-models.sh --hebrew`) and FLOW_HE_WAV (default
//! models/he.wav, then the FLEURS he dev clip 10012360550181770926.wav under
//! ~/.cache/flow-clone-models/fleurs-he-sample/raw/). Model pin:
//! docs/findings/2026-10-30-hebrew-model-pins.md.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::*;
use flow_core::config::{AsrBackend, LanguageModel, SessionConfig};
use flow_core::CommitResult;
use flow_protocol::{ClientMsg, ServerMsg};
use flow_server::Config;

fn with_language(mut cfg: Config, lang: &str, m: LanguageModel) -> Config {
    cfg.model.languages = BTreeMap::from([(lang.to_string(), m)]);
    cfg
}

fn forced(lang: &str) -> SessionConfig {
    SessionConfig {
        languages: vec![lang.into()],
        ..Default::default()
    }
}

/// One dictation of `pcm` on a fresh session; the result (waits up to
/// `wait` for it: a real decode on a shared box can exceed `recv`'s 5 s).
async fn dictate(
    ws: &mut Ws,
    sid: u64,
    cfg: SessionConfig,
    pcm: &[i16],
    wait: Duration,
) -> CommitResult {
    send(ws, &ClientMsg::Start { sid, config: cfg }).await;
    match recv(ws).await {
        Some(ServerMsg::Started { sid: s, .. }) => assert_eq!(s, sid),
        other => panic!("expected started, got {other:?}"),
    }
    send_audio(ws, pcm).await;
    send(ws, &ClientMsg::Commit { sid }).await;
    let r = tokio::time::timeout(wait, async {
        loop {
            match recv_final(ws).await {
                Some(ServerMsg::Result { sid: s, result, .. }) if s == sid => return *result,
                Some(ServerMsg::Result { .. }) => continue,
                other => panic!("expected result, got {other:?}"),
            }
        }
    })
    .await
    .expect("result in time");
    eprintln!(
        "{:?} [{:?}] asr {} ms / audio {} ms: {}",
        r.status, r.language, r.timings.asr_ms, r.timings.audio_ms, r.final_text
    );
    r
}

#[tokio::test(flavor = "multi_thread")]
async fn forced_language_routes_to_its_model() {
    let cfg = with_language(
        mock_cfg("english words"),
        "he",
        LanguageModel {
            asr_backend: AsrBackend::Mock {
                text: "שלום עולם".into(),
            },
            asr_path: String::new(),
            ..Default::default()
        },
    );
    let s = start(cfg).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let h = hello(&mut ws).await;
    assert_eq!(h.languages, vec!["he".to_string()]);
    assert_eq!(
        h.asr_backend, "mock",
        "the default model stays the headline"
    );
    let wait = Duration::from_secs(5);
    for (sid, lang) in [(1, "he"), (2, "he-IL"), (3, "iw")] {
        let r = dictate(&mut ws, sid, forced(lang), &speech(1000), wait).await;
        assert!(r.asr_text.contains("שלום עולם"), "{lang}: {}", r.asr_text);
    }
    for (sid, cfg) in [
        (4, SessionConfig::default()),
        (5, forced("auto")),
        (6, forced("en")),
        (7, forced("fr")),
    ] {
        let r = dictate(&mut ws, sid, cfg, &speech(1000), wait).await;
        assert_eq!(r.asr_text, "english words", "sid {sid}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn no_language_models_omits_the_field() {
    let s = start(mock_cfg("hi")).await;
    let (status, body) = http_get(s.addr(), "/v1/health", Some(TOKEN)).await;
    assert_eq!(status, 200);
    assert!(!body.contains("\"languages\""), "{body}");
}

#[cfg(feature = "parakeet")]
mod zipformer {
    use std::path::{Path, PathBuf};

    use super::*;

    fn models() -> PathBuf {
        std::env::var_os("FLOW_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
    }

    fn cache() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/flow-clone-models"))
    }

    /// The env var, else the first candidate that exists.
    fn pick(var: &str, candidates: Vec<PathBuf>, ok: impl Fn(&Path) -> bool) -> Option<PathBuf> {
        if let Some(p) = std::env::var_os(var) {
            return Some(PathBuf::from(p));
        }
        candidates.into_iter().find(|p| ok(p))
    }

    fn inputs() -> Option<(PathBuf, PathBuf)> {
        let zip = "zipformer2-66m-streaming-he-int8";
        let dir = pick(
            "FLOW_ZIPFORMER_HE_DIR",
            [Some(models().join(zip)), cache().map(|c| c.join(zip))]
                .into_iter()
                .flatten()
                .collect(),
            |p| p.join("tokens.txt").is_file(),
        );
        let wav = pick(
            "FLOW_HE_WAV",
            [
                Some(models().join("he.wav")),
                cache().map(|c| c.join("fleurs-he-sample/raw/10012360550181770926.wav")),
            ]
            .into_iter()
            .flatten()
            .collect(),
            |p| p.is_file(),
        );
        match (dir, wav) {
            (Some(d), Some(w)) if d.join("tokens.txt").is_file() && w.is_file() => Some((d, w)),
            (d, w) => {
                eprintln!("SKIP: zipformer dir {d:?} or Hebrew wav {w:?} missing");
                None
            }
        }
    }

    /// The clip scaled to peak 0.5: some FLEURS he clips are recorded
    /// below the engines' silence gate; dictation audio is louder.
    fn clip(wav: &Path) -> Vec<i16> {
        let pcm = flow_core::audio::read_wav_16k_mono(wav).unwrap();
        let peak = pcm
            .iter()
            .map(|s| (*s as i32).abs())
            .max()
            .unwrap_or(1)
            .max(1) as f32;
        pcm.iter()
            .map(|s| (*s as f32 * 16_384.0 / peak) as i16)
            .collect()
    }

    fn assert_hebrew(text: &str) {
        let letters = text.chars().filter(|c| c.is_alphabetic()).count();
        let he = text
            .chars()
            .filter(|c| ('\u{05D0}'..='\u{05EA}').contains(c))
            .count();
        assert!(letters > 10 && he * 10 >= letters * 9, "not Hebrew: {text}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hebrew_session_decodes_on_the_zipformer() {
        let Some((dir, wav)) = inputs() else { return };
        // Same shape as `[model.languages.he]` in config.example.toml.
        let toml = format!(
            "[model]\nasr_engine = {{ mock = {{ text = \"english words\" }} }}\nthreads = 4\n\
             [model.languages.he]\nasr_backend = \"sherpa_online\"\nasr_path = {:?}\n",
            dir.display().to_string()
        );
        let mut cfg = Config::from_toml(&toml).unwrap();
        let base = mock_cfg("");
        cfg.bind = base.bind;
        cfg.token = base.token;
        let s = start(cfg).await;
        let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
        assert_eq!(hello(&mut ws).await.languages, vec!["he".to_string()]);
        let pcm = clip(&wav);
        let wait = Duration::from_secs(60);

        let r = dictate(&mut ws, 1, forced("he-IL"), &pcm, wait).await;
        assert_ne!(r.status, flow_core::CommitStatus::Error, "{r:?}");
        assert_ne!(r.status, flow_core::CommitStatus::Empty, "{r:?}");
        assert_eq!(r.language.as_deref(), Some("he"));
        assert_hebrew(&r.asr_text);
        assert_hebrew(&r.final_text);

        // Auto-detect stays on the default (English-first) model.
        let e = dictate(&mut ws, 2, SessionConfig::default(), &pcm, wait).await;
        assert_eq!(e.asr_text, "english words");
    }
}
