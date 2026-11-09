//! D22 pause-segmented decoding across routes (docs/proposals/
//! streaming-asr-latency.md §5 "Contract"): the cuts are a function of the
//! audio, so local, server, router fallback replay and an old server that
//! ignores the field all give the same text; the cloud route still makes
//! exactly one upload, at commit.

use std::sync::Arc;
use std::time::Duration;

use flow_contract_tests::*;
use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest, BurstEchoAsr, MockAsr};
use flow_core::config::SessionConfig;
use flow_core::{CommitResult, CommitStatus, FlowCore};
use flow_router::RouterConfig;

const S: usize = 16_000;

/// `n` phrases of 4.5 s (amplitude 1000·k → "w<k>") with 1 s pauses.
fn long_dictation(n: usize) -> (Vec<i16>, String) {
    let mut pcm = vec![0i16; S * 3 / 10];
    let mut words = Vec::new();
    for i in 0..n {
        let k = (i % 30 + 1) as f32;
        pcm.extend((0..S * 9 / 2).map(|j| ((j as f32 * 0.05).sin() * 1000.0 * k) as i16));
        pcm.extend(vec![0i16; S]);
        words.push(format!("w{}", i % 30 + 1));
    }
    (pcm, words.join(" "))
}

fn seg_cfg() -> SessionConfig {
    SessionConfig {
        segment_pause_ms: Some(800),
        llm_timeout_ms: 0,
        ..Default::default()
    }
}

fn burst_core() -> FlowCore {
    FlowCore::from_engines(Arc::new(BurstEchoAsr::new()), None)
}

/// Start, push in 100 ms frames (calling `at(pushed_ms)` before each),
/// commit.
fn dictate(
    d: &dyn Dictation,
    cfg: SessionConfig,
    pcm: &[i16],
    mut at: impl FnMut(usize),
) -> CommitResult {
    d.d_start(cfg);
    for (i, c) in pcm.chunks(FRAME).enumerate() {
        at(i * FRAME / 16);
        d.d_push(c);
        std::thread::sleep(Duration::from_millis(1));
    }
    d.d_commit()
}

#[test]
fn equivalence_segmented_local_vs_server() {
    let (pcm, words) = long_dictation(8);
    let local = dictate(&burst_core(), seg_cfg(), &pcm, |_| {});
    assert_eq!(local.asr_text, words);
    assert!(local.timings.segments >= 4, "{:?}", local.timings);
    let server = spawn_core_server(burst_core());
    let router = remote_router(server.addr());
    let remote = dictate(&router, seg_cfg(), &pcm, |_| {});
    assert_eq!(
        remote.route.as_ref().unwrap().engine,
        "remote",
        "{remote:?}"
    );
    assert_eq!(normalize(&local), normalize(&remote));
    // The server segmented too (its timings come back as they are).
    assert_eq!(remote.timings.segments, local.timings.segments);
    assert!(remote.route.as_ref().unwrap().client_ms.is_some());
}

#[test]
fn equivalence_session_fixtures_with_segmentation() {
    let mut n = 0;
    for c in load_cases().iter().filter(|c| c.mock_text().is_some()) {
        let mut c = c.clone();
        c.session_config.segment_pause_ms = Some(800);
        let text = c.mock_text().unwrap();
        let pcm = audio(&c.audio);
        let local = run_case(
            &FlowCore::from_engines(Arc::new(MockAsr::new(text)), None),
            &c,
            &pcm,
        );
        let server = spawn_mock_server(text);
        let remote = run_case(&remote_router(server.addr()), &c, &pcm);
        assert_eq!(normalize(&local), normalize(&remote), "case {}", c.name);
        n += 1;
    }
    assert!(n >= 40, "{n}");
}

/// Router fallback replay: a 60 s segmented session whose server dies at
/// 50 s; the local core gets the whole buffer replayed, finds the same
/// cuts and gives the local-only text, nothing dropped.
#[test]
fn fallback_replay_segmented_is_identical_to_local_only() {
    let (pcm, words) = long_dictation(11); // 60.8 s
    let reference = dictate(&burst_core(), seg_cfg(), &pcm, |_| {});
    assert_eq!(reference.asr_text, words);
    let server = spawn_core_server(burst_core());
    let mut cfg = router_config(server.addr(), true, true);
    cfg.server.commit_timeout_base_ms = 2000;
    let router = hybrid_router(cfg, Some(Arc::new(burst_core())));
    let mut killed = false;
    let r = dictate(&router, seg_cfg(), &pcm, |ms| {
        if ms >= 50_000 && !killed {
            server.kill();
            killed = true;
        }
    });
    let rt = r.route.clone().unwrap();
    assert_eq!(rt.engine, "local", "{r:?}");
    assert_eq!(rt.fallback.as_deref(), Some("disconnected"));
    assert_eq!(r.timings.dropped_samples, 0);
    assert_eq!(normalize(&r), normalize(&reference));
    assert_eq!(r.timings.segments, reference.timings.segments);
}

/// An engine that ignores segmentation stands in for a pre-D22 server
/// (its core never reads `segment_pause_ms`): the full text still comes
/// back, equal to an unsegmented local run.
struct OldServerAsr;
impl AsrEngine for OldServerAsr {
    fn transcribe(&self, pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: BurstEchoAsr::words(pcm),
            language: Some("en".into()),
            no_speech_prob: None,
        })
    }
    fn segmentable(&self) -> bool {
        false
    }
}

#[test]
fn old_server_ignoring_the_field_still_gives_the_full_text() {
    let (pcm, words) = long_dictation(6);
    let server = spawn_core_server(FlowCore::from_engines(Arc::new(OldServerAsr), None));
    let remote = dictate(&remote_router(server.addr()), seg_cfg(), &pcm, |_| {});
    assert_eq!(remote.status, CommitStatus::RawTranscript, "{remote:?}");
    assert_eq!(remote.asr_text, words);
    assert_eq!(remote.timings.segments, 0);
    let plain = dictate(
        &burst_core(),
        SessionConfig {
            llm_timeout_ms: 0,
            ..Default::default()
        },
        &pcm,
        |_| {},
    );
    assert_eq!(normalize(&remote), normalize(&plain));
}

/// C5: with `segment_pause_ms` set, the cloud route uploads once, at
/// commit, never while the user is speaking.
#[test]
fn cloud_route_with_segmentation_uploads_once_at_commit() {
    let (pcm, _) = long_dictation(6);
    let provider = mock_provider("hello from the cloud");
    let cfg = with_cloud(
        RouterConfig {
            allow_server: false,
            allow_client: false,
            ..Default::default()
        },
        cloud_provider(),
        &[flow_router::Route::Cloud],
    );
    let router = cloud_router(cfg, None);
    let mut seen_before_commit = 0;
    let r = dictate(&router, seg_cfg(), &pcm, |ms| {
        if ms + 100 >= pcm.len() / 16 {
            seen_before_commit = provider.count();
        }
    });
    assert_eq!(r.route.as_ref().unwrap().engine, "cloud", "{r:?}");
    assert_eq!(seen_before_commit, 0, "uploaded before commit");
    assert_eq!(provider.count(), 1);
    assert_eq!(r.timings.segments, 0);
}
