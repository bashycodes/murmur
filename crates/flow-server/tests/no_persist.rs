//! I6: the server writes nothing to disk and logs no text by default.
//! Runs the real binary with HOME / TMPDIR / cwd pointed at an empty
//! directory, debug logging on, and a mock transcript that must not leak.

mod common;

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use flow_core::config::SessionConfig;
use flow_protocol::{ClientMsg, ServerMsg};

const SECRET: &str = "zebra quartz confidential phrase";

#[tokio::test(flavor = "multi_thread")]
async fn a_session_leaves_no_files_and_no_text_in_the_log() {
    let state = tempfile::tempdir().unwrap();
    let conf = tempfile::tempdir().unwrap();
    let cfg_path = conf.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        format!(
            "bind = [\"127.0.0.1:0\"]\nauth = \"none\"\n[model]\nasr_engine = {{ mock = {{ text = \"{SECRET}\" }} }}\n[log]\nlevel = \"debug\"\n"
        ),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_flow-server"))
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(state.path())
        .env("HOME", state.path())
        .env("TMPDIR", state.path())
        .env_remove("RUST_LOG")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
    let addr: std::net::SocketAddr = addr.parse().unwrap();

    let mut ws = connect(addr, None).await.unwrap();
    hello(&mut ws).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    recv(&mut ws).await;
    send_audio(&mut ws, &speech(1000)).await;
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
    let mut rest = String::new();
    err.read_to_string(&mut rest).unwrap();
    log.push_str(&rest);
    assert!(status.success(), "clean shutdown on SIGTERM: {log}");

    assert!(log.contains("result"), "session was logged: {log}");
    assert!(
        !log.contains("zebra"),
        "transcript leaked into the log:\n{log}"
    );
    let left: Vec<_> = std::fs::read_dir(state.path()).unwrap().collect();
    assert!(left.is_empty(), "server wrote files: {left:?}");
}
