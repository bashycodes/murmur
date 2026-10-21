//! The router over a real `FlowCore` (MockAsr): the local path behaves like
//! FlowCore itself, and a fallback replay loses no samples.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use flow_core::asr::MockAsr;
use flow_core::FlowCore;
use flow_router::*;

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn local_cfg() -> RouterConfig {
    RouterConfig {
        allow_server: false,
        ..Default::default()
    }
}

fn session() -> SessionConfig {
    SessionConfig {
        dictionary: serde_json::from_str(r#"[{"word":"wispr","replacement":"Wispr"}]"#).unwrap(),
        ..Default::default()
    }
}

#[test]
fn local_route_equals_flowcore_directly() {
    let asr_text = "um so wispr is great press enter";
    let direct = FlowCore::from_engines(Arc::new(MockAsr::new(asr_text)), None);
    direct.start_session(session()).unwrap();
    direct.set_context(FieldContext::default());
    direct.push_audio(&speech(1000));
    let want = direct.commit();

    let core = Arc::new(FlowCore::from_engines(
        Arc::new(MockAsr::new(asr_text)),
        None,
    ));
    let r = Router::new(local_cfg(), Some(core as Arc<dyn Backend>), None);
    r.start_session(session()).unwrap();
    r.set_context(FieldContext::default());
    assert!(r.push_audio(&speech(1000)));
    let mut got = r.commit();
    let route = got.route.take().unwrap();
    assert_eq!(route.engine, "local");
    assert_eq!(route.fallback, None);
    let strip = |mut c: CommitResult| {
        c.timings = Default::default();
        c
    };
    assert_eq!(strip(got), strip(want));
}

/// A remote that accepts audio until `die_at` samples, then reports the
/// connection lost.
struct DyingRemote {
    got: AtomicU64,
    die_at: u64,
}

impl Backend for DyingRemote {
    fn kind(&self) -> EngineKind {
        EngineKind::Remote
    }
    fn ready(&self) -> bool {
        true
    }
    fn start_session(&self, _: &SessionConfig) -> Result<StartInfo, BackendError> {
        Ok(StartInfo::default())
    }
    fn push_audio(&self, pcm: &[i16]) -> bool {
        self.got.fetch_add(pcm.len() as u64, Ordering::SeqCst) + (pcm.len() as u64) < self.die_at
    }
    fn set_context(&self, _: &FieldContext) {}
    fn partial(&self) -> Option<String> {
        None
    }
    fn commit(&self, _: Instant) -> Result<CommitResult, BackendError> {
        Err(BackendError::new(Failure::Disconnected, "gone"))
    }
    fn cancel(&self) {}
    fn probe(&self) -> HealthReport {
        HealthReport::default()
    }
    fn session_failure(&self) -> Option<BackendError> {
        (self.got.load(Ordering::SeqCst) >= self.die_at)
            .then(|| BackendError::new(Failure::Disconnected, "gone"))
    }
}

#[test]
fn fallback_replay_of_a_long_buffer_drops_nothing() {
    let asr = Arc::new(MockAsr::new("hello"));
    let core = Arc::new(FlowCore::from_engines(asr.clone(), None));
    let factory: RemoteFactory = Box::new(|_| {
        Arc::new(DyingRemote {
            got: AtomicU64::new(0),
            // Dies after 50 s: longer than FlowCore's default 30 s ring.
            die_at: 16_000 * 50,
        }) as Arc<dyn Backend>
    });
    let cfg = RouterConfig {
        server: ServerConfig {
            host: "127.0.0.1".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let r = Router::new(cfg, Some(core as Arc<dyn Backend>), Some(factory));
    let s = SessionConfig {
        vad: false,
        ..Default::default()
    };
    r.start_session(s).unwrap();
    let one_s = speech(1000);
    for _ in 0..60 {
        for c in one_s.chunks(1600) {
            assert!(r.push_audio(c));
        }
    }
    let res = r.commit();
    let route = res.route.clone().unwrap();
    assert_eq!(route.engine, "local");
    assert_eq!(route.fallback.as_deref(), Some("disconnected"));
    assert_eq!(res.timings.dropped_samples, 0);
    assert_eq!(
        asr.last_len(),
        16_000 * 60,
        "every pushed sample reached ASR"
    );
    assert!(res.final_text.to_lowercase().contains("hello"), "{res:?}");
}
