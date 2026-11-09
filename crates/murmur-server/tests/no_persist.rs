//! I6: the server writes no audio or text to disk and logs no text by
//! default. The only files it writes are the key store's (D38: key hashes
//! and per-key counters, in the state dir). Runs the real binary with HOME / TMPDIR / cwd pointed at an empty
//! directory and a mock transcript that must not leak. S9: the same holds at
//! `RUST_LOG=trace`, where dependencies (tungstenite logs every received
//! text frame) would otherwise print dictionaries, contexts and partials.

mod common;

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use flow_core::config::{FieldContext, SessionConfig};
use flow_protocol::{ClientMsg, ServerMsg};

const SECRET: &str = "zebra quartz confidential phrase";
const DICT_SECRET: &str = "xylophonist";
const CTX_SECRET: &str = "marmalade-invoice";

struct Run {
    log: String,
    files_left: usize,
}

/// One full session (start with a dictionary, context, partials, audio,
/// commit) against the real binary; returns its stderr.
async fn run_session(rust_log: Option<&str>, level: &str, log_text: bool) -> Run {
    let state = tempfile::tempdir().unwrap();
    let conf = tempfile::tempdir().unwrap();
    let cfg_path = conf.path().join("config.toml");
    let token_path = conf.path().join("token");
    std::fs::write(&token_path, format!("{TOKEN}\n")).unwrap();
    std::fs::write(
        &cfg_path,
        format!(
            "bind = [\"127.0.0.1:0\"]\ntoken_file = {token_path:?}\n[model]\nasr_engine = {{ mock = {{ text = \"{SECRET}\" }} }}\n[log]\nlevel = \"{level}\"\nlog_text = {log_text}\n"
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_flow-server"));
    cmd.arg("--config")
        .arg(&cfg_path)
        .current_dir(state.path())
        .env("HOME", state.path())
        .env("TMPDIR", state.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    match rust_log {
        Some(v) => cmd.env("RUST_LOG", v),
        None => cmd.env_remove("RUST_LOG"),
    };
    let mut child = cmd.spawn().unwrap();
    let mut err = BufReader::new(child.stderr.take().unwrap());
    let mut log = String::new();
    let t0 = Instant::now();
    let addr = loop {
        let mut line = String::new();
        assert!(
            err.read_line(&mut line).unwrap() > 0,
            "server exited: {log}"
        );
        log.push_str(&line);
        if let Some(i) = line.find("listening=") {
            break line[i + 10..]
                .split_whitespace()
                .next()
                .unwrap()
                .to_string();
        }
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "no ready line: {log}"
        );
    };
    assert!(log.contains("ready"));
    // Drain stderr concurrently so a chatty trace log never blocks the
    // server on a full pipe.
    let reader = std::thread::spawn(move || {
        let mut rest = String::new();
        let _ = err.read_to_string(&mut rest);
        rest
    });
    let addr: std::net::SocketAddr = addr.parse().unwrap();

    let mut ws = session(addr).await;
    let config = SessionConfig {
        dictionary: serde_json::from_str(&format!(r#"[{{"word":"{DICT_SECRET}"}}]"#)).unwrap(),
        partial_interval_ms: Some(100),
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
    send_audio(&mut ws, &speech(1000)).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // let partials flow
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    let Some(ServerMsg::Result { result, .. }) = recv_final(&mut ws).await else {
        panic!()
    };
    assert!(result.final_text.contains("zebra"), "{result:?}");
    drop(ws);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    let status = child.wait().unwrap();
    log.push_str(&reader.join().unwrap());
    assert!(status.success(), "clean shutdown on SIGTERM: {log}");
    assert!(log.contains("result"), "session was logged: {log}");
    let files_left = files_besides_the_key_store(state.path());
    Run { log, files_left }
}

/// Files under `root` other than the key store's files, which must
/// hold no secret, transcript, dictionary or context.
fn files_besides_the_key_store(root: &std::path::Path) -> usize {
    let keys = root.join(".local/state/flow-server");
    let mut n = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.parent() == Some(keys.as_path())
                && ["keys.json", "key-usage.json", "keys.json.lock"]
                    .iter()
                    .any(|f| p.file_name().unwrap() == *f)
            {
                let body = std::fs::read_to_string(&p).unwrap();
                for s in [SECRET, "zebra", DICT_SECRET, CTX_SECRET, TOKEN] {
                    assert!(!body.contains(s), "{s} in {}: {body}", p.display());
                }
            } else {
                n += 1;
            }
        }
    }
    n
}

fn assert_no_secrets(log: &str, allow_transcript: bool) {
    let tail = |s: &str| s.chars().rev().take(4000).collect::<String>();
    if !allow_transcript {
        assert!(
            !log.contains("zebra"),
            "transcript leaked into the log:\n{}",
            tail(log)
        );
    }
    for s in [DICT_SECRET, CTX_SECRET, TOKEN] {
        assert!(!log.contains(s), "{s} leaked into the log:\n{}", tail(log));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_leaves_no_files_and_no_text_in_the_log() {
    let r = run_session(None, "debug", false).await;
    assert_no_secrets(&r.log, false);
    assert_eq!(r.files_left, 0, "server wrote files besides the key store");
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_log_trace_still_logs_no_text_audio_or_token() {
    let r = run_session(Some("trace"), "info", false).await;
    assert_no_secrets(&r.log, false);
    // The cap must not hide the server's own diagnostics.
    assert!(r.log.contains("connected"), "{}", r.log);
    let r = run_session(Some("trace,tungstenite=trace,hyper=trace"), "info", false).await;
    assert_no_secrets(&r.log, false);
    assert_eq!(r.files_left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn log_text_is_the_only_opt_in_and_reveals_only_the_result() {
    let r = run_session(Some("trace"), "debug", true).await;
    assert!(
        r.log.contains("zebra"),
        "log_text = true prints the result text"
    );
    assert!(r.log.contains("log_text = true"), "startup warns");
    assert_no_secrets(&r.log, true);
}
