#![allow(dead_code)] // each test crate uses part of it
//! An in-process flow-server (mock ASR) with a temp model library of one
//! custom whisper model, for tests/library.rs and tests/jni_library.rs.

use std::path::Path;
use std::time::Duration;

use flow_core::config::{AsrBackend, ModelConfig};
use flow_server::{Config, ServerHandle};
use sha2::{Digest, Sha256};

pub const TOKEN: &str = "test-token-0123456789abcdef";

pub fn sha(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 3 MB of not-quite-repeating bytes: several HTTP frames.
pub fn weights() -> Vec<u8> {
    (0..3_000_000u32).map(|i| (i ^ (i >> 7)) as u8).collect()
}

fn put(p: &Path, body: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

pub struct LibraryServer {
    handle: ServerHandle,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

impl LibraryServer {
    pub fn port(&self) -> u16 {
        self.handle.addr().port()
    }

    /// `ServerConfig` JSON as the Android shell sends it.
    pub fn server_json(&self, token: &str) -> String {
        serde_json::json!({"host": "127.0.0.1", "port": self.port(), "tls": false, "token": token})
            .to_string()
    }

    /// `my-whisper` (custom, `ggml-custom.bin` = [`weights`]).
    pub fn start() -> Self {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        put(
            &lib.join("my-whisper/model.toml"),
            b"kind = \"stt\"\ntitle = \"My whisper\"\nengine = \"whisper\"\n",
        );
        put(&lib.join("my-whisper/ggml-custom.bin"), &weights());
        let cfg = Config {
            bind: vec!["127.0.0.1:0".into()],
            token: Some(TOKEN.into()),
            model: ModelConfig {
                asr_backend: AsrBackend::Mock { text: "x".into() },
                ..Default::default()
            },
            state_dir: Some(d.path().join("state")),
            library: flow_server::library::LibraryConfig {
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
            flow_server::serve(cfg, vec![l]).await.unwrap()
        });
        let s = Self {
            handle,
            _rt: rt,
            _dir: d,
        };
        // The first scan hashes in the background.
        for _ in 0..100 {
            let r: serde_json::Value =
                serde_json::from_str(&flow_ffi::library_api::list(&s.server_json(TOKEN))).unwrap();
            if r["models"].as_array().is_some_and(|m| m.len() == 1) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("library never listed the model");
    }
}

impl Drop for LibraryServer {
    fn drop(&mut self) {
        self.handle.kill();
    }
}
