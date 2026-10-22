//! Review S3: provider calls never follow a redirect. A 3xx answer is a
//! `provider_error`, and nothing (key header, audio) reaches the target.

mod common;

use common::*;
use flow_cloud::testing::{MockServer, Reply};
use flow_router::Failure;

#[test]
fn no_adapter_follows_a_redirect() {
    let cases = [
        ("groq", "/openai/v1"),
        ("openrouter", "/api/v1"),
        ("elevenlabs", ""),
        ("deepgram", ""),
        ("assemblyai", ""),
    ];
    for status in [302u16, 307, 308] {
        for (preset, base) in cases {
            let target = MockServer::fixed(Reply::json(200, serde_json::json!({"text": "x"})));
            let loc = target.url("/steal");
            let srv = MockServer::fixed(Reply::raw(status, "").header("location", &loc));
            let e = run(
                &asr(provider(preset, &srv, base)),
                &req(&["term"], &[], None),
            )
            .unwrap_err();
            assert_eq!(e.kind, Failure::ProviderError, "{preset} {status}: {e}");
            assert_eq!(srv.count(), 1, "{preset} {status}: not retried");
            assert_eq!(
                target.count(),
                0,
                "{preset} {status}: redirect followed (key/audio leaked)"
            );
        }
    }
}
