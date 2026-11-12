//! D78: the LiteLLM Cloud provider against a **real** LiteLLM proxy.
//! Ignored by default (needs a running proxy); run with
//!
//! ```sh
//! MURMUR_LITELLM_URL=http://host:4000 MURMUR_LITELLM_MODEL=<model> \
//! MURMUR_LITELLM_KEY=<key, optional> \
//!   cargo test -p murmur-cloud --test litellm_live -- --ignored --nocapture
//! ```
//!
//! It checks what the settings page and a dictation do: the model list
//! from `/models` (with and without the key), "Test cleanup", a whole
//! dictation through the router's cloud cleanup route, and the plain-words
//! errors for a wrong key and an unknown model. It prints latencies and
//! the shape of the answers, never the key.
//!
//! D84: with `MURMUR_LITELLM_STT_MODEL=<audio_transcription model>` also
//! `litellm_proxy_speech_to_text`: speech-model detection from
//! `/model/info`, a transcription of the bundled public-domain clip, the
//! Speech page's Test, and a whole dictation on the proxy's speech route
//! (`--ignored litellm_proxy_speech_to_text`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use murmur_cloud::{custom, CloudAsr, TEST_CLIP};
use murmur_core::asr::MockAsr;
use murmur_core::asr::{AsrEngine, AsrPurpose, AsrRequest};
use murmur_core::MurmurCore;
use murmur_router::cloud::{custom_server_address, CustomCleanupConfig};
use murmur_router::{
    Backend, CleanupRoute, CloudProvider, Route, Router, RouterConfig, SecretString, SessionConfig,
};

/// A made-up dictation with fillers and a self-correction.
const SAID: &str =
    "um so the review is on tuesday no wait thursday at three and uh bring the slides";

struct Live {
    url: String,
    model: String,
    key: Option<SecretString>,
}

fn live() -> Live {
    let url = std::env::var("MURMUR_LITELLM_URL").expect("MURMUR_LITELLM_URL");
    let model = std::env::var("MURMUR_LITELLM_MODEL").expect("MURMUR_LITELLM_MODEL");
    let key = std::env::var("MURMUR_LITELLM_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(SecretString::new);
    Live { url, model, key }
}

fn cfg(l: &Live, model: &str) -> CustomCleanupConfig {
    let a = custom_server_address(&l.url).expect("usable proxy address");
    CustomCleanupConfig {
        base_url: a.url,
        model: model.into(),
        // A small local model behind the proxy on a shared box.
        timeout_ms: 30_000,
        consent_host: a.host,
        consent_at_ms: 1,
        ..Default::default()
    }
}

/// Shape of a text for the report: length and word count, not the words.
fn shape(s: &str) -> String {
    format!("{} chars, {} words", s.len(), s.split_whitespace().count())
}

#[test]
#[ignore = "needs a running LiteLLM proxy (MURMUR_LITELLM_URL / _MODEL / _KEY)"]
fn litellm_proxy_end_to_end() {
    let l = live();
    let secret = l.key.as_ref().map(|k| k.expose().to_string());
    let no_secret = |s: &str| {
        if let Some(k) = &secret {
            assert!(!s.contains(k.as_str()), "the key leaked into output");
        }
    };

    // 1. Settings page: Test connection / model list from /models.
    let t0 = Instant::now();
    let models = custom::list_models(&l.url, l.key.as_ref(), Duration::from_secs(10))
        .expect("the proxy lists its models");
    println!(
        "models: {} ids in {} ms",
        models.len(),
        t0.elapsed().as_millis()
    );
    assert!(models.contains(&l.model), "{models:?}");

    // 2. Without the key a keyed proxy says so in plain words.
    if l.key.is_some() {
        let j = custom::models_json(custom::list_models(&l.url, None, Duration::from_secs(10)));
        println!("models without key: {j}");
        assert_eq!(j["error"], "auth_failed", "{j}");
        assert_eq!(j["message"], "the server rejected the key, or needs one");
        let j = custom::models_json(custom::list_models(
            &l.url,
            Some(&SecretString::new("sk-not-the-key")),
            Duration::from_secs(10),
        ));
        println!("models with a wrong key: {j}");
        assert_eq!(j["error"], "auth_failed", "{j}");
    }

    // 3. "Test cleanup" on the page: one streamed chat completion.
    let t = custom::test_cleanup(&cfg(&l, &l.model), l.key.clone());
    let out = serde_json::to_string(&t).unwrap();
    no_secret(&out);
    println!(
        "test cleanup: ok={} latency={:?} ms cost={:?} text: {}",
        t.ok,
        t.latency_ms,
        t.cost_usd,
        t.text.as_deref().map(shape).unwrap_or_default()
    );
    assert!(t.ok, "{out}");
    assert!(!t.text.as_deref().unwrap_or("").trim().is_empty());

    // 4. An unknown model: a plain-words error, not the proxy's body.
    let bad = custom::test_cleanup(&cfg(&l, "no-such-model-d78"), l.key.clone());
    println!("unknown model: {:?} / {:?}", bad.error, bad.message);
    assert!(!bad.ok);
    assert_eq!(
        bad.message.as_deref(),
        Some("the server has no model by that name: pick one from its list")
    );

    // 5. Whole dictations: router cloud cleanup route on the LiteLLM
    // provider (the speech engine is a fixed mock transcript). A small model
    // on a busy CPU can need longer than the 30 s cap for the first
    // (uncached) cleanup prompt; that timeout opens the circuit, so each
    // attempt gets a fresh router and the first may only warm the proxy.
    let new_router = || {
        let mut c = RouterConfig {
            allow_server: false,
            allow_cloud_cleanup: true,
            cleanup_order: vec![CleanupRoute::Cloud, CleanupRoute::Local],
            ..Default::default()
        };
        c.cloud.provider = CloudProvider::LiteLlm;
        c.cloud.custom_cleanup = cfg(&l, &l.model);
        c.validate().unwrap();
        let core = MurmurCore::from_engines(Arc::new(MockAsr::new(SAID)), None);
        let router = murmur_cloud::attach(
            Router::new(c, Some(Arc::new(core) as Arc<dyn Backend>), None),
            None,
        );
        router.set_cloud_key("custom", l.key.clone());
        router
    };
    let mut cleaned = 0;
    let mut router = new_router();
    for i in 0..4 {
        if router.cleanup_status().circuit.is_some_and(|c| c.open) {
            router = new_router();
        }
        router
            .start_session(SessionConfig {
                llm_skip_clean: false,
                llm_timeout_ms: 30_000,
                ..Default::default()
            })
            .unwrap();
        let pcm: Vec<i16> = (0..300 * 16)
            .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
            .collect();
        assert!(router.push_audio(&pcm));
        let t0 = Instant::now();
        let r = router.commit();
        let ms = t0.elapsed().as_millis();
        let rt = r.route.clone().unwrap();
        println!(
            "dictation {i}: {ms} ms, llm {:?} {:?} {} ms, final: {}",
            rt.llm_engine,
            rt.llm_model,
            r.timings.llm_ms,
            shape(&r.final_text)
        );
        no_secret(&format!("{r:?}"));
        // Whatever happens, the dictation is never lost.
        assert!(!r.final_text.trim().is_empty(), "{r:?}");
        if rt.llm_engine.as_deref() == Some("cloud") {
            assert_eq!(rt.llm_model.as_deref(), Some(l.model.as_str()));
            assert_ne!(r.final_text.trim(), SAID, "cleanup changed the text");
            cleaned += 1;
        } else {
            println!("  fell back: {:?}", rt.cloud_fallback);
        }
    }
    assert!(
        cleaned >= 2,
        "{cleaned} of 4 dictations cleaned by the proxy"
    );
    let st = router.cleanup_status();
    println!(
        "cleanup status: provider {:?}, host {}, consent {}, blocked {:?}, circuit open {:?}",
        st.provider,
        st.host,
        st.consent,
        st.blocked,
        st.circuit.as_ref().map(|c| c.open)
    );
    assert_eq!(st.provider, CloudProvider::LiteLlm);
    assert!(st.blocked.is_none());
    println!(
        "month cost: {} total, {} reported by the proxy",
        st.month_cost_usd.0, st.custom_month_cost_usd.0
    );
    // Whatever the proxy reports is its own; none of it is OpenRouter's.
    assert_eq!(st.month_cost_usd.0, st.custom_month_cost_usd.0);
}

fn clip() -> Vec<i16> {
    TEST_CLIP
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

#[test]
#[ignore = "needs a LiteLLM proxy with a speech model (MURMUR_LITELLM_STT_MODEL)"]
fn litellm_proxy_speech_to_text() {
    let l = live();
    let stt = std::env::var("MURMUR_LITELLM_STT_MODEL").expect("MURMUR_LITELLM_STT_MODEL");
    let secret = l.key.as_ref().map(|k| k.expose().to_string());
    let no_secret = |s: &str| {
        if let Some(k) = &secret {
            assert!(!s.contains(k.as_str()), "the key leaked into output");
        }
    };
    let mut c = cfg(&l, &l.model);
    c.stt_model = stt.clone();
    c.stt_consent_host = c.consent_host.clone();
    c.stt_consent_at_ms = 1;

    // 1. Test connection: the speech model is found by its mode.
    let j = custom::models_with_speech_json(&l.url, l.key.as_ref(), Duration::from_secs(10));
    println!("speech models: {}", j["speech_models"]);
    assert!(
        j["speech_models"]
            .as_array()
            .is_some_and(|a| a.iter().any(|m| m == stt.as_str())),
        "{j}"
    );
    if l.key.is_some() {
        let j = custom::models_with_speech_json(&l.url, None, Duration::from_secs(10));
        assert_eq!(j["error"], "auth_failed", "{j}");
    }

    // 2. One transcription of the public-domain clip.
    let asr = CloudAsr::litellm(&c, l.key.clone());
    let pcm: Vec<f32> = clip().iter().map(|s| *s as f32 / 32768.0).collect();
    let req = AsrRequest {
        language: Some("en".into()),
        purpose: AsrPurpose::Final,
        dictionary_terms: vec!["Americans".into()],
        ..Default::default()
    };
    let t0 = Instant::now();
    let out = asr.transcribe(&pcm, &req).expect("the proxy transcribes");
    let info = asr.last_call().unwrap();
    no_secret(&format!("{info:?}"));
    println!(
        "transcription: {} ms, cost {:?} (estimated {}), request id {}, text: {:?}",
        t0.elapsed().as_millis(),
        info.cost_usd,
        info.cost_estimated,
        info.request_id.is_some(),
        out.text
    );
    assert!(out.text.to_lowercase().contains("fellow"), "{:?}", out.text);
    assert_eq!(info.provider_id, "custom");

    // 3. An unknown speech model: a plain-words error.
    let mut bad = c.clone();
    bad.stt_model = "no-such-model-d84".into();
    let e = CloudAsr::litellm(&bad, l.key.clone())
        .transcribe(&pcm, &req)
        .unwrap_err();
    println!("unknown speech model: {e}");

    // 4. A whole dictation on the proxy's speech route, and the Speech
    // page's Test.
    let mut rc = RouterConfig {
        allow_cloud: true,
        allow_server: false,
        order: vec![Route::Cloud],
        ..Default::default()
    };
    rc.cloud.provider = CloudProvider::LiteLlm;
    rc.cloud.custom_cleanup = c.clone();
    rc.validate().unwrap();
    let router = murmur_cloud::attach(Router::new(rc, None, None), None);
    router.set_cloud_key("custom", l.key.clone());
    let h = router.probe_custom_stt(&c, None);
    println!(
        "speech test: ok={} {:?} ms {:?}",
        h.ok, h.latency_ms, h.transcript
    );
    assert!(h.ok, "{h:?}");
    router.start_session(SessionConfig::default()).unwrap();
    for ch in clip().chunks(1600) {
        router.push_audio(ch);
    }
    let r = router.commit();
    let rt = r.route.clone().unwrap();
    no_secret(&format!("{r:?}"));
    println!(
        "dictation: engine {} provider {:?} model {:?} cost {:?}, asr: {:?}",
        rt.engine, rt.provider, rt.model, rt.cost_usd, r.asr_text
    );
    assert_eq!(rt.engine, "cloud");
    assert_eq!(rt.provider.as_deref(), Some("custom"));
    assert!(r.asr_text.to_lowercase().contains("fellow"), "{r:?}");
    let u = router.cloud_usage();
    println!(
        "usage: proxy {} USD, OpenRouter budget {} USD",
        u.custom_cost(),
        u.budget_cost()
    );
    assert_eq!(u.budget_cost(), 0.0);
}
