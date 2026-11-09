//! D41 on Android: murmur-ffi's JSON model-library API against a real
//! in-process murmur-server: list, verified download, resume from `.part`,
//! a pin that differs from the server's copy, cancel, a wrong key.
#![cfg(feature = "library")]

mod support;

use murmur_ffi::library_api::{download, list};
use serde_json::{json, Value};
use support::library_server::{sha, weights, LibraryServer, TOKEN};

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn never() -> bool {
    false
}

fn file_json(sha256: &str) -> String {
    json!({"name": "ggml-custom.bin", "size": 3_000_000, "sha256": sha256}).to_string()
}

#[test]
fn lists_the_library() {
    let s = LibraryServer::start();
    let r = parse(&list(&s.server_json(TOKEN)));
    assert_eq!(r["ok"], true, "{r}");
    let m = &r["models"][0];
    assert_eq!(m["id"], "my-whisper");
    assert_eq!(m["kind"], "stt");
    assert_eq!(m["engine"], "whisper");
    assert_eq!(m["files"][0]["name"], "ggml-custom.bin");
    assert_eq!(m["files"][0]["size"], 3_000_000);
    assert_eq!(m["files"][0]["sha256"], sha(&weights()));
    assert!(!r.to_string().contains(TOKEN));
}

#[test]
fn downloads_verifies_and_resumes() {
    let s = LibraryServer::start();
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("models/ggml-custom.bin");
    let part = d.path().join("models/ggml-custom.bin.part");
    let (dest_s, part_s) = (dest.to_str().unwrap(), part.to_str().unwrap());
    let w = weights();
    // What an interrupted download left behind.
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, &w[..1_000_000]).unwrap();
    let mut seen = Vec::new();
    let r = parse(&download(
        &s.server_json(TOKEN),
        "my-whisper",
        &file_json(&sha(&w)),
        dest_s,
        part_s,
        &mut |n| seen.push(n),
        &never,
    ));
    assert_eq!(r, json!({"ok": true}));
    assert_eq!(std::fs::read(&dest).unwrap(), w);
    assert!(!part.exists());
    assert_eq!(seen.first(), Some(&1_000_000), "resumed at the prefix");
    assert_eq!(seen.last(), Some(&3_000_000));
}

#[test]
fn a_pin_the_server_copy_does_not_match_is_refused() {
    let s = LibraryServer::start();
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("x.bin");
    let part = d.path().join("x.bin.part");
    let r = parse(&download(
        &s.server_json(TOKEN),
        "my-whisper",
        &file_json(&"0".repeat(64)),
        dest.to_str().unwrap(),
        part.to_str().unwrap(),
        &mut |_| {},
        &never,
    ));
    assert_eq!(r["error"], "mismatch", "{r}");
    assert!(!dest.exists() && !part.exists());
}

#[test]
fn cancel_keeps_nothing_and_a_wrong_key_is_auth() {
    let s = LibraryServer::start();
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("x.bin");
    let part = d.path().join("x.bin.part");
    let p2 = part.clone();
    let r = parse(&download(
        &s.server_json(TOKEN),
        "my-whisper",
        &file_json(&sha(&weights())),
        dest.to_str().unwrap(),
        part.to_str().unwrap(),
        &mut |_| {},
        &move || std::fs::metadata(&p2).is_ok_and(|m| m.len() > 0),
    ));
    assert_eq!(r["error"], "cancelled", "{r}");
    assert!(!dest.exists() && !part.exists());
    let r = parse(&list(&s.server_json("wrong-token-0123456789")));
    assert_eq!(r["error"], "auth", "{r}");
}

#[test]
fn a_server_that_is_gone_is_a_network_error() {
    let port = {
        let s = LibraryServer::start();
        s.port()
    };
    let r = parse(&list(
        &json!({"host": "127.0.0.1", "port": port, "tls": false, "token": TOKEN, "connect_timeout_ms": 1000}).to_string(),
    ));
    assert_eq!(r["error"], "network", "{r}");
}
