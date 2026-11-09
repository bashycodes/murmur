//! `[debug] capture`: off writes nothing; on saves the session's WAV and one
//! JSON line (transcript, cleaned text, timings, route) with private modes,
//! and never the token.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::*;
use murmur_core::config::{FieldContext, SessionConfig};
use murmur_protocol::{ClientMsg, ServerMsg};

const SAID: &str = "replay me please";
const CTX_SECRET: &str = "marmalade-invoice";
const FAKE_KEY: &str = "sk-or-v1-0000fakeapikey0000";

async fn one_session(dir: &Path, capture: bool) {
    let mut cfg = mock_cfg(SAID);
    cfg.debug.capture = capture;
    cfg.debug.dir = Some(dir.to_path_buf());
    let h = start(cfg).await;
    let mut ws = session(h.addr()).await;
    let config = SessionConfig {
        dictionary: serde_json::from_str(&format!(r#"[{{"word":"{FAKE_KEY}"}}]"#)).unwrap(),
        ..Default::default()
    };
    send(&mut ws, &ClientMsg::Start { sid: 1, config }).await;
    recv(&mut ws).await;
    send(
        &mut ws,
        &ClientMsg::Context {
            sid: 1,
            ctx: FieldContext {
                before: CTX_SECRET.into(),
                ..Default::default()
            },
        },
    )
    .await;
    send_audio(&mut ws, &speech(500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    let Some(ServerMsg::Result { result, .. }) = recv_final(&mut ws).await else {
        panic!("no result")
    };
    assert!(
        result.final_text.to_lowercase().contains("replay"),
        "{result:?}"
    );
    // The save runs after the result is sent.
    let t0 = Instant::now();
    while capture && !dir.join("dictations.jsonl").exists() && t0.elapsed() < Duration::from_secs(5)
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_off_writes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("debug");
    one_session(&dir, false).await;
    assert!(!dir.exists(), "capture off created {}", dir.display());
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_on_writes_wav_and_a_json_line_without_secrets() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("debug");
    one_session(&dir, true).await;
    let wavs: Vec<String> = std::fs::read_dir(dir.join("audio"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(wavs.len(), 1, "{wavs:?}");
    assert!(
        wavs[0].ends_with("Z.wav") && !wavs[0].contains(':'),
        "{wavs:?}"
    );
    let wav = std::fs::read(dir.join("audio").join(&wavs[0])).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(wav.len(), 44 + 500 * 16 * 2, "every received sample");

    let log = std::fs::read_to_string(dir.join("dictations.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    let l = &lines[0];
    assert_eq!(l["audio"], wavs[0].as_str());
    assert_eq!(l["route"], "server");
    assert_eq!(l["engine"], "mock");
    assert!(
        l["raw"].as_str().unwrap().to_lowercase().contains("replay"),
        "{l}"
    );
    assert!(
        l["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("replay"),
        "{l}"
    );
    assert!(l["total_ms"].is_u64() && l["audio_ms"].is_u64(), "{l}");
    assert_eq!(
        l["ts"].as_str().unwrap().replace(':', "-") + ".wav",
        wavs[0]
    );

    // Never the token / client auth, an API key, the context or dictionary.
    for secret in [TOKEN, "Bearer", "Authorization", FAKE_KEY, CTX_SECRET] {
        assert!(!log.contains(secret), "{secret} reached the capture: {log}");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("audio")), 0o700);
        assert_eq!(mode(&dir.join("dictations.jsonl")), 0o600);
        assert_eq!(mode(&dir.join("audio").join(&wavs[0])), 0o600);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_prunes_old_files_at_start() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("debug");
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    let old = dir.join("audio").join("2026-01-01T00-00-00.000Z.wav");
    std::fs::write(&old, b"RIFF").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(80 * 3600))
        .unwrap();
    one_session(&dir, true).await;
    assert!(
        !old.exists(),
        "a WAV older than max_age_h survived the start"
    );
    assert_eq!(std::fs::read_dir(dir.join("audio")).unwrap().count(), 1);
}
