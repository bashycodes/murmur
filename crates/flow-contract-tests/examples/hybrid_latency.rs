//! Latency probe: the same WAV files dictated locally (in-process FlowCore)
//! and through Router → flow-remote → a running flow-server.
//!
//! cargo run --release -p flow-contract-tests --features parakeet --example hybrid_latency -- \
//!     --server 127.0.0.1:7313 --token-file TOKEN --model '{"asr_engine":"parakeet","asr_path":"…","threads":4}' \
//!     a.wav b.wav …
//!
//! Audio is pushed in 100 ms frames at real-time pace (like a shell), so
//! the number that matters is commit → result ("release → text").
//! `--session '{"partial_interval_ms":1500,"segment_pause_ms":800}'` sets
//! the SessionConfig of both routes (default `{}`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_core::config::{ModelConfig, SessionConfig};
use flow_core::{CommitResult, FlowCore};
use flow_router::{Router, RouterConfig, ServerConfig};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn files() -> Vec<String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.len() {
        if a[i].starts_with("--") {
            i += 2;
        } else {
            out.push(a[i].clone());
            i += 1;
        }
    }
    out
}

/// Push at real time, then time the commit.
fn dictate(
    start: &dyn Fn(),
    push: &dyn Fn(&[i16]),
    commit: &dyn Fn() -> CommitResult,
    pcm: &[i16],
    pace: bool,
) -> (CommitResult, Duration) {
    start();
    let t0 = Instant::now();
    for (i, c) in pcm.chunks(1600).enumerate() {
        push(c);
        if pace {
            let due = Duration::from_millis(100 * (i as u64 + 1));
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    }
    let t = Instant::now();
    let r = commit();
    (r, t.elapsed())
}

fn main() {
    let server: std::net::SocketAddr = arg("--server")
        .unwrap_or("127.0.0.1:7313".into())
        .parse()
        .unwrap();
    let token = arg("--token-file")
        .map(|p| std::fs::read_to_string(p).unwrap().trim().to_string())
        .unwrap_or_default();
    let model: ModelConfig = serde_json::from_str(&arg("--model").expect("--model JSON")).unwrap();
    let pace = arg("--no-pace").is_none();
    let t = Instant::now();
    let core = Arc::new(FlowCore::load_models(&model).expect("local model"));
    eprintln!("local load: {} ms", t.elapsed().as_millis());
    let router = Router::new(
        RouterConfig {
            allow_client: false,
            server: ServerConfig {
                host: server.ip().to_string(),
                port: server.port(),
                token,
                ..Default::default()
            },
            ..Default::default()
        },
        None,
        Some(flow_remote::factory("hybrid_latency")),
    );
    let h = router.probe_server();
    eprintln!(
        "probe: ok={} rtt={:?} server={:?} asr={:?}",
        h.ok, h.rtt_ms, h.server_version, h.asr_model
    );
    assert!(h.ok, "{h:?}");
    // `--session JSON`: the SessionConfig both routes use (e.g. the
    // Android shell's `{"partial_interval_ms":1500}`, plus
    // `"segment_pause_ms":800` for D22). Default `{}`.
    let cfg: SessionConfig =
        serde_json::from_str(&arg("--session").unwrap_or_else(|| "{}".into())).unwrap();
    println!("file\taudio_ms\tlocal_commit_ms\tremote_commit_ms\tlocal_asr_ms\tserver_asr_ms\tsame_text\ttext");
    let (mut sl, mut sr, mut n) = (0u128, 0u128, 0u128);
    for f in files() {
        let pcm = flow_core::audio::read_wav_16k_mono(std::path::Path::new(&f)).unwrap();
        let c2 = core.clone();
        let (l, lt) = dictate(
            &|| c2.start_session(cfg.clone()).unwrap(),
            &|p| {
                core.push_audio(p);
            },
            &|| core.commit(),
            &pcm,
            pace,
        );
        let (r, rt) = dictate(
            &|| router.start_session(cfg.clone()).unwrap(),
            &|p| {
                router.push_audio(p);
            },
            &|| router.commit(),
            &pcm,
            pace,
        );
        let route = r.route.clone().unwrap();
        assert_eq!(route.engine, "remote", "{route:?} {:?}", r.error);
        let name = std::path::Path::new(&f)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        println!(
            "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            l.timings.audio_ms,
            lt.as_millis(),
            rt.as_millis(),
            l.timings.asr_ms,
            r.timings.asr_ms,
            l.final_text == r.final_text,
            r.final_text
        );
        sl += lt.as_millis();
        sr += rt.as_millis();
        n += 1;
    }
    if let (Some(ml), Some(mr)) = (sl.checked_div(n), sr.checked_div(n)) {
        println!("MEAN\t-\t{ml}\t{mr}\t-\t-\t-\t-");
    }
}
