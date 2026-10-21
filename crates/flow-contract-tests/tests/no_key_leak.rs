//! D18 C3/C4: a sentinel API key never appears in any output: router config
//! JSON and Debug, route status, health reports, capabilities, usage, commit
//! results and error strings of every failure path, the provider call
//! metadata, and the flow-protocol frames a session sends to flow-server.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_cloud::testing::{MockServer, Reply};
use flow_contract_tests::*;
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_router::{Route, Router, RouterConfig};

const NEEDLES: [&str; 2] = [CLOUD_KEY, "LEAKCHECK"];

fn assert_clean(what: &str, s: &str) {
    for n in NEEDLES {
        assert!(!s.contains(n), "key leaked in {what}: {s}");
    }
}

fn session() -> SessionConfig {
    SessionConfig {
        llm_timeout_ms: 0,
        dictionary: Dictionary {
            entries: vec![DictEntry::word("Kubernetes")],
        },
        ..Default::default()
    }
}

fn dictate(r: &Router) -> flow_core::CommitResult {
    let _ = r.start_session(session());
    for c in speech(1500).chunks(FRAME) {
        r.push_audio(c);
    }
    r.set_context(FieldContext {
        app_id: "notepad".into(),
        screen_words: vec!["Alice".into()],
        ..Default::default()
    });
    r.commit()
}

/// Everything a shell can read from the router, serialised.
fn router_outputs(r: &Router) -> Vec<(String, String)> {
    let cfg = r.config();
    let p = cfg.cloud.providers[0].clone();
    vec![
        ("config json".into(), serde_json::to_string(&cfg).unwrap()),
        ("config debug".into(), format!("{cfg:?}")),
        (
            "route_status".into(),
            serde_json::to_string(&r.route_status()).unwrap(),
        ),
        (
            "capabilities".into(),
            serde_json::to_string(&r.capabilities()).unwrap(),
        ),
        (
            "cloud_capabilities".into(),
            serde_json::to_string(&r.cloud_capabilities()).unwrap(),
        ),
        (
            "cloud_usage".into(),
            serde_json::to_string(&r.cloud_usage()).unwrap(),
        ),
        (
            "probe_cloud".into(),
            serde_json::to_string(&r.probe_cloud(&p, None)).unwrap(),
        ),
        (
            "probe_cloud debug".into(),
            format!("{:?}", r.probe_cloud(&p, None)),
        ),
    ]
}

fn cloud_only(provider: &MockServer) -> Router {
    let cfg = with_cloud(
        RouterConfig {
            allow_server: false,
            ..Default::default()
        },
        cloud_provider(&provider.url("/openai/v1")),
        &[Route::Cloud],
    );
    cloud_router(cfg, None)
}

#[test]
fn key_absent_from_every_output_on_success() {
    let provider = MockServer::fixed(Reply::json(200, serde_json::json!({"text": "hi"})));
    let r = cloud_only(&provider);
    let res = dictate(&r);
    assert_eq!(res.route.as_ref().unwrap().engine, "cloud");
    assert_clean("commit result", &serde_json::to_string(&res).unwrap());
    assert_clean("commit result debug", &format!("{res:?}"));
    for (what, s) in router_outputs(&r) {
        assert_clean(&what, &s);
    }
    // The key went to the provider only in the auth header.
    for req in provider.requests() {
        for (k, v) in &req.headers {
            if v.contains(CLOUD_KEY) {
                assert_eq!(k, "authorization");
            }
        }
        assert!(!req.target.contains(CLOUD_KEY));
        assert!(!String::from_utf8_lossy(&req.body).contains(CLOUD_KEY));
    }
}

#[test]
fn key_absent_from_every_failure_path() {
    let replies = [
        Reply::json(
            401,
            serde_json::json!({"error": {"message": format!("bad key {CLOUD_KEY}")}}),
        ),
        Reply::json(402, serde_json::json!({})),
        Reply::json(403, serde_json::json!({})),
        Reply::json(413, serde_json::json!({})),
        Reply::json(422, serde_json::json!({"error": {"code": CLOUD_KEY}})),
        Reply::json(429, serde_json::json!({})),
        Reply::json(500, serde_json::json!({})),
        Reply::json(503, serde_json::json!({})),
        // A provider that echoes the key back in a 2xx garbage body.
        Reply::raw(200, &format!("not json {CLOUD_KEY}")),
        Reply::json(200, serde_json::json!({"text": "slow"})).delayed(Duration::from_secs(10)),
    ];
    for reply in replies {
        let status = reply.status;
        let provider = MockServer::fixed(reply);
        let r = cloud_only(&provider);
        let mut cfg = r.config();
        cfg.cloud.providers[0].timeout_base_ms = 1000;
        r.update_config(cfg);
        r.set_cloud_key("groq-1", Some(flow_router::SecretString::new(CLOUD_KEY)));
        let res = dictate(&r);
        assert_eq!(res.status, flow_core::CommitStatus::Error, "HTTP {status}");
        let what = format!("HTTP {status}");
        assert_clean(&what, &serde_json::to_string(&res).unwrap());
        assert_clean(&what, res.error.as_deref().unwrap_or(""));
        // The session after a failure (circuit / sticky paths).
        let res2 = dictate(&r);
        assert_clean(&what, &serde_json::to_string(&res2).unwrap());
        for (w, s) in router_outputs(&r) {
            assert_clean(&format!("{what} {w}"), &s);
        }
    }
    // Connection refused.
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let provider = MockServer::fixed(Reply::raw(200, ""));
    let r = cloud_only(&provider);
    let mut cfg = r.config();
    cfg.cloud.providers[0].base_url = format!("http://127.0.0.1:{port}/v1");
    r.update_config(cfg);
    let res = dictate(&r);
    assert_eq!(
        res.route.unwrap().fallback.as_deref(),
        Some("connect_failed")
    );
    assert_clean("connect", res.error.as_deref().unwrap());
}

#[test]
fn provider_call_metadata_has_no_key_or_text() {
    let provider = MockServer::fixed(Reply::json(401, serde_json::json!({})));
    let b = flow_cloud::CloudBackend::new(
        cloud_provider(&provider.url("/openai/v1")),
        flow_router::SecretString::new(CLOUD_KEY),
        None,
    );
    let h = flow_router::Backend::probe(&b);
    assert_clean("probe", &serde_json::to_string(&h).unwrap());
    let info = format!("{:?}", b.last_call());
    assert_clean("last_call", &info);
}

// ---- C4: nothing cloud-related reaches flow-server ----------------------------

/// TCP proxy in front of flow-server recording client → server bytes.
fn recording_proxy(target: std::net::SocketAddr) -> (std::net::SocketAddr, Arc<Mutex<Vec<u8>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut client) = c else { break };
            let Ok(mut server) = TcpStream::connect(target) else {
                break;
            };
            let (mut c2, mut s2) = (client.try_clone().unwrap(), server.try_clone().unwrap());
            let log3 = log2.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 16384];
                while let Ok(n) = c2.read(&mut buf) {
                    if n == 0 || s2.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    log3.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            });
            std::thread::spawn(move || {
                let mut buf = [0u8; 16384];
                while let Ok(n) = server.read(&mut buf) {
                    if n == 0 || client.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    (addr, log)
}

/// Unmasked text payloads of the client → server WebSocket stream, after
/// the HTTP upgrade request (returned first).
fn ws_texts(raw: &[u8]) -> (String, Vec<String>) {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(raw.len());
    let upgrade = String::from_utf8_lossy(&raw[..end]).to_string();
    let mut texts = Vec::new();
    let mut i = end;
    while i + 2 <= raw.len() {
        let op = raw[i] & 0x0f;
        let masked = raw[i + 1] & 0x80 != 0;
        let mut len = (raw[i + 1] & 0x7f) as usize;
        let mut j = i + 2;
        if len == 126 {
            len = u16::from_be_bytes([raw[j], raw[j + 1]]) as usize;
            j += 2;
        } else if len == 127 {
            len = u64::from_be_bytes(raw[j..j + 8].try_into().unwrap()) as usize;
            j += 8;
        }
        let mask = if masked {
            let m = [raw[j], raw[j + 1], raw[j + 2], raw[j + 3]];
            j += 4;
            Some(m)
        } else {
            None
        };
        if j + len > raw.len() {
            break;
        }
        let mut p = raw[j..j + len].to_vec();
        if let Some(m) = mask {
            for (k, b) in p.iter_mut().enumerate() {
                *b ^= m[k % 4];
            }
        }
        if op == 1 {
            texts.push(String::from_utf8_lossy(&p).to_string());
        }
        i = j + len;
    }
    (upgrade, texts)
}

#[test]
fn start_frame_has_no_cloud_fields() {
    let server = spawn_mock_server("hello");
    let (proxy, log) = recording_proxy(server.addr());
    let provider = MockServer::fixed(Reply::json(200, serde_json::json!({"text": "cloud"})));
    // Server first, cloud configured and allowed as the fallback.
    let cfg = with_cloud(
        router_config(proxy, true, false),
        cloud_provider(&provider.url("/openai/v1")),
        &[Route::Server, Route::Cloud],
    );
    let r = cloud_router(cfg, None);
    let res = dictate(&r);
    assert_eq!(res.route.as_ref().unwrap().engine, "remote", "{res:?}");
    assert_eq!(provider.count(), 0);
    std::thread::sleep(Duration::from_millis(100));
    let raw = log.lock().unwrap().clone();
    let (upgrade, texts) = ws_texts(&raw);
    assert!(upgrade.starts_with("GET "), "{upgrade}");
    assert_clean("ws upgrade", &upgrade);
    let start = texts
        .iter()
        .find(|t| t.contains("\"start\""))
        .unwrap_or_else(|| panic!("no start frame in {texts:?}"));
    for t in &texts {
        assert_clean("ws frame", t);
        for f in ["cloud", "provider", "openai", "groq", "api_key"] {
            assert!(!t.contains(f), "{f} in frame {t}");
        }
    }
    // The start frame carries the session config (dictionary) only.
    assert!(start.contains("Kubernetes"), "{start}");
    assert_clean("raw ws bytes", &String::from_utf8_lossy(&raw));
}
