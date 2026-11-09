//! Meeting transcription over the router (D44): every chunk is a normal
//! dictation on the route order, with its fallbacks. Fake server, mock ASR.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use murmur_core::asr::MockAsr;
use murmur_core::MurmurCore;
use murmur_router::meeting::{transcribe_pcm, MeetingParams};
use murmur_router::*;

const S: usize = 16_000;

fn voiced(ms: usize) -> Vec<i16> {
    (0..ms * S / 1000)
        .map(|i| {
            let t = i as f32 / S as f32;
            let env = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
            (8000.0 * env * (2.0 * std::f32::consts::PI * 150.0 * t).sin()) as i16
        })
        .collect()
}

fn quiet(ms: usize) -> Vec<i16> {
    let mut x: u32 = 7;
    (0..ms * S / 1000)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((x >> 16) % 61) as i16 - 30
        })
        .collect()
}

/// A server that is down: every start fails to connect.
struct DownServer(Arc<AtomicUsize>);

impl Backend for DownServer {
    fn kind(&self) -> EngineKind {
        EngineKind::Remote
    }
    fn ready(&self) -> bool {
        true
    }
    fn start_session(&self, _cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(BackendError::new(Failure::ConnectFailed, "refused"))
    }
    fn push_audio(&self, _pcm: &[i16]) -> bool {
        false
    }
    fn set_context(&self, _ctx: &FieldContext) {}
    fn partial(&self) -> Option<String> {
        None
    }
    fn commit(&self, _deadline: Instant) -> Result<CommitResult, BackendError> {
        Err(BackendError::new(Failure::Disconnected, "down"))
    }
    fn cancel(&self) {}
    fn probe(&self) -> HealthReport {
        HealthReport::default()
    }
}

fn router(local: bool, tries: Arc<AtomicUsize>) -> Router {
    let cfg = RouterConfig {
        server: ServerConfig {
            host: "100.64.0.10".into(),
            token: "dev-token-0123456789abcdef".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let core: Option<Arc<dyn Backend>> = local.then(|| {
        Arc::new(MurmurCore::from_engines(
            Arc::new(MockAsr::new("we agreed to ship on friday")),
            None,
        )) as Arc<dyn Backend>
    });
    let factory: RemoteFactory =
        Box::new(move |_s: &ServerConfig| Arc::new(DownServer(tries.clone())) as Arc<dyn Backend>);
    Router::new(cfg, core, Some(factory))
}

fn meeting() -> Vec<i16> {
    let mut pcm = Vec::new();
    for _ in 0..3 {
        pcm.extend(quiet(1500));
        pcm.extend(voiced(12_000));
    }
    pcm.extend(quiet(1500));
    pcm
}

#[test]
fn chunks_fall_back_from_a_dead_server_to_this_device() {
    let tries = Arc::new(AtomicUsize::new(0));
    let r = Arc::new(router(true, tries.clone()));
    let tr = transcribe_pcm(
        r.clone(),
        &meeting(),
        &SessionConfig {
            languages: vec!["en".into()],
            ..Default::default()
        },
        MeetingParams::default(),
    )
    .unwrap();
    assert_eq!(tr.segments.len(), 3, "{tr:?}");
    for (i, s) in tr.segments.iter().enumerate() {
        assert_eq!(s.engine.as_deref(), Some("local"));
        assert!(s.text.to_lowercase().contains("ship on friday"), "{s:?}");
        let start = 1500 + i as u64 * 13_500;
        assert!(s.start_ms.abs_diff(start) <= 60, "{s:?}");
    }
    // The server was tried (the circuit then opens; no retry storm).
    assert!(tries.load(Ordering::SeqCst) >= 1);
    let last = r.route_status().last.unwrap();
    assert_eq!(last.engine, "local");
}

#[test]
fn no_route_at_all_ends_the_job_after_three_chunks() {
    let tries = Arc::new(AtomicUsize::new(0));
    let r = Arc::new(router(false, tries));
    let mut pcm = meeting();
    pcm.extend(meeting());
    let e = transcribe_pcm(r, &pcm, &SessionConfig::default(), MeetingParams::default());
    assert!(matches!(e, Err(meeting::MeetingError::Failed(_))), "{e:?}");
}
