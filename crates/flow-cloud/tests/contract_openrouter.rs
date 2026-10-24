//! OpenRouter STT wire contract (D28 §2.4, §2.7, §5.1) against a mock
//! OpenRouter: both paths, privacy defaults, biasing, chunking, the WAV
//! retry, the chat-audio sanity check, the error table and deferred cost.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use common::*;
use flow_cloud::testing::Reply;
use flow_cloud::{SttPath, SttPathChoice, TRANSCRIBE_PROMPT};
use flow_core::asr::AsrEngine;
use flow_router::Failure;

fn b64(v: &serde_json::Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(v.as_str().unwrap())
        .unwrap()
}

#[test]
fn path_a_request_shape_and_reported_cost() {
    let srv = mock_fixed(
        Reply::json(200, fixture("transcriptions_ok.json")).header("X-Generation-Id", "gen-abc123"),
    );
    let a = whisper();
    assert_eq!(a.path(), SttPath::Transcriptions);
    let text = run(&a, &req(&["Kubernetes"], &["Alice"], Some("en"))).unwrap();
    assert_eq!(text, "Ask not what your country can do for you.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/api/v1/audio/transcriptions");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let j = r.json();
    assert_eq!(j["model"], "openai/whisper-large-v3-turbo");
    assert_eq!(j["language"], "en");
    assert_eq!(j["response_format"], "json");
    assert_eq!(j["temperature"], 0);
    assert_eq!(j["input_audio"]["format"], "flac");
    let audio = b64(&j["input_audio"]["data"]);
    assert_eq!(&audio[..4], b"fLaC");
    // Most private routing by default.
    assert_eq!(j["provider"]["zdr"], true);
    assert_eq!(j["provider"]["data_collection"], "deny");
    // Dictionary → Groq whisper prompt; screen words never.
    assert_eq!(
        j["provider"]["options"]["groq"]["prompt"],
        "Vocabulary: Kubernetes."
    );
    assert!(!r.raw_text().contains("Alice"));
    for k in ["user", "session_id"] {
        assert!(j.get(k).is_none(), "{k} sent");
    }
    assert_no_key_in(r, "authorization");
    let info = a.last_call().unwrap();
    assert_eq!(info.cost_usd, Some(0.000225));
    assert!(!info.cost_estimated);
    assert_eq!(info.billed_audio_ms, 3000);
    assert_eq!(info.request_id.as_deref(), Some("gen-abc123"));
    assert_eq!(info.stt_path, "transcriptions");
    assert_eq!(info.provider_id, "openrouter");
}

#[test]
fn flac_upload_round_trips_bit_exact() {
    let srv = mock_fixed(Reply::json(200, fixture("transcriptions_ok.json")));
    let pcm = speech(1000);
    whisper().transcribe(&pcm, &req(&[], &[], None)).unwrap();
    let flac = b64(&srv.requests()[0].json()["input_audio"]["data"]);
    let mut r = claxon::FlacReader::new(std::io::Cursor::new(flac)).unwrap();
    let got: Vec<i16> = r.samples().map(|s| s.unwrap() as i16).collect();
    assert_eq!(got, flow_cloud::encode::to_i16(&pcm));
}

#[test]
fn privacy_off_and_dictionary_off_omit_the_provider_block() {
    let srv = mock_fixed(Reply::json(200, fixture("transcriptions_ok.json")));
    let mut s = settings(flow_cloud::DEFAULT_STT_MODEL);
    s.privacy.zdr = false;
    s.privacy.data_collection = "allow".into();
    s.send_dictionary = false;
    run(&asr(s), &req(&["Kubernetes"], &[], None)).unwrap();
    let j = srv.requests()[0].json();
    assert!(
        j.get("provider").is_none() && j.get("language").is_none(),
        "{j}"
    );
}

#[test]
fn non_whisper_models_get_no_terms() {
    let srv = mock_fixed(Reply::json(200, fixture("transcriptions_ok.json")));
    run(
        &asr(settings("deepgram/nova-3")),
        &req(&["Kubernetes"], &[], None),
    )
    .unwrap();
    let r = &srv.requests()[0];
    assert!(!r.raw_text().contains("Kubernetes"));
    assert!(r.json()["provider"].get("options").is_none());
}

#[test]
fn long_audio_is_chunked_below_the_60_s_upstream_limit() {
    let srv = mock(|_, i| {
        Reply::json(
            200,
            serde_json::json!({"text": format!("part{i}"), "usage": {"seconds": 1.0, "cost": 0.001}}),
        )
    });
    let a = whisper();
    let out = a.transcribe(&speech(70_000), &req(&[], &[], None)).unwrap();
    assert_eq!(out.text, "part0 part1");
    assert_eq!(srv.count(), 2);
    let info = a.last_call().unwrap();
    assert_eq!(info.requests, 2);
    assert!((info.cost_usd.unwrap() - 0.002).abs() < 1e-12, "costs sum");
    assert_eq!(info.billed_audio_ms, 2000);
}

#[test]
fn format_400_retries_once_with_wav_and_remembers() {
    let model = "mistralai/voxtral-mini-transcribe-3-0";
    let srv = mock_seq(vec![
        Reply::json(400, fixture("err_400_format.json")),
        Reply::json(200, fixture("transcriptions_ok.json")),
    ]);
    let a = asr(settings(model));
    run(&a, &req(&[], &[], None)).unwrap();
    let rs = srv.requests();
    assert_eq!(rs.len(), 2, "exactly one retry");
    assert_eq!(rs[0].json()["input_audio"]["format"], "flac");
    assert_eq!(rs[1].json()["input_audio"]["format"], "wav");
    assert_eq!(&b64(&rs[1].json()["input_audio"]["data"])[..4], b"RIFF");
    // Remembered for the model: the next backend starts with WAV.
    run(&asr(settings(model)), &req(&[], &[], None)).unwrap();
    assert_eq!(srv.requests()[2].json()["input_audio"]["format"], "wav");
}

#[test]
fn format_400_twice_is_an_error_not_a_loop() {
    let srv = mock_fixed(Reply::json(400, fixture("err_400_format.json")));
    let a = asr(settings("qwen/qwen3-asr-0.6b"));
    let e = run(&a, &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::ProviderError);
    assert_eq!(srv.count(), 2);
}

fn chat_asr() -> flow_cloud::CloudAsr {
    let mut i = info("google/gemini-3.5-flash-lite", SttPath::ChatAudio);
    i.supports.reasoning = true;
    i.reasoning_mandatory = true;
    i.reasoning_efforts = vec!["high".into(), "low".into(), "minimal".into()];
    asr(settings("google/gemini-3.5-flash-lite")).with_model_info(Some(i))
}

#[test]
fn path_b_chat_audio_request_shape() {
    let srv = mock_fixed(Reply::json(200, fixture("chat_audio_ok.json")));
    let a = chat_asr();
    assert_eq!(a.path(), SttPath::ChatAudio);
    let text = run(&a, &req(&["Kubernetes"], &["Alice"], Some("en"))).unwrap();
    assert_eq!(text, "And so, my fellow Americans.");
    let r = &srv.requests()[0];
    assert_eq!(r.path(), "/api/v1/chat/completions");
    let j = r.json();
    assert_eq!(j["model"], "google/gemini-3.5-flash-lite");
    assert_eq!(j["messages"][0]["role"], "system");
    assert_eq!(j["messages"][0]["content"], TRANSCRIBE_PROMPT);
    let parts = &j["messages"][1]["content"];
    assert_eq!(parts[0]["type"], "input_audio");
    assert_eq!(parts[0]["input_audio"]["format"], "wav");
    assert_eq!(&b64(&parts[0]["input_audio"]["data"])[..4], b"RIFF");
    assert_eq!(
        parts[1]["text"],
        "Language: en. Spell these names exactly: Kubernetes."
    );
    assert_eq!(j["temperature"], 0);
    assert_eq!(j["max_completion_tokens"], 40, "ceil(1 s × 8) + 32");
    assert_eq!(j["reasoning"]["effort"], "minimal", "lowest allowed");
    assert_eq!(j["stream"], false);
    assert_eq!(j["provider"]["zdr"], true);
    assert_eq!(j["provider"]["data_collection"], "deny");
    assert!(!r.raw_text().contains("Alice"));
    assert_no_key_in(r, "authorization");
    let info = a.last_call().unwrap();
    assert_eq!(info.cost_usd, Some(0.000041));
    assert_eq!(info.request_id.as_deref(), Some("gen-chat-audio-1"));
    assert_eq!(info.stt_path, "chat_audio");
}

#[test]
fn path_b_preamble_is_rejected_as_unverbatim() {
    let _srv = mock_fixed(Reply::json(200, fixture("chat_audio_preamble.json")));
    let a = chat_asr();
    let e = run(&a, &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::ProviderError);
    assert!(e.message.contains("verbatim"), "{e}");
}

#[test]
fn path_is_chosen_per_catalogue_entry() {
    let _srv = mock_fixed(Reply::json(200, fixture("transcriptions_ok.json")));
    let cat = flow_cloud::Catalog::baked();
    let chat = cat
        .stt_models()
        .find(|m| m.stt_path == Some(SttPath::ChatAudio))
        .expect("a chat-audio model is baked");
    assert_eq!(asr(settings(&chat.id)).path(), SttPath::ChatAudio);
    assert_eq!(whisper().path(), SttPath::Transcriptions);
    // Unknown free-text id: path A.
    assert_eq!(
        asr(settings("acme/brand-new-asr")).path(),
        SttPath::Transcriptions
    );
    // Config override wins.
    let mut s = settings(&chat.id);
    s.stt_path = SttPathChoice::Transcriptions;
    assert_eq!(asr(s).path(), SttPath::Transcriptions);
}

#[test]
fn error_table() {
    let cases: Vec<(Reply, &str, usize)> = vec![
        (Reply::json(401, fixture("err_401.json")), "auth_failed", 1),
        (
            Reply::json(402, fixture("err_402_credits.json")),
            "quota_exceeded",
            1,
        ),
        (
            Reply::json(402, fixture("err_402_inflight.json")).header("Retry-After", "0.1"),
            "busy",
            2,
        ),
        (
            Reply::json(402, fixture("err_402_inflight.json")),
            "busy",
            1,
        ),
        (
            Reply::json(403, fixture("err_403_key_limit.json")),
            "quota_exceeded",
            1,
        ),
        (
            Reply::json(403, fixture("err_403_moderation.json")),
            "provider_error",
            1,
        ),
        (Reply::json(403, serde_json::json!({})), "auth_failed", 1),
        (Reply::json(408, fixture("err_408.json")), "timeout", 1),
        (Reply::json(429, fixture("err_429.json")), "busy", 1),
        (
            Reply::json(429, fixture("err_429.json")).header("Retry-After", "30"),
            "busy",
            1,
        ),
        (
            Reply::json(502, fixture("err_502.json")),
            "provider_error",
            2,
        ),
        (Reply::json(500, serde_json::json!({})), "provider_error", 1),
        (
            Reply::json(503, fixture("err_503_routing.json")),
            "no_private_route",
            1,
        ),
        (Reply::json(503, serde_json::json!({})), "provider_error", 2),
        (Reply::json(400, serde_json::json!({})), "provider_error", 1),
        (Reply::raw(200, "<html>oops</html>"), "provider_error", 1),
    ];
    for (reply, want, tries) in cases {
        let status = reply.status;
        let srv = mock_fixed(reply);
        let a = whisper();
        let e = run(&a, &req(&[], &[], None)).unwrap_err();
        assert_eq!(last_code(&a), want, "HTTP {status}: {e}");
        assert_eq!(srv.count(), tries, "HTTP {status}: attempts");
        assert!(!e.message.contains(KEY) && !e.message.contains("oops"));
        assert!(
            !e.message.contains("openrouter.ai/credits") && !e.message.contains("harassment"),
            "no provider free text: {e}"
        );
        if want == "no_private_route" {
            assert!(
                e.message.contains("model has no zero-retention provider"),
                "{e}"
            );
        }
    }
}

#[test]
fn transport_errors() {
    check_transport_errors();
}

#[test]
fn redirects_are_never_followed() {
    for status in [302u16, 307, 308] {
        let target = flow_cloud::testing::MockServer::fixed(Reply::json(
            200,
            serde_json::json!({"text": "x"}),
        ));
        let loc = target.url("/steal");
        let srv = mock_fixed(Reply::raw(status, "").header("location", &loc));
        let e = run(&whisper(), &req(&["term"], &[], None)).unwrap_err();
        assert_eq!(e.kind, Failure::ProviderError, "{status}: {e}");
        assert_eq!(srv.count(), 1, "{status}: not retried");
        assert_eq!(
            target.count(),
            0,
            "{status}: redirect followed (key/audio leaked)"
        );
    }
}

#[test]
fn missing_cost_is_estimated_then_looked_up() {
    let srv = mock(|r, _| {
        if r.path().ends_with("/generation") {
            assert_eq!(r.query_all("id"), ["gen-late-1"]);
            return Reply::json(200, serde_json::json!({"data": {"total_cost": 0.00042}}));
        }
        Reply::json(200, serde_json::json!({"text": "hi"})).header("X-Generation-Id", "gen-late-1")
    });
    let got = Arc::new(Mutex::new(Vec::new()));
    let g2 = got.clone();
    let a = whisper()
        .with_cost_sink(Arc::new(move |c| g2.lock().unwrap().push(c)))
        .with_cost_lookup_delay(Duration::from_millis(10));
    run(&a, &req(&[], &[], None)).unwrap();
    let info = a.last_call().unwrap();
    assert!(info.cost_estimated);
    let est = info.cost_usd.expect("catalogue estimate");
    assert!(est > 0.0 && est < 0.001, "{est}");
    for _ in 0..200 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let c = got.lock().unwrap()[0].clone();
    assert_eq!(c.generation_id, "gen-late-1");
    assert_eq!(c.actual_usd, Some(0.00042));
    assert_eq!(c.estimated_usd, Some(est));
    let g = srv
        .requests()
        .into_iter()
        .find(|r| r.path().ends_with("/generation"))
        .unwrap();
    assert_no_key_in(&g, "authorization");
}
