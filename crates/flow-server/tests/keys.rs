//! Named keys (D38): migration from the single token, revoke on the next
//! connection, hot reload of CLI changes, usage stats across a restart,
//! and the `flow-server keys` CLI.

mod common;

use std::path::Path;
use std::process::Command;

use common::*;
use flow_core::config::SessionConfig;
use flow_protocol::{ClientMsg, ServerMsg};
use flow_server::keys::{KeyStore, KEYS_FILE, OWNER, USAGE_FILE};
use flow_server::Config;

/// A config as an upgraded install has it: `token_file` + a state dir.
fn file_cfg(dir: &Path) -> Config {
    let token = dir.join("token");
    if !token.exists() {
        std::fs::write(&token, format!("{TOKEN}\n")).unwrap();
    }
    let mut c = mock_cfg("hello");
    c.token = None;
    c.token_file = Some(token);
    c.state_dir = Some(dir.join("state"));
    c
}

/// The CLI is another process with its own store on the same directory.
fn cli(dir: &Path) -> KeyStore {
    KeyStore::open(&dir.join("state"), None).unwrap()
}

async fn dictate(ws: &mut Ws, sid: u64, ms: usize) {
    send(
        ws,
        &ClientMsg::Start {
            sid,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert!(matches!(recv(ws).await, Some(ServerMsg::Started { .. })));
    send_audio(ws, &speech(ms)).await;
    send(ws, &ClientMsg::Commit { sid }).await;
    match recv_final(ws).await {
        Some(ServerMsg::Result { sid: got, .. }) => assert_eq!(got, sid),
        other => panic!("expected a result, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn old_token_becomes_admin_owner_and_unchanged_client_connects() {
    let d = tempfile::tempdir().unwrap();
    let s = start(file_cfg(d.path())).await;
    // The client still sends the old token, exactly as before.
    let mut ws = session(s.addr()).await;
    dictate(&mut ws, 1, 500).await;
    assert_eq!(http_get(s.addr(), "/v1/health", Some(TOKEN)).await.0, 200);
    let keys = cli(d.path()).list();
    assert_eq!(keys.len(), 1);
    assert_eq!(
        (keys[0].key.name.as_str(), keys[0].key.admin),
        (OWNER, true)
    );
    let body = std::fs::read_to_string(d.path().join("state").join(KEYS_FILE)).unwrap();
    assert!(!body.contains(TOKEN), "only the hash is stored: {body}");
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_added_key_works_without_restart_and_revoke_hits_the_next_connection() {
    let d = tempfile::tempdir().unwrap();
    let s = start(file_cfg(d.path())).await;
    let (_, secret) = cli(d.path()).add("phone", false).unwrap();
    // Hot reload: the running server knows the new key at once.
    let mut open = connect(s.addr(), Some(&secret)).await.unwrap();
    hello(&mut open).await;
    assert_eq!(http_get(s.addr(), "/v1/health", Some(&secret)).await.0, 200);

    cli(d.path()).revoke("phone", false).unwrap();
    // Next connection and health: 401.
    assert!(connect(s.addr(), Some(&secret)).await.is_err());
    assert_eq!(http_get(s.addr(), "/v1/health", Some(&secret)).await.0, 401);
    // The open session is not killed: it still dictates.
    dictate(&mut open, 1, 300).await;
    // Other keys are untouched.
    assert!(connect(s.addr(), Some(TOKEN)).await.is_ok());
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_stats_update_per_dictation_and_survive_a_restart() {
    let d = tempfile::tempdir().unwrap();
    let s = start(file_cfg(d.path())).await;
    let mut ws = session(s.addr()).await;
    dictate(&mut ws, 1, 1000).await;
    dictate(&mut ws, 2, 500).await;
    let id = cli(d.path()).list()[0].key.id.clone();
    let live = s.keys().unwrap().list();
    assert_eq!(live[0].usage.dictations, 2);
    drop(ws);
    s.kill(); // flushes
    assert!(d.path().join("state").join(USAGE_FILE).is_file());

    let s = start(file_cfg(d.path())).await;
    let k = &s.keys().unwrap().list()[0];
    assert_eq!(k.key.id, id, "same key after the restart");
    assert_eq!((k.usage.dictations, k.usage.audio_ms), (2, 1500));
    assert_eq!(k.usage.client.as_deref(), Some("test"));
    assert!(k.usage.last_seen.is_some());
    let mut ws = session(s.addr()).await;
    dictate(&mut ws, 1, 250).await;
    s.flush_usage();
    let k = &cli(d.path()).list()[0];
    assert_eq!((k.usage.dictations, k.usage.audio_ms), (3, 1750));
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_file_without_token_file_is_enough() {
    let d = tempfile::tempdir().unwrap();
    let (_, secret) = cli(d.path()).add("laptop", true).unwrap();
    let mut c = file_cfg(d.path());
    c.token_file = None;
    let s = start(c).await;
    assert!(connect(s.addr(), Some(&secret)).await.is_ok());
    assert!(connect(s.addr(), Some(TOKEN)).await.is_err());
    s.kill();
}

fn run(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_flow-server"))
        .args(["--config"])
        .arg(dir.join("config.toml"))
        .args(args)
        .env_remove("STATE_DIRECTORY")
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn keys_cli_add_list_revoke() {
    let d = tempfile::tempdir().unwrap();
    let token = d.path().join("token");
    std::fs::write(&token, format!("{TOKEN}\n")).unwrap();
    let state = d.path().join("state");
    std::fs::write(
        d.path().join("config.toml"),
        format!("bind = [\"127.0.0.1:0\"]\ntoken_file = {token:?}\nstate_dir = {state:?}\n"),
    )
    .unwrap();

    let (ok, secret, err) = run(d.path(), &["keys", "add", "phone"]);
    assert!(ok, "{err}");
    let secret = secret.trim();
    assert_eq!(secret.len(), 43, "stdout is just the secret");
    assert!(!err.contains(secret), "printed once: {err}");
    let (ok, _, err) = run(d.path(), &["keys", "add", "phone"]);
    assert!(!ok && err.contains("already exists"), "{err}");
    let (ok, _, _) = run(d.path(), &["keys", "add", "desk", "--admin"]);
    assert!(ok);

    let (ok, table, _) = run(d.path(), &["keys", "list"]);
    assert!(ok);
    assert_eq!(
        table.lines().count(),
        4,
        "header + owner + phone + desk: {table}"
    );
    assert!(table.contains(OWNER) && table.contains("phone") && table.contains("desk"));
    assert!(!table.contains(secret));
    let body = std::fs::read_to_string(state.join(KEYS_FILE)).unwrap();
    assert!(!body.contains(secret) && !body.contains(TOKEN), "{body}");

    let (ok, _, err) = run(d.path(), &["keys", "revoke", "phone"]);
    assert!(ok, "{err}");
    let (_, table, _) = run(d.path(), &["keys", "list"]);
    let phone = table.lines().find(|l| l.starts_with("phone")).unwrap();
    assert!(phone.contains("revoked"), "{table}");
    let (ok, _, err) = run(d.path(), &["keys", "revoke", "phone"]);
    assert!(!ok && err.contains("no active key"), "{err}");
    // Last-admin guard: desk goes (owner remains), then owner is the
    // last admin and needs --force.
    let (ok, _, err) = run(d.path(), &["keys", "revoke", "desk"]);
    assert!(ok, "{err}");
    let (ok, _, err) = run(d.path(), &["keys", "revoke", OWNER]);
    assert!(
        !ok && err.contains("last admin") && err.contains("--force"),
        "{err}"
    );
    let (ok, _, err) = run(d.path(), &["keys", "revoke", OWNER, "--force"]);
    assert!(ok, "{err}");
    let (ok, _, _) = run(d.path(), &["keys", "frobnicate"]);
    assert!(!ok);
    // gen-token still works (it writes a token file; the owner key follows it).
    let (ok, _, err) = run(
        d.path(),
        &["gen-token", "--out", d.path().join("t2").to_str().unwrap()],
    );
    assert!(ok, "{err}");
}
