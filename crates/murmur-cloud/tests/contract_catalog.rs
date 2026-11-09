//! Catalogue, `/key` and `/generation` against a mock OpenRouter (D28 §2.2,
//! §2.6, §5.1).

mod common;

use std::time::{Duration, Instant};

use common::*;
use murmur_cloud::catalog::{self, CatalogSource, CatalogStore, Filters, Use};
use murmur_cloud::testing::Reply;
use murmur_cloud::{Api, SttPath};
use murmur_router::SecretString;

fn healthy(r: &murmur_cloud::testing::Recorded) -> Reply {
    let q = r.query_all("output_modalities");
    match r.path() {
        "/api/v1/models" if q == ["transcription"] => {
            Reply::json(200, fixture("models_transcription.json"))
        }
        "/api/v1/models" => Reply::json(200, fixture("models_all.json")),
        "/api/v1/models/user" => Reply::json(
            200,
            serde_json::json!({"data": [{"id": "openai/whisper-large-v3-turbo"}]}),
        ),
        "/api/v1/endpoints/zdr" => Reply::json(200, fixture("endpoints_zdr.json")),
        _ => Reply::json(404, serde_json::json!({})),
    }
}

#[test]
fn refresh_joins_models_zdr_and_account() {
    let srv = mock(|r, _| healthy(r));
    let c = catalog::refresh(&Api::new(), None).unwrap();
    assert_eq!(c.source, CatalogSource::Live);
    assert!(!c.account_filtered);
    let w = c.get("openai/whisper-large-v3-turbo").unwrap();
    assert_eq!(w.stt_path, Some(SttPath::Transcriptions));
    assert!(w.zdr && w.fast_zdr_providers == ["groq"]);
    assert_eq!(w.allowed_by_account, None);
    let g = c.get("openai/gpt-transcribe").unwrap();
    assert!(!g.zdr, "the old default has no ZDR endpoint");
    assert!(!c.get("microsoft/mai-transcribe-2").unwrap().price_unit_ok);
    let llama = c.get("meta-llama/llama-3.3-70b-instruct").unwrap();
    assert!(llama.is_cleanup() && llama.zdr);
    assert!(c
        .get("openrouter/auto")
        .is_some_and(|m| m.router && !m.is_cleanup()));
    // Unauthenticated: no key header, no /models/user.
    for r in srv.requests() {
        assert!(r.header("authorization").is_none(), "{}", r.target);
        assert_ne!(r.path(), "/api/v1/models/user");
    }
    // Picker: ZDR on by default hides gpt-transcribe.
    let ids: Vec<_> = c
        .search("", Use::Stt, Filters::default())
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(ids[0], "openai/whisper-large-v3-turbo");
    assert!(!ids.contains(&"openai/gpt-transcribe".to_string()));
}

#[test]
fn refresh_with_key_marks_account_exclusions() {
    let srv = mock(|r, _| healthy(r));
    let key = SecretString::new(KEY);
    let c = catalog::refresh(&Api::new(), Some(&key)).unwrap();
    assert!(c.account_filtered);
    assert_eq!(
        c.get("openai/whisper-large-v3-turbo")
            .unwrap()
            .allowed_by_account,
        Some(true)
    );
    let llama = c.get("meta-llama/llama-3.3-70b-instruct").unwrap();
    assert_eq!(llama.allowed_by_account, Some(false));
    assert!(llama
        .badges(Use::Cleanup)
        .contains(&"not allowed by your OpenRouter privacy settings"));
    let user: Vec<_> = srv
        .requests()
        .into_iter()
        .filter(|r| r.path() == "/api/v1/models/user")
        .collect();
    assert_eq!(user.len(), 1);
    assert_no_key_in(&user[0], "authorization");
    for r in srv
        .requests()
        .iter()
        .filter(|r| r.path() != "/api/v1/models/user")
    {
        assert!(
            r.header("authorization").is_none(),
            "key only for /models/user"
        );
    }
}

#[test]
fn pagination_follows_same_origin_only() {
    let srv = mock(|r, _| {
        if r.path() == "/api/v1/models" && r.query_all("offset") == ["1"] {
            return Reply::json(
                200,
                serde_json::json!({"data": [{"id": "x/page-two", "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}}]}),
            );
        }
        if r.path() == "/api/v1/models" && r.query_all("output_modalities") == ["all"] {
            return Reply::json(
                200,
                serde_json::json!({
                    "data": [{"id": "x/page-one", "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}}],
                    "links": {"next": "/api/v1/models?output_modalities=all&offset=1"}
                }),
            );
        }
        if r.path() == "/api/v1/models" {
            return Reply::json(
                200,
                serde_json::json!({
                    "data": [],
                    "links": {"next": "https://evil.example/api/v1/models?steal=1"}
                }),
            );
        }
        Reply::json(200, serde_json::json!({"data": []}))
    });
    let c = catalog::refresh(&Api::new(), None).unwrap();
    assert!(c.get("x/page-one").is_some() && c.get("x/page-two").is_some());
    assert!(srv.requests().iter().all(|r| !r.target.contains("steal")));
}

#[test]
fn zdr_list_failure_degrades_models_failure_errors() {
    let _srv = mock(|r, _| {
        if r.path() == "/api/v1/endpoints/zdr" {
            return Reply::json(500, serde_json::json!({}));
        }
        healthy(r)
    });
    let c = catalog::refresh(&Api::new(), None).unwrap();
    assert!(c.models.iter().all(|m| !m.zdr), "ZDR unknown = not ZDR");
    drop(_srv);
    let _srv = mock_fixed(Reply::json(500, serde_json::json!({})));
    assert!(catalog::refresh(&Api::new(), None).is_err());
}

#[test]
fn oversized_answer_is_refused() {
    let big = format!(
        "{{\"data\":[],\"pad\":\"{}\"}}",
        "x".repeat(9 * 1024 * 1024)
    );
    let _srv = mock(move |r, _| {
        if r.path() == "/api/v1/models" {
            return Reply::raw(200, &big);
        }
        Reply::json(200, serde_json::json!({"data": []}))
    });
    assert!(catalog::refresh(&Api::new(), None).is_err());
}

#[test]
fn store_refresh_writes_the_cache_and_falls_back() {
    let dir = std::env::temp_dir().join(format!("murmur-cloud-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join(catalog::CACHE_FILE_NAME);
    let _ = std::fs::remove_file(&f);
    {
        let _srv = mock(|r, _| healthy(r));
        let s = CatalogStore::new(&f);
        assert_eq!(s.current().source, CatalogSource::Baked);
        let c = s.refresh(None, false).unwrap();
        assert_eq!(c.source, CatalogSource::Live);
        assert!(f.is_file());
        // Fresh: no second fetch.
        let n = _srv.count();
        s.refresh(None, false).unwrap();
        assert_eq!(_srv.count(), n);
        s.refresh(None, true).unwrap();
        assert!(_srv.count() > n);
    }
    // Offline: the cache is used.
    let _dead = mock_fixed(Reply::json(500, serde_json::json!({})));
    let s = CatalogStore::new(&f);
    assert_eq!(s.current().source, CatalogSource::Cache);
    assert!(s.refresh(None, true).is_err());
    assert_eq!(
        s.current().source,
        CatalogSource::Cache,
        "error keeps the old one"
    );
    let st: serde_json::Value = serde_json::from_str(&s.status_json()).unwrap();
    assert_eq!(st["source"], "Cache");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lookup_unknown_model_by_endpoints() {
    let srv = mock_fixed(Reply::json(
        200,
        serde_json::json!({"data": {
            "id": "acme/new-asr", "name": "Acme ASR",
            "architecture": {"input_modalities": ["audio"], "output_modalities": ["transcription"]},
            "endpoints": [{"pricing": {"prompt": "0.000005"}}]
        }}),
    ));
    let m = catalog::lookup_model(&Api::new(), "acme/new-asr:nitro").unwrap();
    assert_eq!(m.id, "acme/new-asr:nitro");
    assert_eq!(m.stt_path, Some(SttPath::Transcriptions));
    assert_eq!(m.price.per_audio_second, Some(0.000005));
    assert_eq!(
        srv.requests()[0].path(),
        "/api/v1/models/acme/new-asr/endpoints"
    );
    assert!(catalog::lookup_model(&Api::new(), "../../etc").is_err());
    assert_eq!(srv.count(), 1, "invalid id sends nothing");
}

#[test]
fn lookup_takes_supported_parameters_from_endpoints() {
    // Optional-reasoning models (nvidia/nemotron-3.5-lightning) reason by
    // default; without the endpoints' parameter list the cleaner sends no
    // `reasoning` and the whole token budget goes to hidden reasoning.
    let _srv = mock_fixed(Reply::json(
        200,
        serde_json::json!({"data": {
            "id": "acme/thinker", "name": "Acme Thinker",
            "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]},
            "endpoints": [
                {"pricing": {"prompt": "0.0000001", "completion": "0.0000002"},
                 "supported_parameters": ["max_tokens", "temperature"]},
                {"pricing": {"prompt": "0.0000002", "completion": "0.0000004"},
                 "supported_parameters": ["max_tokens", "reasoning", "include_reasoning"]}
            ]
        }}),
    ));
    let m = catalog::lookup_model(&Api::new(), "acme/thinker").unwrap();
    assert!(m.supports.reasoning && m.supports.include_reasoning && m.supports.temperature);
    assert_eq!(m.reasoning_effort(), Some("none"));
}

#[test]
fn key_info() {
    let srv = mock_seq(vec![
        Reply::json(200, fixture("key_ok.json")),
        Reply::json(200, fixture("key_no_limit.json")),
        Reply::json(401, fixture("err_401.json")),
    ]);
    let key = SecretString::new(KEY);
    let d = || Instant::now() + Duration::from_secs(3);
    let k = murmur_cloud::keyinfo::fetch_with(&Api::new(), &key, d()).unwrap();
    assert_eq!(k.limit, Some(10.0));
    assert_eq!(k.limit_remaining, Some(7.25));
    assert_eq!(k.usage_monthly, Some(2.75));
    assert_eq!(k.free_requests_remaining, Some(997));
    assert!(!k.warn_no_limit());
    let r = &srv.requests()[0];
    assert_eq!(r.path(), "/api/v1/key");
    assert_no_key_in(r, "authorization");
    let k = murmur_cloud::keyinfo::fetch_with(&Api::new(), &key, d()).unwrap();
    assert!(k.warn_no_limit() && k.is_free_tier);
    let e = murmur_cloud::keyinfo::fetch_with(&Api::new(), &key, d()).unwrap_err();
    assert_eq!(e.code(), "auth_failed");
    let js = serde_json::to_string(&k).unwrap();
    assert!(!js.contains(KEY));
}

#[test]
fn generation_lookup() {
    let srv = mock_fixed(Reply::json(200, fixture("generation_ok.json")));
    let key = SecretString::new(KEY);
    let g = murmur_cloud::generation::fetch(
        &Api::new(),
        &key,
        "gen-stream-1",
        Instant::now() + Duration::from_secs(3),
    )
    .unwrap();
    assert_eq!(g.total_cost, Some(0.0000058));
    assert!(g.cancelled);
    assert_eq!(srv.requests()[0].query_all("id"), ["gen-stream-1"]);
    assert!(murmur_cloud::generation::fetch(
        &Api::new(),
        &key,
        "bad id&x=1",
        Instant::now() + Duration::from_secs(1)
    )
    .is_err());
    assert_eq!(srv.count(), 1);
}
