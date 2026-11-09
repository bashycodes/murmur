//! Models from the server's library (D41) end to end: a real in-process
//! murmur-server with a temp library, the Windows planner and installer on
//! top of murmur-remote's client. A custom model downloads, verifies,
//! installs under `models\server\<id>` and is selected; a catalogue
//! model comes from the server (its pins), never from Hugging Face, and
//! with Hugging Face unreachable it still installs and resumes (D79).
#![cfg(feature = "remote")]

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use murmur_core::config::{AsrBackend, ModelConfig};
use murmur_remote::library::LibraryClient;
use murmur_router::ServerConfig;
use murmur_windows::model_download::{
    self as md, CatalogEntry, ModelEngine, ModelFile, State, Stop,
};
use murmur_windows::prefs::{AsrEngineChoice, ModelSource, Prefs};
use murmur_windows::server_library::{self as sl, Origin, Slot};
use sha2::{Digest, Sha256};

const TOKEN: &str = "test-token-0123456789abcdef";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("murmur-srvlib-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn put(p: &Path, body: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn sha(b: &[u8]) -> &'static str {
    Box::leak(
        Sha256::digest(b)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            .into_boxed_str(),
    )
}

const WHISPER: &[u8] = b"custom whisper weights";
const CATALOGUE: &[u8] = b"catalogue copy bytes";

struct Server {
    handle: murmur_server::ServerHandle,
    _rt: tokio::runtime::Runtime,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.kill();
    }
}

fn start(lib: &Path, state: &Path, models: usize) -> (Server, ServerConfig) {
    let cfg = murmur_server::Config {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model: ModelConfig {
            asr_backend: AsrBackend::Mock { text: "x".into() },
            ..Default::default()
        },
        state_dir: Some(state.to_path_buf()),
        library: murmur_server::library::LibraryConfig {
            dir: Some(lib.to_path_buf()),
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
    let sc = ServerConfig {
        host: "127.0.0.1".into(),
        port: handle.addr().port(),
        token: TOKEN.into(),
        ..Default::default()
    };
    let c = LibraryClient::new(sc.clone(), "test").unwrap();
    for _ in 0..100 {
        if c.list().unwrap().models.len() == models {
            return (Server { handle, _rt: rt }, sc);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("library never listed {models} models");
}

#[test]
fn a_custom_model_downloads_installs_and_is_selected() {
    let d = tmp("custom");
    put(
        &d.join("lib/my-whisper/model.toml"),
        b"kind = \"stt\"\ntitle = \"My whisper\"\nengine = \"whisper\"\n",
    );
    put(&d.join("lib/my-whisper/ggml-custom.bin"), WHISPER);
    put(
        &d.join("lib/vad/model.toml"),
        b"kind = \"vad\"\ntitle = \"Silero\"\nengine = \"silero\"\n",
    );
    put(&d.join("lib/vad/silero.onnx"), b"v");
    let (_s, cfg) = start(&d.join("lib"), &d.join("state"), 2);
    let models = d.join("models");
    let client = LibraryClient::new(cfg, "test").unwrap();
    let list = client.list().unwrap().models;
    let all: &dyn Fn(ModelEngine) -> bool = &|_| true;

    // Settings' list: sizes, a slot or the reason.
    let v = sl::views(&list, &models, &Prefs::default(), None, all);
    let w = v.iter().find(|m| m.id == "my-whisper").unwrap();
    assert_eq!(
        (w.size, w.slot, w.installed),
        (WHISPER.len() as u64, Some(Slot::SttWhisper), false)
    );
    let vad = v.iter().find(|m| m.id == "vad").unwrap();
    assert!(vad.slot.is_none() && vad.reason.is_some());

    // Download + verify + install + select.
    let m = list.iter().find(|m| m.id == "my-whisper").unwrap();
    let plan = sl::plan(m, all).unwrap();
    let mut events = Vec::new();
    let path = sl::install_custom(
        &client,
        m,
        &plan,
        &models,
        &AtomicBool::new(false),
        &mut |p| events.push(p),
    )
    .unwrap();
    assert_eq!(
        path,
        models
            .join("server")
            .join("my-whisper")
            .join("ggml-custom.bin")
    );
    assert_eq!(std::fs::read(&path).unwrap(), WHISPER);
    assert!(events.iter().all(|p| p.id == "server:my-whisper"));
    assert!(events.iter().any(|p| p.state == State::Verifying));
    let mut prefs = Prefs {
        asr_engine: AsrEngineChoice::Parakeet,
        ..Default::default()
    };
    sl::apply(plan.slot, &path, &mut prefs);
    assert_eq!(prefs.asr_engine, AsrEngineChoice::Whisper);
    assert_eq!(prefs.model_path.as_deref(), Some(path.to_str().unwrap()));
    let v = sl::views(&list, &models, &prefs, None, all);
    let w = v.iter().find(|m| m.id == "my-whisper").unwrap();
    assert!(w.installed && w.in_use);
    let _ = std::fs::remove_dir_all(&d);
}

/// A synthetic catalogue entry pinned to `CATALOGUE` (the real ones pin
/// files too large for a test).
fn pinned_entry() -> CatalogEntry {
    CatalogEntry {
        id: "whisper-base",
        title: "Whisper base",
        desc: "",
        engine: ModelEngine::Whisper,
        dest: "ggml-base.bin",
        page: "https://huggingface.co/r",
        recommended: false,
        recommended_gpu: false,
        language: None,
        license: None,
        extras: &[],
        files: Box::leak(Box::new([ModelFile {
            name: "ggml-base.bin",
            url: "https://huggingface.co/r/resolve/0000000000000000000000000000000000000000/ggml-base.bin",
            size: CATALOGUE.len() as u64,
            sha256: sha(CATALOGUE),
        }])),
    }
}

#[test]
fn catalogue_models_come_from_the_server_never_hugging_face() {
    let d = tmp("catalogue");
    put(
        &d.join("lib/wb/model.toml"),
        b"kind = \"stt\"\ntitle = \"Whisper base\"\ncatalogue_id = \"whisper-base\"\nengine = \"whisper\"\n",
    );
    put(&d.join("lib/wb/ggml-base.bin"), CATALOGUE);
    let (_s, cfg) = start(&d.join("lib"), &d.join("state"), 1);
    let models = d.join("models");
    let hf_calls = AtomicUsize::new(0);
    let hf = || -> Result<Box<dyn md::Fetch + Send>, String> {
        hf_calls.fetch_add(1, Ordering::SeqCst);
        Err("Hugging Face must not be asked".into())
    };
    let origin = Origin::of(Some(cfg.clone()), ModelSource::Server);
    let e = pinned_entry();

    // The server's copy matches the (synthetic) pin: installed where the
    // catalogue puts it, from the server.
    let path = sl::download_catalogue(
        &e,
        &models,
        &origin,
        "t",
        &hf,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(path, models.join("ggml-base.bin"));
    assert_eq!(std::fs::read(&path).unwrap(), CATALOGUE);

    // The real whisper-base pin differs from the server's bytes: refused,
    // nothing written, and still no Hugging Face request.
    let real = md::entry("whisper-base").unwrap();
    let r = sl::download_catalogue(
        real,
        &d.join("m2"),
        &origin,
        "t",
        &hf,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    let Err(Stop::Failed(why)) = r else {
        panic!("{r:?}")
    };
    assert!(why.contains("fingerprint"), "{why}");
    assert!(!d.join("m2").join(real.dest).exists());

    // A catalogue model the server lacks: an error, no fallback.
    let pk = md::entry("parakeet-tdt-0.6b-v2-int8").unwrap();
    let r = sl::download_catalogue(
        pk,
        &d.join("m3"),
        &origin,
        "t",
        &hf,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    let Err(Stop::Failed(why)) = r else {
        panic!("{r:?}")
    };
    assert!(why.contains("not on your server"), "{why}");

    assert_eq!(
        hf_calls.load(Ordering::SeqCst),
        0,
        "no Hugging Face request while a server is set"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Hugging Face as an air-gapped network sees it: every request fails.
struct Blocked(Arc<AtomicUsize>);

impl md::Fetch for Blocked {
    fn open(&self, _url: &str) -> Result<Box<dyn std::io::Read + Send>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err("huggingface.co: network unreachable".into())
    }
}

/// A TCP relay in front of the server that keeps every byte passing each
/// way, so the test reads the HTTP exchange itself. A byte reaches the
/// other side only after it is kept, so once the download returned the
/// exchange is complete: no waiting.
struct Tap {
    port: u16,
    up: Arc<Mutex<Vec<u8>>>,
    down: Arc<Mutex<Vec<u8>>>,
}

fn tap(server_port: u16) -> Tap {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (up, down) = (Arc::default(), Arc::default());
    let (u, d) = (Arc::clone(&up), Arc::clone(&down));
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(c) = c else { return };
            let s = TcpStream::connect(("127.0.0.1", server_port)).unwrap();
            relay(
                c.try_clone().unwrap(),
                s.try_clone().unwrap(),
                Arc::clone(&u),
            );
            relay(s, c, Arc::clone(&d));
        }
    });
    Tap { port, up, down }
}

fn relay(mut from: TcpStream, mut to: TcpStream, log: Arc<Mutex<Vec<u8>>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n @ 1..) = from.read(&mut buf) {
            log.lock().unwrap().extend_from_slice(&buf[..n]);
            if to.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = to.shutdown(Shutdown::Write);
    });
}

#[test]
fn hugging_face_blocked_the_server_installs_and_resumes_a_catalogue_model() {
    let d = tmp("offline");
    put(
        &d.join("lib/wb/model.toml"),
        b"kind = \"stt\"\ntitle = \"Whisper base\"\ncatalogue_id = \"whisper-base\"\n",
    );
    put(&d.join("lib/wb/ggml-base.bin"), CATALOGUE);
    let (s, cfg) = start(&d.join("lib"), &d.join("state"), 1);
    let wire = tap(s.handle.addr().port());
    let cfg = ServerConfig {
        port: wire.port,
        ..cfg
    };
    let hf_requests = Arc::new(AtomicUsize::new(0));
    let hf = {
        let n = Arc::clone(&hf_requests);
        move || -> Result<Box<dyn md::Fetch + Send>, String> {
            Ok(Box::new(Blocked(Arc::clone(&n))))
        }
    };
    let e = pinned_entry();

    // "Download from: Hugging Face" fails on this network.
    let r = sl::download_catalogue(
        &e,
        &d.join("hf"),
        &Origin::of(Some(cfg.clone()), ModelSource::HuggingFace),
        "t",
        &hf,
        &AtomicBool::new(false),
        &mut |_| {},
    );
    assert!(matches!(r, Err(Stop::Failed(_))), "{r:?}");
    assert_eq!(hf_requests.swap(0, Ordering::SeqCst), 1);

    // "Your server" (the default): an interrupted download left a prefix
    // in the `.part` file; the retry asks only for the rest, checks the
    // pin and installs it.
    let models = d.join("models");
    let cut = 8;
    put(&models.join("ggml-base.bin.part"), &CATALOGUE[..cut]);
    let mut events = Vec::new();
    let path = sl::download_catalogue(
        &e,
        &models,
        &Origin::of(Some(cfg), Prefs::default().model_source),
        "t",
        &hf,
        &AtomicBool::new(false),
        &mut |p| events.push(p),
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), CATALOGUE);
    assert!(!models.join("ggml-base.bin.part").exists());
    assert!(events.iter().any(|p| p.state == State::Verifying));
    assert_eq!(
        hf_requests.load(Ordering::SeqCst),
        0,
        "Hugging Face never asked"
    );

    // On the wire: a Range request for the rest, a 206 answer, and the
    // whole file never sent.
    let up = String::from_utf8_lossy(&wire.up.lock().unwrap()).to_lowercase();
    let down = wire.down.lock().unwrap().clone();
    assert!(up.contains(&format!("range: bytes={cut}-")), "{up}");
    assert!(String::from_utf8_lossy(&down).contains("HTTP/1.1 206"));
    assert!(down.ends_with(&CATALOGUE[cut..]), "the rest was sent last");
    assert!(
        !down.windows(CATALOGUE.len()).any(|w| w == CATALOGUE),
        "the whole file was sent"
    );
    let _ = std::fs::remove_dir_all(&d);
}
