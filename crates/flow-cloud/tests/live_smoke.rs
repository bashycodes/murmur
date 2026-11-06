//! Live OpenRouter smoke tests (D28 §5.3). Run only when
//! `FLOW_CLOUD_LIVE_OPENROUTER_KEY` is set; otherwise each prints a skip
//! line and passes. Optional `FLOW_CLOUD_LIVE_OPENROUTER_STT_MODEL` /
//! `FLOW_CLOUD_LIVE_OPENROUTER_CLEANUP_MODEL` (default: the recommended
//! STT models + one chat-audio model, and the default cleanup model).
//! Audio: `jfk.wav` from `FLOW_MODELS_DIR` (or `models/`). Never part of
//! default CI: it sends audio/text to OpenRouter and costs money.
//!
//! `cargo test -p flow-cloud --test live_smoke -- --nocapture --test-threads 1`

use std::time::{Duration, Instant};

use flow_cloud::catalog::{Catalog, RECOMMENDED_STT};
use flow_cloud::{CleanupSettings, CloudBackend, OpenRouterCleaner, SttPath, SttSettings};
use flow_core::asr::AbortFlag;
use flow_core::cleanup::{build_prompt, Cleaner};
use flow_core::config::{CleanupLevel, FieldContext, SessionConfig, Style};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_router::{Backend, SecretString};

const ENV_KEY: &str = "FLOW_CLOUD_LIVE_OPENROUTER_KEY";

fn key() -> Option<SecretString> {
    match std::env::var(ENV_KEY) {
        Ok(k) if !k.trim().is_empty() => Some(SecretString::new(k)),
        _ => {
            eprintln!("SKIP live openrouter: {ENV_KEY} not set");
            None
        }
    }
}

const REFERENCE: &str = "and so my fellow americans ask not what your country can do for you \
                         ask what you can do for your country";

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Word error rate (Levenshtein over words / reference length).
fn wer(reference: &str, hyp: &str) -> f64 {
    let (r, h) = (words(reference), words(hyp));
    let mut d: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut prev = d[0];
        d[0] = i;
        for j in 1..=h.len() {
            let cur = d[j];
            d[j] = if r[i - 1] == h[j - 1] {
                prev
            } else {
                1 + prev.min(d[j]).min(d[j - 1])
            };
            prev = cur;
        }
    }
    d[h.len()] as f64 / r.len().max(1) as f64
}

fn jfk() -> Option<Vec<i16>> {
    let dirs = [
        std::env::var("FLOW_MODELS_DIR").unwrap_or_default(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../models").to_string(),
    ];
    let p = dirs
        .iter()
        .filter(|d| !d.is_empty())
        .map(|d| std::path::Path::new(d).join("jfk.wav"))
        .find(|p| p.is_file())?;
    let mut r = hound::WavReader::open(p).ok()?;
    Some(r.samples::<i16>().map(|s| s.unwrap()).collect())
}

fn stt_once(key: &SecretString, pcm: &[i16], model: &str) {
    let s = SttSettings {
        model: model.into(),
        ..Default::default()
    };
    let b = CloudBackend::new(s, key.clone(), None);
    let cfg = SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::word("Meadowcroft")],
        },
        ..Default::default()
    };
    b.start_session(&cfg).unwrap();
    for c in pcm.chunks(1600) {
        b.push_audio(c);
    }
    let t = Instant::now();
    let r = b
        .commit(Instant::now() + Duration::from_secs(30))
        .unwrap_or_else(|e| panic!("{model}: {e}"));
    let ms = t.elapsed().as_millis();
    let w = wer(REFERENCE, &r.asr_text);
    let info = b.last_call().unwrap();
    eprintln!(
        "LIVE stt model={model} path={} commit_ms={ms} upload+provider_ms={} bytes={} billed_ms={} cost_usd={:?} estimated={} gen={:?} wer={w:.3} text={:?}",
        info.stt_path, info.latency_ms, info.bytes, info.billed_audio_ms, info.cost_usd, info.cost_estimated, info.request_id, r.asr_text
    );
    assert!(w <= 0.10, "{model}: WER {w:.3} > 10 %: {:?}", r.asr_text);
}

#[test]
fn wer_helper() {
    assert_eq!(wer(REFERENCE, REFERENCE), 0.0);
    assert!((wer("a b c d", "a x c") - 0.5).abs() < 1e-9);
}

#[test]
fn live_openrouter_stt() {
    let Some(key) = key() else { return };
    let Some(pcm) = jfk() else {
        eprintln!("SKIP live openrouter stt: jfk.wav not found (FLOW_MODELS_DIR)");
        return;
    };
    let models: Vec<String> = match std::env::var("FLOW_CLOUD_LIVE_OPENROUTER_STT_MODEL") {
        Ok(m) => vec![m],
        Err(_) => {
            let mut v: Vec<String> = RECOMMENDED_STT.iter().map(|s| s.to_string()).collect();
            if let Some(m) = Catalog::baked()
                .search("", flow_cloud::catalog::Use::Stt, Default::default())
                .into_iter()
                .find(|m| m.stt_path == Some(SttPath::ChatAudio))
            {
                v.push(m.id.clone());
            }
            v
        }
    };
    for m in models {
        stt_once(&key, &pcm, &m);
    }
}

const MESSY: [&str; 8] = [
    "um so i think we should uh meet on tuesday",
    "send the report to john no wait to jane by friday",
    "can you like pick up eggs milk bread and butter",
    "i mean the the meeting went well overall",
    "okay so the deadline is next week right",
    "hey just checking in on the invoice you know the one from march",
    "lets grab lunch at noon actually make it one",
    "the server is down again sorry i mean the staging server",
];

#[test]
fn live_openrouter_cleanup() {
    let Some(key) = key() else { return };
    let mut s = CleanupSettings::default();
    if let Ok(m) = std::env::var("FLOW_CLOUD_LIVE_OPENROUTER_CLEANUP_MODEL") {
        s.model = m;
    }
    let timeout = Duration::from_millis(s.timeout_ms);
    let c = std::sync::Arc::new(OpenRouterCleaner::new(s.clone(), key));
    let mut ok = 0;
    let mut lat = Vec::new();
    for t in MESSY {
        let p = build_prompt(
            t,
            CleanupLevel::Light,
            Style::default(),
            &FieldContext::default(),
            &Dictionary::default(),
        );
        let t0 = Instant::now();
        let out = flow_core::cleanup::run_cleanup(c.clone(), p, t, timeout);
        lat.push(t0.elapsed().as_millis() as u64);
        let call = c.last_call().unwrap_or_default();
        eprintln!(
            "LIVE cleanup model={} ms={} ttft={:?} cost={:?} gen={:?} err={:?} outcome={out:?}",
            s.model,
            t0.elapsed().as_millis(),
            call.ttft_ms,
            call.cost_usd,
            call.generation_id,
            call.error
        );
        if matches!(out, flow_core::cleanup::CleanOutcome::Formatted(_)) {
            ok += 1;
        }
    }
    lat.sort();
    let p90 = lat[(lat.len() * 9).div_ceil(10) - 1];
    eprintln!("LIVE cleanup formatted={ok}/8 p90_ms={p90}");
    assert!(ok >= 6, "formatted only {ok}/8");
    assert!(p90 < s.timeout_ms, "p90 {p90} ms ≥ timeout");
    let _ = c.clean_cancellable(
        &flow_core::cleanup::warm_prompt(),
        Instant::now() + timeout,
        &AbortFlag::default(),
    );
}

#[test]
fn live_openrouter_key_info() {
    let Some(key) = key() else { return };
    let k = flow_cloud::keyinfo::fetch(&key).expect("/key parses");
    eprintln!(
        "LIVE key limit={:?} remaining={:?} usage_monthly={:?} free_tier={}",
        k.limit, k.limit_remaining, k.usage_monthly, k.is_free_tier
    );
}

#[test]
fn live_openrouter_catalog() {
    let Some(key) = key() else { return };
    let t = Instant::now();
    let c = flow_cloud::catalog::refresh(&flow_cloud::Api::new(), Some(&key)).expect("catalogue");
    eprintln!(
        "LIVE catalog ms={} models={} stt={} cleanup={}",
        t.elapsed().as_millis(),
        c.models.len(),
        c.stt_models().count(),
        c.cleanup_models().count()
    );
    assert!(c.get(flow_cloud::DEFAULT_STT_MODEL).is_some());
}
