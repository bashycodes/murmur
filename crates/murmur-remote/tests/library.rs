//! The model library client (D41) against a real in-process murmur-server
//! with a temp library: list, download + verify, resume from a `.part`,
//! the expected fingerprint enforced (ETag and content), cancel, auth,
//! unknown files.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use murmur_core::config::{AsrBackend, ModelConfig};
use murmur_protocol::library::{LibraryFile, ModelKind};
use murmur_remote::library::{ErrorKind, LibraryClient};
use murmur_router::ServerConfig;
use murmur_server::{Config, ServerHandle};
use sha2::{Digest, Sha256};

const TOKEN: &str = "test-token-0123456789abcdef";

fn put(p: &Path, body: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn sha(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 3 MB of not-quite-repeating bytes: several HTTP frames.
fn weights() -> Vec<u8> {
    (0..3_000_000u32).map(|i| (i ^ (i >> 7)) as u8).collect()
}

struct Server {
    handle: ServerHandle,
    rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

impl Server {
    fn cfg(&self, token: &str) -> ServerConfig {
        ServerConfig {
            host: "127.0.0.1".into(),
            port: self.handle.addr().port(),
            token: token.into(),
            ..Default::default()
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.kill();
        let _ = &self.rt;
    }
}

/// murmur-server with a library of one custom whisper model and one Hebrew
/// sherpa folder (with a subfolder file).
fn start() -> Server {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    put(
        &lib.join("my-whisper/model.toml"),
        b"kind = \"stt\"\ntitle = \"My whisper\"\nengine = \"whisper\"\n",
    );
    put(&lib.join("my-whisper/ggml-custom.bin"), &weights());
    put(
        &lib.join("he-online/model.toml"),
        b"kind = \"hebrew\"\ntitle = \"Hebrew online\"\nlanguage = \"he\"\nengine = \"sherpa_online\"\n",
    );
    put(&lib.join("he-online/tokens.txt"), b"a 0\nb 1\n");
    put(&lib.join("he-online/onnx/encoder.onnx"), b"enc");
    let cfg = Config {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model: ModelConfig {
            asr_backend: AsrBackend::Mock { text: "x".into() },
            ..Default::default()
        },
        state_dir: Some(d.path().join("state")),
        library: murmur_server::library::LibraryConfig {
            dir: Some(lib),
            rescan_s: 0,
        },
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        murmur_server::serve(cfg, vec![l]).await.unwrap()
    });
    let s = Server {
        handle,
        rt,
        _dir: d,
    };
    // The first scan hashes in the background.
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    for _ in 0..100 {
        if c.list().unwrap().models.len() == 2 {
            return s;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("library never listed both models");
}

fn whisper_file(c: &LibraryClient) -> LibraryFile {
    let l = c.list().unwrap();
    let m = l.models.iter().find(|m| m.id == "my-whisper").unwrap();
    m.files[0].clone()
}

fn never() -> bool {
    false
}

#[test]
fn lists_the_library_with_sizes_and_hashes() {
    let s = start();
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let l = c.list().unwrap();
    let w = l.models.iter().find(|m| m.id == "my-whisper").unwrap();
    assert_eq!(w.kind, ModelKind::Stt);
    assert_eq!(w.engine.as_deref(), Some("whisper"));
    assert_eq!(w.files[0].name, "ggml-custom.bin");
    assert_eq!(w.files[0].size, 3_000_000);
    assert_eq!(w.files[0].sha256, sha(&weights()));
    let h = l.models.iter().find(|m| m.id == "he-online").unwrap();
    assert_eq!(h.kind, ModelKind::Hebrew);
    assert!(h.files.iter().any(|f| f.name == "onnx/encoder.onnx"));
}

#[test]
fn downloads_and_verifies_into_dest() {
    let s = start();
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let f = whisper_file(&c);
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("m/ggml-custom.bin");
    let part = d.path().join("m/ggml-custom.bin.part");
    let mut seen = Vec::new();
    c.download(
        "my-whisper",
        &f,
        &dest,
        &part,
        &mut |n| seen.push(n),
        &never,
    )
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), weights());
    assert!(!part.exists());
    assert_eq!(seen.first(), Some(&0));
    assert_eq!(seen.last(), Some(&3_000_000));
    // Already there and intact: no request (the server could be gone).
    let gone = ServerConfig {
        port: 1,
        ..s.cfg(TOKEN)
    };
    let c2 = LibraryClient::new(gone, "test").unwrap();
    c2.download("my-whisper", &f, &dest, &part, &mut |_| {}, &never)
        .unwrap();
    // A file in a subfolder of a model.
    let l = c.list().unwrap();
    let h = l.models.iter().find(|m| m.id == "he-online").unwrap();
    let enc = h
        .files
        .iter()
        .find(|f| f.name == "onnx/encoder.onnx")
        .unwrap();
    let dest = d.path().join("he/onnx/encoder.onnx");
    c.download(
        "he-online",
        enc,
        &dest,
        &dest.with_extension("onnx.part"),
        &mut |_| {},
        &never,
    )
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"enc");
}

#[test]
fn resumes_from_a_partial_file_with_a_range_request() {
    let s = start();
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let f = whisper_file(&c);
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("ggml-custom.bin");
    let part = d.path().join("ggml-custom.bin.part");
    // What an interrupted download left behind: the first 1 MB.
    let w = weights();
    std::fs::write(&part, &w[..1_000_000]).unwrap();
    let mut seen = Vec::new();
    c.download(
        "my-whisper",
        &f,
        &dest,
        &part,
        &mut |n| seen.push(n),
        &never,
    )
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), w);
    assert_eq!(
        seen.first(),
        Some(&1_000_000),
        "progress starts at the prefix"
    );

    // Proof that only the rest was fetched: a wrong prefix of the right
    // length is kept by the server's 206 and fails the fingerprint (and
    // is deleted), instead of being replaced by a whole new file.
    std::fs::remove_file(&dest).unwrap();
    std::fs::write(&part, vec![0u8; 1_000_000]).unwrap();
    let e = c
        .download("my-whisper", &f, &dest, &part, &mut |_| {}, &never)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Mismatch, "{e}");
    assert!(!part.exists() && !dest.exists());

    // A part longer than the file starts over.
    std::fs::write(&part, vec![0u8; 3_000_001]).unwrap();
    c.download("my-whisper", &f, &dest, &part, &mut |_| {}, &never)
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), w);
}

#[test]
fn a_server_copy_with_another_fingerprint_is_refused_before_downloading() {
    let s = start();
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let mut f = whisper_file(&c);
    // What a catalogue model's built-in pin looks like when the server's
    // copy differs.
    f.sha256 = "0".repeat(64);
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("x.bin");
    let part = d.path().join("x.bin.part");
    let mut seen = Vec::new();
    let e = c
        .download(
            "my-whisper",
            &f,
            &dest,
            &part,
            &mut |n| seen.push(n),
            &never,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Mismatch, "{e}");
    assert!(
        seen.iter().all(|n| *n == 0),
        "no byte was written: {seen:?}"
    );
    assert!(!part.exists() && !dest.exists());
}

#[test]
fn cancel_keeps_nothing() {
    let s = start();
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let f = whisper_file(&c);
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("x.bin");
    let part = d.path().join("x.bin.part");
    let stop = AtomicBool::new(false);
    let e = c
        .download(
            "my-whisper",
            &f,
            &dest,
            &part,
            &mut |n| {
                if n > 0 {
                    stop.store(true, Ordering::SeqCst)
                }
            },
            &|| stop.load(Ordering::SeqCst) || std::fs::metadata(&part).is_ok_and(|m| m.len() > 0),
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert!(!part.exists() && !dest.exists());
}

#[test]
fn a_wrong_key_and_unknown_files_are_clear_errors() {
    let s = start();
    let bad = LibraryClient::new(s.cfg("wrong-token-0123456789"), "test").unwrap();
    assert_eq!(bad.list().unwrap_err().kind, ErrorKind::Auth);
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let mut f = whisper_file(&c);
    f.name = "nope.bin".into();
    let d = tempfile::tempdir().unwrap();
    let e = c
        .download(
            "my-whisper",
            &f,
            &d.path().join("n"),
            &d.path().join("n.part"),
            &mut |_| {},
            &never,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    f.name = "../evil".into();
    let e = c
        .download(
            "my-whisper",
            &f,
            &d.path().join("n"),
            &d.path().join("n.part"),
            &mut |_| {},
            &never,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Protocol);
}

#[test]
fn a_server_that_is_down_keeps_the_part_for_a_retry() {
    let s = start();
    let c = LibraryClient::new(s.cfg(TOKEN), "test").unwrap();
    let f = whisper_file(&c);
    let port = s.handle.addr().port();
    drop(s);
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("x.bin.part");
    std::fs::write(&part, &weights()[..10]).unwrap();
    let down = ServerConfig {
        host: "127.0.0.1".into(),
        port,
        token: TOKEN.into(),
        connect_timeout_ms: 1000,
        ..Default::default()
    };
    let c = LibraryClient::new(down, "test").unwrap();
    let e = c
        .download(
            "my-whisper",
            &f,
            &d.path().join("x.bin"),
            &part,
            &mut |_| {},
            &never,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e}");
    assert_eq!(
        std::fs::metadata(&part).unwrap().len(),
        10,
        "kept for resume"
    );
}
