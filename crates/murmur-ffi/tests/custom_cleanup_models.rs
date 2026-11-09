//! D52 `json_api::custom_cleanup_models` (`nativeCustomCleanupModels`):
//! `{"models": [...]}` from a fake OpenAI-compatible server, the typed key
//! sent as bearer, `{"error", "message"}` without the key or URL.
#![cfg(feature = "cloud")]

use flow_cloud::testing::{MockServer, Reply};
use flow_ffi::json_api::custom_cleanup_models;

#[test]
fn lists_models_and_reports_errors_without_secrets() {
    let srv = MockServer::fixed(Reply::json(
        200,
        serde_json::json!({"data": [{"id": "gpt-4o-mini"}, {"id": "Llama-3.1-8B"}]}),
    ));
    let v: serde_json::Value = serde_json::from_str(&custom_cleanup_models(
        None,
        &srv.url(""),
        Some("sk-ffi-LEAKCHECK-01".into()),
    ))
    .unwrap();
    assert_eq!(
        v["models"],
        serde_json::json!(["Llama-3.1-8B", "gpt-4o-mini"])
    );
    assert_eq!(
        srv.requests()[0].header("authorization"),
        Some("Bearer sk-ffi-LEAKCHECK-01")
    );
    let srv = MockServer::fixed(Reply::json(401, serde_json::json!({"error": "nope"})));
    let out = custom_cleanup_models(None, &srv.url("/v1"), Some("sk-ffi-LEAKCHECK-02".into()));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"], "auth_failed");
    assert!(
        !out.contains("LEAKCHECK") && !out.contains("127.0.0.1"),
        "{out}"
    );
    let v: serde_json::Value =
        serde_json::from_str(&custom_cleanup_models(None, "not a url", None)).unwrap();
    assert_eq!(v["error"], "connect_failed");
}

#[test]
fn address_is_normalised_by_the_router() {
    use flow_ffi::json_api::custom_cleanup_address;
    let v: serde_json::Value = serde_json::from_str(&custom_cleanup_address(
        " http://[2001:DB8::0:1]:4000/ ",
        "m-1",
    ))
    .unwrap();
    assert_eq!(v["url"], "http://[2001:db8::1]:4000/v1");
    assert_eq!(v["host"], "2001:db8::1");
    assert_eq!(v["model_ok"], true);
    let v: serde_json::Value =
        serde_json::from_str(&custom_cleanup_address("http://x/v1?k=1", "two words")).unwrap();
    assert_eq!(v["error"], "bad_base_url");
    assert_eq!(v["model_ok"], false);
    assert!(v.get("host").is_none());
}
