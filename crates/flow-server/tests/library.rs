//! Model library (D40) over HTTP: auth, listing, downloads with Range,
//! traversal refused, per-key download counters, rescan without restart,
//! and the `flow-server library list` CLI.

mod common;

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use common::*;
use flow_protocol::library::{file_path, ModelKind, ModelList, PATH_MODELS};
use flow_server::{Config, ServerHandle};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WEIGHTS: &[u8] = b"0123456789abcdefghij";

fn put(p: &Path, body: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn lib_cfg(d: &Path) -> Config {
    let mut c = mock_cfg("hello");
    c.state_dir = Some(d.join("state"));
    c.library.dir = Some(d.join("lib"));
    c.library.rescan_s = 1;
    c
}

fn tag_whisper(d: &Path) {
    put(
        &d.join("lib/whisper-base/model.toml"),
        b"kind = \"stt\"\ntitle = \"Whisper base\"\ncatalogue_id = \"whisper-base\"\nengine = \"whisper\"\n",
    );
    put(&d.join("lib/whisper-base/ggml-base.bin"), WEIGHTS);
}

struct Resp {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }
}

async fn get(addr: SocketAddr, path: &str, token: Option<&str>, extra: &str) -> Resp {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: x\r\n{auth}{extra}Connection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    Resp {
        status: head[9..12].parse().unwrap(),
        head,
        body: out[split + 4..].to_vec(),
    }
}

async fn list(addr: SocketAddr) -> ModelList {
    let r = get(addr, PATH_MODELS, Some(TOKEN), "").await;
    assert_eq!(r.status, 200);
    serde_json::from_slice(&r.body).unwrap()
}

/// Wait (≤ 5 s) until the listing has `n` models.
async fn wait_models(s: &ServerHandle, n: usize) -> ModelList {
    for _ in 0..50 {
        let l = list(s.addr()).await;
        if l.models.len() == n {
            return l;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("library never reached {n} models");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_and_download_need_a_key() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    let s = start(lib_cfg(d.path())).await;
    wait_models(&s, 1).await;
    let f = file_path("whisper-base", "ggml-base.bin");
    for path in [PATH_MODELS, f.as_str()] {
        assert_eq!(get(s.addr(), path, None, "").await.status, 401, "{path}");
        assert_eq!(
            get(s.addr(), path, Some("wrong-token-0123456789"), "")
                .await
                .status,
            401
        );
    }
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn list_shows_tagged_models_with_hashes() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    put(&d.path().join("lib/untagged/x.bin"), b"x");
    let s = start(lib_cfg(d.path())).await;
    let l = wait_models(&s, 1).await;
    let m = &l.models[0];
    assert_eq!(m.id, "whisper-base");
    assert_eq!(m.title, "Whisper base");
    assert_eq!(m.kind, ModelKind::Stt);
    assert_eq!(m.catalogue_id.as_deref(), Some("whisper-base"));
    assert_eq!(m.engine.as_deref(), Some("whisper"));
    assert_eq!(m.files.len(), 1);
    assert_eq!(m.files[0].name, "ggml-base.bin");
    assert_eq!(m.files[0].size, WEIGHTS.len() as u64);
    let want: String = Sha256::digest(WEIGHTS)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(m.files[0].sha256, want);
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn download_full_and_ranges() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    let s = start(lib_cfg(d.path())).await;
    let sha = wait_models(&s, 1).await.models[0].files[0].sha256.clone();
    let f = file_path("whisper-base", "ggml-base.bin");
    let r = get(s.addr(), &f, Some(TOKEN), "").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, WEIGHTS);
    assert_eq!(r.header("accept-ranges"), Some("bytes"));
    assert_eq!(r.header("etag"), Some(format!("\"{sha}\"").as_str()));

    let r = get(s.addr(), &f, Some(TOKEN), "Range: bytes=5-9\r\n").await;
    assert_eq!(r.status, 206);
    assert_eq!(r.body, &WEIGHTS[5..10]);
    assert_eq!(r.header("content-range"), Some("bytes 5-9/20"));
    assert_eq!(r.header("content-length"), Some("5"));

    // Resume from an offset to the end.
    let r = get(s.addr(), &f, Some(TOKEN), "Range: bytes=15-\r\n").await;
    assert_eq!((r.status, r.body.as_slice()), (206, &WEIGHTS[15..]));
    let r = get(s.addr(), &f, Some(TOKEN), "Range: bytes=-3\r\n").await;
    assert_eq!((r.status, r.body.as_slice()), (206, &WEIGHTS[17..]));

    let r = get(s.addr(), &f, Some(TOKEN), "Range: bytes=20-\r\n").await;
    assert_eq!(r.status, 416);
    assert_eq!(r.header("content-range"), Some("bytes */20"));

    // If-Range with another validator: the whole file.
    let r = get(
        s.addr(),
        &f,
        Some(TOKEN),
        "Range: bytes=5-9\r\nIf-Range: \"other\"\r\n",
    )
    .await;
    assert_eq!((r.status, r.body.len()), (200, WEIGHTS.len()));
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn only_indexed_files_are_served() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    put(&d.path().join("lib/whisper-base/.secret"), b"hidden");
    put(&d.path().join("outside.txt"), b"outside");
    let s = start(lib_cfg(d.path())).await;
    wait_models(&s, 1).await;
    for path in [
        "/v1/models/whisper-base/files/model.toml",
        "/v1/models/whisper-base/files/.secret",
        "/v1/models/whisper-base/files/../../outside.txt",
        "/v1/models/whisper-base/files/%2e%2e/%2e%2e/outside.txt",
        "/v1/models/whisper-base/files/..%2f..%2foutside.txt",
        "/v1/models/..%2f..%2f/files/outside.txt",
        "/v1/models/nope/files/ggml-base.bin",
        "/v1/models/whisper-base/files//etc/passwd",
    ] {
        let r = get(s.addr(), path, Some(TOKEN), "").await;
        assert_ne!(r.status, 200, "{path}");
        assert!(
            !r.body.starts_with(b"outside") && r.body != b"hidden",
            "{path}"
        );
    }
    s.kill();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_file_swapped_for_a_symlink_after_the_scan_is_not_served() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    put(&d.path().join("outside.txt"), b"outside-outside-outs");
    let mut cfg = lib_cfg(d.path());
    cfg.library.rescan_s = 3600;
    let s = start(cfg).await;
    wait_models(&s, 1).await;
    let bin = d.path().join("lib/whisper-base/ggml-base.bin");
    std::fs::remove_file(&bin).unwrap();
    std::os::unix::fs::symlink(d.path().join("outside.txt"), &bin).unwrap();
    let r = get(
        s.addr(),
        &file_path("whisper-base", "ggml-base.bin"),
        Some(TOKEN),
        "",
    )
    .await;
    assert_eq!(r.status, 404);
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_count_toward_the_key() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    let s = start(lib_cfg(d.path())).await;
    wait_models(&s, 1).await;
    let f = file_path("whisper-base", "ggml-base.bin");
    get(s.addr(), &f, Some(TOKEN), "").await;
    get(s.addr(), &f, Some(TOKEN), "Range: bytes=10-\r\n").await;
    // Listing and failed requests are not downloads.
    list(s.addr()).await;
    get(s.addr(), &f, Some(TOKEN), "Range: bytes=99-\r\n").await;
    let mut u = Default::default();
    for _ in 0..50 {
        u = s.keys().unwrap().list()[0].usage.clone();
        if u.downloads == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!((u.downloads, u.download_bytes), (2, 30));
    s.kill();
    let saved = std::fs::read_to_string(d.path().join("state/key-usage.json")).unwrap();
    assert!(saved.contains("\"download_bytes\": 30"), "{saved}");
}

#[tokio::test(flavor = "multi_thread")]
async fn models_added_changed_and_removed_without_restart() {
    let d = tempfile::tempdir().unwrap();
    let s = start(lib_cfg(d.path())).await;
    assert!(list(s.addr()).await.models.is_empty());
    tag_whisper(d.path());
    wait_models(&s, 1).await;
    put(
        &d.path().join("lib/qwen/model.toml"),
        b"kind = \"llm\"\ntitle = \"Qwen\"\nengine = \"gguf\"\n",
    );
    put(&d.path().join("lib/qwen/q.gguf"), b"gguf");
    let l = wait_models(&s, 2).await;
    assert_eq!(l.models[0].id, "qwen");
    std::fs::remove_dir_all(d.path().join("lib/qwen")).unwrap();
    wait_models(&s, 1).await;
    s.kill();
}

/// Editing only a tag (same files, same model set) shows on the next
/// rescan: no restart (docs/server-setup.md §10).
#[tokio::test(flavor = "multi_thread")]
async fn tag_edits_show_without_restart() {
    let d = tempfile::tempdir().unwrap();
    put(
        &d.path().join("lib/pk/model.toml"),
        b"kind = \"stt\"\ntitle = \"Parakeet\"\n",
    );
    put(&d.path().join("lib/pk/tokens.txt"), b"a b");
    let s = start(lib_cfg(d.path())).await;
    let l = wait_models(&s, 1).await;
    assert_eq!(l.models[0].engine, None);
    put(
        &d.path().join("lib/pk/model.toml"),
        b"kind = \"stt\"\ntitle = \"Parakeet (English)\"\nlanguage = \"en\"\nengine = \"parakeet\"\n",
    );
    for _ in 0..50 {
        let m = list(s.addr()).await.models.remove(0);
        if m.engine.is_some() {
            assert_eq!(m.title, "Parakeet (English)");
            assert_eq!(m.language.as_deref(), Some("en"));
            assert_eq!(m.engine.as_deref(), Some("parakeet"));
            s.kill();
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the edited tag never showed");
}

#[test]
fn cli_library_list_shows_served_and_skipped() {
    let d = tempfile::tempdir().unwrap();
    tag_whisper(d.path());
    put(&d.path().join("lib/untagged/x.bin"), b"x");
    let conf = d.path().join("config.toml");
    std::fs::write(
        &conf,
        format!(
            "[library]\ndir = {:?}\n",
            d.path().join("lib").to_str().unwrap()
        ),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_flow-server"))
        .args(["library", "list", "--config"])
        .arg(&conf)
        .arg("--state-dir")
        .arg(d.path().join("state"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let t = String::from_utf8(out.stdout).unwrap();
    assert!(t.contains("whisper-base") && t.contains("stt"), "{t}");
    assert!(
        t.contains("skipped untagged: untagged: no model.toml"),
        "{t}"
    );
    assert!(d.path().join("state/library-hashes.json").is_file());
}
