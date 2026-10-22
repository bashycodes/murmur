//! The URL vectors shared with the Android shell
//! (`tests/fixtures/url_parse_vectors.json`, read by the Kotlin
//! `UrlParseFixtureTest`) must stay true for the Rust parser the router
//! uses (review S2), so the Kotlin mirror can't drift from it unnoticed.

use flow_router::cloud::parse_base_url;
use flow_router::CloudProviderConfig;
use serde_json::Value;

fn fixture() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/url_parse_vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn provider(url: &str) -> CloudProviderConfig {
    CloudProviderConfig {
        id: "p-1".into(),
        base_url: url.into(),
        model: "m".into(),
        ..Default::default()
    }
}

#[test]
fn rejected_vectors_are_rejected_by_the_router() {
    let f = fixture();
    let bad = strs(&f["rejected"]);
    assert!(bad.len() >= 17);
    for u in bad {
        assert!(parse_base_url(&u).is_err(), "{u:?}");
        assert!(provider(&u).validate().is_err(), "{u:?}");
        assert_eq!(provider(&u).host(), "", "{u:?}");
    }
}

#[test]
fn host_vectors_match_the_router() {
    let f = fixture();
    for c in f["hosts"].as_array().unwrap() {
        let u = c["url"].as_str().unwrap();
        let b = parse_base_url(u).unwrap_or_else(|e| panic!("{u}: {e}"));
        assert_eq!(b.https, c["https"].as_bool().unwrap(), "{u}");
        assert_eq!(b.host, c["host"].as_str().unwrap(), "{u}");
        assert_eq!(provider(u).host(), b.host, "{u}");
    }
}

#[test]
fn cleartext_vectors_match_the_router() {
    let f = fixture();
    for u in strs(&f["cleartext_refused"]) {
        assert!(provider(&u).validate().is_err(), "{u}");
    }
    for u in strs(&f["cleartext_ok"]) {
        assert_eq!(provider(&u).validate(), Ok(()), "{u}");
    }
}

#[test]
fn consent_vectors_match_the_router() {
    let f = fixture();
    for c in f["consent"].as_array().unwrap() {
        let mut p = provider(c["base_url"].as_str().unwrap());
        p.consent_host = c["consent_host"].as_str().unwrap().into();
        p.consent_at_ms = 1;
        assert_eq!(
            p.consent_ok(),
            c["ok"].as_bool().unwrap(),
            "{:?}",
            p.base_url
        );
    }
}
