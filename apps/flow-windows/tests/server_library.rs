//! Models from the server's library (D41) end to end: a real in-process
//! flow-server with a temp library, the Windows planner and installer on
//! top of flow-remote's client. A custom model downloads, verifies,
//! installs under `models\server\<id>` and is selected; a catalogue
//! model comes from the server (its pins), never from Hugging Face, and
//! with Hugging Face unreachable it still installs and resumes (D79).
#![cfg(feature = "remote")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use flow_core::config::{AsrBackend, ModelConfig};
use flow_remote::library::LibraryClient;
use flow_router::ServerConfig;
use flow_windows::model_download::{self as md, CatalogEntry, ModelEngine, ModelFile, State, Stop};
use flow_windows::prefs::{AsrEngineChoice, ModelSource, Prefs};
use flow_windows::server_library::{self as sl, Origin, Slot};
use sha2::{Digest, Sha256};

const TOKEN: &str = "test-token-0123456789abcdef";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("flow-srvlib-it-{name}-{}", std::process::id()));
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
    handle: flow_server::ServerHandle,
    _rt: tokio::runtime::Runtime,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.kill();
    }
}

fn start(lib: &Path, state: &Path, models: usize) -> (Server, ServerConfig) {
    let cfg = flow_server::Config {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model: ModelConfig {
            asr_backend: AsrBackend::Mock { text: "x".into() },
            ..Default::default()
        },
        state_dir: Some(state.to_path_buf()),
        library: flow_server::library::LibraryConfig {
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
        flow_server::serve(cfg, vec![l]).await.unwrap()
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
struct Blocked(&'static AtomicUsize);

impl md::Fetch for Blocked {
    fn open(&self, _url: &str) -> Result<Box<dyn std::io::Read + Send>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err("huggingface.co: network unreachable".into())
    }
}

#[test]
fn with_no_internet_the_server_serves_and_resumes_catalogue_models() {
    let d = tmp("offline");
    put(
        &d.join("lib/wb/model.toml"),
        b"kind = \"stt\"\ntitle = \"Whisper base\"\ncatalogue_id = \"whisper-base\"\n",
    );
    put(&d.join("lib/wb/ggml-base.bin"), CATALOGUE);
    let (s, cfg) = start(&d.join("lib"), &d.join("state"), 1);
    static HF_REQUESTS: AtomicUsize = AtomicUsize::new(0);
    let hf =
        || -> Result<Box<dyn md::Fetch + Send>, String> { Ok(Box::new(Blocked(&HF_REQUESTS))) };
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
    assert_eq!(HF_REQUESTS.swap(0, Ordering::SeqCst), 1);

    // "Your server" (the default): an interrupted download left a prefix
    // in the `.part` file; the retry fetches only the rest with a Range
    // request, checks the pin and installs it.
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
        HF_REQUESTS.load(Ordering::SeqCst),
        0,
        "Hugging Face never asked"
    );
    let mut sent = 0;
    for _ in 0..50 {
        sent = s.handle.keys().unwrap().list()[0].usage.download_bytes;
        if sent > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        sent,
        (CATALOGUE.len() - cut) as u64,
        "only the rest was sent"
    );
    let _ = std::fs::remove_dir_all(&d);
}
