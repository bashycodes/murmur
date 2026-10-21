//! assemblyai adapter wire contract (upload → create → poll → delete)
//! against a mock HTTP server.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;
use flow_cloud::testing::{MockServer, Recorded, Reply};

fn healthy(polls_until_done: usize, final_body: &'static str) -> (MockServer, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let p2 = polls.clone();
    let srv = MockServer::start(move |r: &Recorded, _| match (r.method.as_str(), r.path()) {
        ("POST", "/v2/upload") => Reply::json(200, fixture("assemblyai/upload.json")),
        ("POST", "/v2/transcript") => Reply::json(200, fixture("assemblyai/queued.json")),
        ("GET", "/v2/transcript/tx-0123abcd") => {
            if p2.fetch_add(1, Ordering::SeqCst) + 1 >= polls_until_done {
                Reply::json(200, fixture(final_body))
            } else {
                Reply::json(
                    200,
                    serde_json::json!({"id":"tx-0123abcd","status":"processing"}),
                )
            }
        }
        ("DELETE", "/v2/transcript/tx-0123abcd") => Reply::json(200, serde_json::json!({})),
        _ => Reply::json(404, serde_json::json!({})),
    });
    (srv, polls)
}

#[test]
fn upload_create_poll_delete() {
    let (srv, polls) = healthy(2, "assemblyai/completed.json");
    let a = asr(provider("assemblyai", &srv, ""));
    let text = run(&a, &req(&["Kubernetes"], &["Alice"], Some("en"))).unwrap();
    assert_eq!(text, "Ask not what your country can do for you.");
    assert_eq!(polls.load(Ordering::SeqCst), 2);
    let rs = srv.requests();
    let steps: Vec<(String, String)> = rs
        .iter()
        .map(|r| (r.method.clone(), r.path().to_string()))
        .collect();
    assert_eq!(
        steps,
        [
            ("POST".into(), "/v2/upload".into()),
            ("POST".into(), "/v2/transcript".into()),
            ("GET".into(), "/v2/transcript/tx-0123abcd".into()),
            ("GET".into(), "/v2/transcript/tx-0123abcd".into()),
            ("DELETE".into(), "/v2/transcript/tx-0123abcd".into()),
        ]
    );
    for r in &rs {
        assert_eq!(r.header("authorization"), Some(KEY));
        assert_no_key_in(r, "authorization");
    }
    assert_eq!(&rs[0].body[..4], b"fLaC");
    let j = rs[1].json();
    assert_eq!(
        j["audio_url"],
        "https://cdn.assemblyai.com/upload/0123-abcd"
    );
    assert_eq!(j["speech_model"], "universal");
    assert_eq!(j["language_code"], "en");
    assert_eq!(j["keyterms_prompt"], serde_json::json!(["Kubernetes"]));
    let info = a.last_call().unwrap();
    assert_eq!(info.billed_audio_ms, 3000);
    assert_eq!(info.request_id.as_deref(), Some("tx-0123abcd"));
}

#[test]
fn auto_language_no_terms_no_delete_without_zero_retention() {
    let (srv, _) = healthy(1, "assemblyai/completed.json");
    let mut p = provider("assemblyai", &srv, "");
    p.zero_retention = false;
    run(&asr(p), &req(&[], &[], None)).unwrap();
    let rs = srv.requests();
    let j = rs[1].json();
    assert_eq!(j["language_detection"], true);
    assert!(j.get("keyterms_prompt").is_none() && j.get("language_code").is_none());
    assert!(rs.iter().all(|r| r.method != "DELETE"));
}

#[test]
fn transcript_error_status_is_provider_error_and_still_deleted() {
    let (srv, _) = healthy(1, "assemblyai/error.json");
    let e = run(&asr(provider("assemblyai", &srv, "")), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::ProviderError);
    assert!(!e.message.contains("decoded"), "no provider free text: {e}");
    assert!(srv.requests().iter().any(|r| r.method == "DELETE"));
}

#[test]
fn polling_stops_at_the_deadline() {
    let (srv, _) = healthy(usize::MAX, "assemblyai/completed.json");
    let mut p = provider("assemblyai", &srv, "");
    p.timeout_base_ms = 1000;
    let t = std::time::Instant::now();
    let e = run(&asr(p), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::Timeout);
    assert!(t.elapsed() < std::time::Duration::from_millis(2500));
}

#[test]
fn error_mapping() {
    check_error_mapping("assemblyai", "", "/v2/upload");
}

#[test]
fn transport_errors() {
    check_transport_errors("assemblyai", "");
}
