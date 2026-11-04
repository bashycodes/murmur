//! D52 `CustomCleaner` wire contract against a fake OpenAI-compatible
//! server (plain http on loopback, no base-URL override needed: the URL is
//! user-typed): streamed plain chat completions with no OpenRouter fields,
//! optional key, `/models` list, optional LiteLLM cost header, and a
//! router-level dictation through `flow_cloud::attach`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_cloud::custom::{self, CustomCleaner, COST_HEADER};
use flow_cloud::testing::{MockServer, Reply};
use flow_cloud::CloudError;
use flow_core::cleanup::{build_prompt, CleanPrompt, Cleaner, PromptScope};
use flow_core::config::{CleanupLevel, FieldContext, Style};
use flow_core::dictionary::Dictionary;
use flow_router::cloud::CustomCleanupConfig;
use flow_router::SecretString;

const KEY: &str = "sk-litellm-LEAKCHECK-contract-0001";
const MODEL: &str = "Qwen/Qwen2.5-7B-Instruct";

fn prompt() -> CleanPrompt {
    build_prompt(
        "lets meet at noon",
        CleanupLevel::Light,
        Style::default(),
        &FieldContext::default(),
        &Dictionary::default(),
    )
}

fn cfg(base: &str) -> CustomCleanupConfig {
    CustomCleanupConfig {
        base_url: base.into(),
        model: MODEL.into(),
        ..Default::default()
    }
}

fn sse_ok() -> Reply {
    Reply::sse(
        &[
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Let's meet\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\" at noon.\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ],
        Duration::from_millis(5),
    )
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn streamed_cleanup_sends_plain_openai_and_no_key_header() {
    let srv = MockServer::fixed(sse_ok());
    // Typed without /v1: normalised to {base}/v1.
    let c = CustomCleaner::new(&cfg(&srv.url("")), PromptScope::MINIMAL, None);
    assert_eq!(
        c.clean(&prompt(), deadline()).unwrap(),
        "Let's meet at noon."
    );
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/v1/chat/completions");
    assert_eq!(r.header("authorization"), None, "no key: no Authorization");
    assert_eq!(r.header("accept"), Some("text/event-stream"));
    let j = r.json();
    assert_eq!(j["model"], MODEL, "case kept");
    assert_eq!(j["stream"], true);
    assert_eq!(j["temperature"], 0);
    assert!(j["max_tokens"].as_u64().unwrap() > 0);
    let msgs = j["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), prompt().messages().len());
    assert_eq!(msgs[0]["role"], "system");
    // None of OpenRouter's extensions.
    for k in [
        "provider",
        "zdr",
        "max_price",
        "reasoning",
        "include_reasoning",
        "seed",
        "max_completion_tokens",
        "user",
        "session_id",
    ] {
        assert!(j.get(k).is_none(), "{k} sent");
    }
    assert!(!r.raw_text().contains("openrouter"));
    let call = c.last_call().unwrap();
    assert_eq!(call.cost_usd, None, "no cost header: unknown");
    assert_eq!(call.generation_id, None);
    assert_eq!(call.finish_reason.as_deref(), Some("stop"));
    // Only the one request: no /key, /generation or credits lookups.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(srv.count(), 1);
}

#[test]
fn key_is_sent_as_bearer_when_set() {
    let srv = MockServer::fixed(sse_ok());
    let c = CustomCleaner::new(
        &cfg(&srv.url("/v1/")),
        PromptScope::MINIMAL,
        Some(SecretString::new(KEY)),
    );
    c.clean(&prompt(), deadline()).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.path(), "/v1/chat/completions");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    // A blank key counts as none.
    let srv = MockServer::fixed(sse_ok());
    let c = CustomCleaner::new(
        &cfg(&srv.url("/v1")),
        PromptScope::MINIMAL,
        Some(SecretString::new("  ")),
    );
    c.clean(&prompt(), deadline()).unwrap();
    assert_eq!(srv.requests()[0].header("authorization"), None);
}

#[test]
fn litellm_cost_header_is_read_when_present() {
    let srv = MockServer::fixed(sse_ok().header(COST_HEADER, "0.000123"));
    let c = CustomCleaner::new(&cfg(&srv.url("/api/v1")), PromptScope::MINIMAL, None);
    let (r, call) = c.run(&prompt(), deadline(), &Default::default());
    assert!(r.is_ok());
    assert_eq!(srv.requests()[0].path(), "/api/v1/chat/completions");
    assert_eq!(call.cost_usd, Some(0.000123));
    assert!(!call.cost_estimated);
    let u = call.usage();
    assert_eq!(u.engine.as_deref(), Some("cloud"));
    assert_eq!(u.model.as_deref(), Some(MODEL));
}

#[test]
fn plain_json_answer_is_accepted() {
    let srv = MockServer::fixed(Reply::json(
        200,
        serde_json::json!({"id": "chatcmpl-2", "choices": [
            {"message": {"role": "assistant", "content": "Let's meet at noon."}, "finish_reason": "stop"}
        ]}),
    ));
    let c = CustomCleaner::new(&cfg(&srv.url("")), PromptScope::MINIMAL, None);
    assert_eq!(
        c.clean(&prompt(), deadline()).unwrap(),
        "Let's meet at noon."
    );
}

#[test]
fn errors_map_to_codes_and_redirects_are_not_followed() {
    let srv = MockServer::fixed(Reply::json(
        401,
        serde_json::json!({"error": {"message": "Authentication Error", "type": "auth_error"}}),
    ));
    let c = CustomCleaner::new(&cfg(&srv.url("")), PromptScope::MINIMAL, None);
    let e = c.clean(&prompt(), deadline()).unwrap_err();
    assert_eq!(e.to_string(), "auth_failed");
    assert!(matches!(e, flow_core::cleanup::CleanError::Unavailable(_)));

    let srv = MockServer::fixed(Reply::raw(307, "").header("Location", "http://127.0.0.1:9/x"));
    let c = CustomCleaner::new(&cfg(&srv.url("")), PromptScope::MINIMAL, None);
    let (r, _) = c.run(&prompt(), deadline(), &Default::default());
    assert!(matches!(r, Err(CloudError::BadRequest { status: 307, .. })));
    assert_eq!(srv.count(), 1);

    // Nothing listening: a fast connect failure (the local LLM may run).
    let dead = CustomCleaner::new(&cfg("http://127.0.0.1:9"), PromptScope::MINIMAL, None);
    let e = dead.clean(&prompt(), deadline()).unwrap_err();
    assert!(
        matches!(e, flow_core::cleanup::CleanError::Unavailable(_)),
        "{e:?}"
    );

    // An unusable URL never sends anything.
    let bad = CustomCleaner::new(&cfg("not a url"), PromptScope::MINIMAL, None);
    let (r, _) = bad.run(&prompt(), deadline(), &Default::default());
    assert!(matches!(r, Err(CloudError::Connect(_))));
}

#[test]
fn models_list_parses_ids() {
    let srv = MockServer::fixed(Reply::json(
        200,
        serde_json::json!({"object": "list", "data": [
            {"id": "gpt-4o-mini", "object": "model", "owned_by": "openai"},
            {"id": MODEL, "object": "model"},
        ]}),
    ));
    let ids = custom::list_models(&srv.url(""), None, Duration::from_secs(3)).unwrap();
    assert_eq!(ids, [MODEL, "gpt-4o-mini"]);
    let r = &srv.requests()[0];
    assert_eq!((r.method.as_str(), r.path()), ("GET", "/v1/models"));
    assert_eq!(r.header("authorization"), None);
    let key = SecretString::new(KEY);
    custom::list_models(&srv.url("/v1"), Some(&key), Duration::from_secs(3)).unwrap();
    assert_eq!(
        srv.requests()[1].header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let srv = MockServer::fixed(Reply::raw(200, "<html>"));
    assert_eq!(
        custom::list_models(&srv.url(""), None, Duration::from_secs(3)),
        Err(CloudError::Malformed)
    );
    assert!(matches!(
        custom::list_models("ftp://x", None, Duration::from_secs(1)),
        Err(CloudError::Connect(_))
    ));
}

#[test]
fn router_dictation_uses_the_custom_server() {
    use flow_core::asr::MockAsr;
    use flow_core::FlowCore;
    use flow_router::{
        Backend, CleanupProvider, CleanupRoute, Router, RouterConfig, SessionConfig,
    };

    let srv = MockServer::fixed(sse_ok());
    let mut c = RouterConfig {
        allow_server: false,
        allow_cloud_cleanup: true,
        cleanup_order: vec![CleanupRoute::Cloud, CleanupRoute::Local],
        ..Default::default()
    };
    c.cloud.cleanup_provider = CleanupProvider::Custom;
    c.cloud.custom_cleanup = cfg(&srv.url(""));
    c.cloud.custom_cleanup.consent_host = "127.0.0.1".into();
    c.cloud.custom_cleanup.consent_at_ms = 1;
    c.validate().unwrap();
    let core = FlowCore::from_engines(Arc::new(MockAsr::new("lets meet at noon")), None);
    let router = flow_cloud::attach(
        Router::new(c, Some(Arc::new(core) as Arc<dyn Backend>), None),
        None,
    );
    router
        .start_session(SessionConfig {
            llm_skip_clean: false,
            ..Default::default()
        })
        .unwrap();
    let pcm: Vec<i16> = (0..300 * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    assert!(router.push_audio(&pcm));
    let r = router.commit();
    assert!(r.final_text.contains("Let's meet at noon."), "{r:?}");
    let rt = r.route.unwrap();
    assert_eq!(rt.llm_engine.as_deref(), Some("cloud"));
    assert_eq!(rt.llm_model.as_deref(), Some(MODEL));
    // The connection is warmed with a content-free GET /models (prefill).
    let reqs = srv.requests();
    assert_eq!(
        reqs.iter()
            .filter(|q| q.path() == "/v1/chat/completions")
            .count(),
        1
    );
    for q in reqs.iter().filter(|q| q.path() != "/v1/chat/completions") {
        assert_eq!(
            (q.method.as_str(), q.path(), q.body.len()),
            ("GET", "/v1/models", 0)
        );
    }
    assert_eq!(router.cleanup_status().provider, CleanupProvider::Custom);
}
