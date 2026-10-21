//! Router JSON fixtures shared with the shells (Android `RouterJsonTest`
//! parses the same files; Windows prefs serialise to `router_config_*`).
//! Every file parses into its type and re-serialises to the same JSON.
//! Regenerate: `FLOW_REGEN_FIXTURES=1 cargo test -p flow-router --test fixtures`.

use std::path::PathBuf;
use std::sync::Arc;

use flow_core::asr::MockAsr;
use flow_core::FlowCore;
use flow_router::*;
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn server() -> ServerConfig {
    ServerConfig {
        host: "100.64.0.10".into(),
        token: "dev-token-0123456789abcdef".into(),
        ..Default::default()
    }
}

fn configs() -> Vec<(&'static str, RouterConfig)> {
    vec![
        ("router_config_default", RouterConfig::default()),
        (
            "router_config_hybrid",
            RouterConfig {
                server: server(),
                ..Default::default()
            },
        ),
        (
            "router_config_remote_only",
            RouterConfig {
                allow_client: false,
                keep_local_warm: false,
                server: ServerConfig {
                    host: "flow.example.com".into(),
                    port: 443,
                    tls: true,
                    tls_pin_sha256: Some(
                        "3f2a9c0b7e4d1f6a8b5c2e9d0a7f4b1c6e3d8a5f2b9c0e7d4a1f6b3c8e5d2a9f".into(),
                    ),
                    connect_timeout_ms: 1500,
                    ..server()
                },
                ..Default::default()
            },
        ),
        (
            "router_config_local_only",
            RouterConfig {
                allow_server: false,
                prefer: Prefer::Client,
                ..Default::default()
            },
        ),
    ]
}

fn capabilities() -> Vec<(&'static str, Capabilities)> {
    let base = Capabilities {
        proto: 1,
        core_version: "0.1.0".into(),
        ..Default::default()
    };
    vec![
        (
            "capabilities_local",
            Capabilities {
                local: true,
                asr_engines: vec!["whisper".into()],
                llm: true,
                ..base.clone()
            },
        ),
        (
            "capabilities_hybrid",
            Capabilities {
                local: true,
                remote: true,
                asr_engines: vec!["whisper".into(), "parakeet".into()],
                llm: true,
                ..base.clone()
            },
        ),
        (
            "capabilities_remote",
            Capabilities {
                remote: true,
                ..base
            },
        ),
    ]
}

fn health_ok() -> HealthReport {
    HealthReport {
        ok: true,
        engine: EngineKind::Remote,
        server: Some("100.64.0.10:7313".into()),
        rtt_ms: Some(Rtt {
            min: 3,
            median: 4,
            max: 6,
        }),
        server_version: Some("flow-server 0.1.0".into()),
        server_core: Some("0.1.0".into()),
        postprocess_rev: Some("2026-10-21.1".into()),
        postprocess_match: Some(true),
        proto: Some(1),
        asr_backend: Some("parakeet".into()),
        asr_model: Some("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8".into()),
        llm: Some(false),
        workers: Some(1),
        busy: Some(0),
        checked_at_ms: 1_792_540_800_000,
        ..Default::default()
    }
}

fn health_auth_failed() -> HealthReport {
    HealthReport {
        ok: false,
        engine: EngineKind::Remote,
        error: Some("auth_failed".into()),
        message: Some("server rejected the token (HTTP 401)".into()),
        server: Some("100.64.0.10:7313".into()),
        checked_at_ms: 1_792_540_800_000,
        ..Default::default()
    }
}

fn result(route: RouteInfo) -> CommitResult {
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new("ask not what your country can do")),
        None,
    );
    core.start_session(SessionConfig::default()).unwrap();
    let pcm: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    core.push_audio(&pcm);
    let mut r = core.commit();
    r.timings = flow_core::Timings {
        audio_ms: 1000,
        asr_ms: 310,
        llm_ms: 0,
        postprocess_ms: 0,
        total_ms: 330,
        dropped_samples: 0,
        asr_audio_ms: 1000,
    };
    r.route = Some(route);
    r
}

fn remote_route() -> RouteInfo {
    RouteInfo {
        engine: "remote".into(),
        fallback: None,
        server: Some("100.64.0.10:7313".into()),
        rtt_ms: Some(4),
        queue_ms: Some(0),
        server_core: Some("0.1.0".into()),
        provider: None,
    }
}

fn fallback_route() -> RouteInfo {
    RouteInfo {
        engine: "local".into(),
        fallback: Some("connect_failed".into()),
        server: Some("100.64.0.10:7313".into()),
        ..Default::default()
    }
}

fn route_status() -> RouteStatus {
    RouteStatus {
        last: Some(fallback_route()),
        circuit: CircuitStatus {
            open: true,
            reason: Some("connect_failed".into()),
            retry_in_ms: Some(4200),
            until_config_change: false,
        },
        health: Some(health_ok()),
    }
}

fn all() -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = Vec::new();
    for (n, c) in configs() {
        v.push((n.into(), serde_json::to_value(c).unwrap()));
    }
    for (n, c) in capabilities() {
        v.push((n.into(), serde_json::to_value(c).unwrap()));
    }
    v.push((
        "health_ok".into(),
        serde_json::to_value(health_ok()).unwrap(),
    ));
    v.push((
        "health_auth_failed".into(),
        serde_json::to_value(health_auth_failed()).unwrap(),
    ));
    v.push((
        "route_status".into(),
        serde_json::to_value(route_status()).unwrap(),
    ));
    v.push((
        "commit_result_remote".into(),
        serde_json::to_value(result(remote_route())).unwrap(),
    ));
    v.push((
        "commit_result_fallback".into(),
        serde_json::to_value(result(fallback_route())).unwrap(),
    ));
    v
}

#[test]
fn canonical_values_equal_fixtures() {
    for (name, v) in all() {
        let p = dir().join(format!("{name}.json"));
        if std::env::var_os("FLOW_REGEN_FIXTURES").is_some() {
            std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap();
            continue;
        }
        let disk: Value = serde_json::from_str(
            &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{name}: {e}")),
        )
        .unwrap();
        assert_eq!(disk, v, "fixture {name}");
    }
}

fn round_trip<T: serde::de::DeserializeOwned + serde::Serialize>(v: &Value) -> Value {
    serde_json::to_value(serde_json::from_value::<T>(v.clone()).unwrap()).unwrap()
}

#[test]
fn every_fixture_parses_and_round_trips() {
    let mut n = 0;
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_stem().unwrap().to_str().unwrap().to_string();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let back = match name.split('_').next().unwrap() {
            "router" => round_trip::<RouterConfig>(&v),
            "capabilities" => round_trip::<Capabilities>(&v),
            "health" => round_trip::<HealthReport>(&v),
            "route" => round_trip::<RouteStatus>(&v),
            "commit" => round_trip::<CommitResult>(&v),
            other => panic!("unknown fixture kind {other} ({name})"),
        };
        assert_eq!(back, v, "{name}");
        n += 1;
    }
    assert_eq!(n, 12);
}

#[test]
fn configs_validate_and_partial_json_gets_defaults() {
    for (n, c) in configs() {
        assert!(c.validate().is_ok(), "{n}");
    }
    let c: RouterConfig = serde_json::from_str(r#"{"server":{"host":"desktop"}}"#).unwrap();
    assert!(c.allow_server && c.allow_client && c.keep_local_warm);
    assert_eq!(c.prefer, Prefer::Server);
    assert_eq!(c.server.port, 7313);
    assert_eq!(c.server.connect_timeout_ms, 800);
    assert_eq!(c.server.commit_timeout_base_ms, 3000);
    assert_eq!(c.server.health_ttl_s, 30);
    let c: RouterConfig = serde_json::from_str(r#"{"prefer":"client","unknown":1}"#).unwrap();
    assert_eq!(c.prefer, Prefer::Client);
}

#[test]
fn router_produces_fixture_shaped_json() {
    let r = Router::new(RouterConfig::default(), None, None);
    let caps = serde_json::to_value(r.capabilities()).unwrap();
    let want: Value = serde_json::from_str(
        &std::fs::read_to_string(dir().join("capabilities_remote.json")).unwrap(),
    )
    .unwrap();
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&caps), keys(&want));
    let st = serde_json::to_value(r.route_status()).unwrap();
    assert!(st.get("circuit").is_some() && st.get("last").is_some());
}
