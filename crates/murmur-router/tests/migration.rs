//! D28 one-time migration (spec §2.1): schema-1 configs of every old
//! preset → schema 2. Fixtures in `fixtures/migration/` are shared with
//! the shells (input `<case>.v1.json`, output `<case>.v2.json`, report
//! `<case>.report.json`). Regenerate:
//! `FLOW_REGEN_FIXTURES=1 cargo test -p flow-router --test migration`.

use std::path::PathBuf;

use flow_router::cloud::{DEFAULT_STT_MODEL, OPENROUTER_HOST};
use flow_router::*;
use serde_json::{json, Value};

/// The D18 preset table (flow-cloud `presets()` before D28), frozen here
/// because the migration must keep reading it after the presets are gone.
fn legacy_provider(preset: &str) -> Value {
    let (adapter, label, base, model, zdr) = match preset {
        "openrouter" => (
            "openrouter",
            "OpenRouter",
            "https://openrouter.ai/api/v1",
            "openai/gpt-transcribe",
            true,
        ),
        "elevenlabs" => (
            "elevenlabs",
            "ElevenLabs Scribe",
            "https://api.elevenlabs.io",
            "scribe_v2",
            false,
        ),
        "groq" => (
            "openai_compat",
            "Groq",
            "https://api.groq.com/openai/v1",
            "whisper-large-v3-turbo",
            false,
        ),
        "openai" => (
            "openai_compat",
            "OpenAI",
            "https://api.openai.com/v1",
            "gpt-transcribe",
            false,
        ),
        "deepgram" => (
            "deepgram",
            "Deepgram",
            "https://api.deepgram.com",
            "nova-3",
            true,
        ),
        "assemblyai" => (
            "assemblyai",
            "AssemblyAI",
            "https://api.assemblyai.com",
            "universal",
            true,
        ),
        "custom" => (
            "openai_compat",
            "Speaches",
            "http://100.64.0.20:8000/v1",
            "Systran/faster-whisper-small",
            false,
        ),
        p => panic!("{p}"),
    };
    let host = base
        .split("://")
        .nth(1)
        .unwrap()
        .split(['/', ':'])
        .next()
        .unwrap();
    json!({
        "id": format!("{preset}-1"), "preset": preset, "adapter": adapter, "label": label,
        "base_url": base, "model": model, "zero_retention": zdr, "send_dictionary": true,
        "consent_host": host, "consent_at_ms": 1_792_540_800_000u64,
    })
}

const PRESETS: [&str; 7] = [
    "openrouter",
    "elevenlabs",
    "groq",
    "openai",
    "deepgram",
    "assemblyai",
    "custom",
];

fn legacy_config(providers: Vec<Value>, active: &str) -> Value {
    json!({
        "allow_cloud": true,
        "order": ["cloud", "server", "client"],
        "server": {"host": "100.64.0.10", "token": "dev-token-0123456789abcdef"},
        "cloud": {"active": active, "providers": providers, "monthly_budget_usd": 5.0}
    })
}

fn parse(v: &Value) -> RouterConfig {
    serde_json::from_value(v.clone()).unwrap()
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/migration")
}

fn cases() -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = PRESETS
        .iter()
        .map(|p| {
            (
                p.to_string(),
                legacy_config(vec![legacy_provider(p)], &format!("{p}-1")),
            )
        })
        .collect();
    // OpenRouter kept, Groq active: cloud turned off, Groq key deleted.
    v.push((
        "mixed_groq_active".into(),
        legacy_config(
            vec![legacy_provider("openrouter"), legacy_provider("groq")],
            "groq-1",
        ),
    ));
    v
}

#[test]
fn fixtures_match_for_every_old_preset() {
    std::fs::create_dir_all(dir()).unwrap();
    let regen = std::env::var_os("FLOW_REGEN_FIXTURES").is_some();
    for (name, input) in cases() {
        let mut c = parse(&input);
        let rep = c.migrate_d28();
        let files = [
            (format!("{name}.v1.json"), input.clone()),
            (format!("{name}.v2.json"), serde_json::to_value(&c).unwrap()),
            (
                format!("{name}.report.json"),
                serde_json::to_value(&rep).unwrap(),
            ),
        ];
        for (f, v) in files {
            let p = dir().join(&f);
            if regen {
                std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap();
            }
            let want: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            assert_eq!(v, want, "{f}");
        }
        assert_eq!(c.validate(), Ok(()), "{name}");
    }
}

#[test]
fn openrouter_entry_keeps_key_consent_and_settings() {
    let mut p = legacy_provider("openrouter");
    p["model"] = json!("openai/whisper-large-v3");
    p["language"] = json!("de");
    p["send_dictionary"] = json!(false);
    p["format_override"] = json!("wav");
    p["timeout_base_ms"] = json!(9000);
    let mut c = parse(&legacy_config(vec![p], "openrouter-1"));
    let rep = c.migrate_d28();
    assert!(rep.migrated);
    assert_eq!(rep.key_from.as_deref(), Some("openrouter-1"));
    assert_eq!(rep.key_to.as_deref(), Some("openrouter"));
    assert!(rep.delete_keys.is_empty() && rep.removed.is_empty());
    assert!(!rep.cloud_disabled);
    assert_eq!(rep.notice, None, "nothing user-visible changed");
    let s = &c.cloud.openrouter.stt;
    assert_eq!(s.model, "openai/whisper-large-v3");
    assert_eq!(s.language, "de");
    assert!(!s.send_dictionary);
    assert_eq!(s.format_override, "wav");
    assert_eq!(s.timeout_base_ms, 9000);
    assert!(s.consent_ok() && s.consent_host == OPENROUTER_HOST);
    assert!(
        !c.cloud.openrouter.cleanup.consent_ok(),
        "STT consent is not cleanup consent"
    );
    assert!(c.allow_cloud && c.order.contains(&Route::Cloud));
    assert!(!c.allow_cloud_cleanup && c.cleanup_order.is_empty());
    assert_eq!(c.cloud.monthly_budget_usd, Some(Usd(5.0)));
    assert_eq!(c.cloud.schema, 2);
    assert!(c.cloud.providers.is_empty() && c.cloud.active.is_empty());
}

#[test]
fn every_other_preset_is_deleted_with_its_key_and_cloud_turned_off() {
    for p in PRESETS.iter().filter(|p| **p != "openrouter") {
        let mut c = parse(&legacy_config(vec![legacy_provider(p)], &format!("{p}-1")));
        let rep = c.migrate_d28();
        assert_eq!(rep.delete_keys, [format!("{p}-1")], "{p}");
        assert_eq!(rep.removed.len(), 1);
        assert!(rep.key_from.is_none());
        assert!(rep.cloud_disabled, "{p}: not moved to OpenRouter silently");
        assert!(!c.allow_cloud && !c.order.contains(&Route::Cloud));
        assert_eq!(c.order, [Route::Server, Route::Client]);
        let n = rep.notice.expect("notice");
        assert!(
            n.starts_with("Cloud speech-to-text now uses OpenRouter only."),
            "{n}"
        );
        assert!(n.contains("removed and its key deleted"), "{n}");
        assert!(n.contains("Set up OpenRouter"), "{n}");
        // The default OpenRouter entry carries no consent.
        assert!(!c.cloud.openrouter.stt.consent_ok());
        assert_eq!(c.cloud.openrouter.stt.model, DEFAULT_STT_MODEL);
    }
}

#[test]
fn mixed_config_keeps_openrouter_but_disables_the_route() {
    let mut c = parse(&legacy_config(
        vec![legacy_provider("openrouter"), legacy_provider("groq")],
        "groq-1",
    ));
    let rep = c.migrate_d28();
    assert_eq!(rep.key_from.as_deref(), Some("openrouter-1"));
    assert_eq!(rep.delete_keys, ["groq-1"]);
    assert!(rep.cloud_disabled);
    assert!(
        c.cloud.openrouter.stt.consent_ok(),
        "OpenRouter consent kept"
    );
    assert!(!c.allow_cloud);
    assert!(rep.notice.unwrap().contains("Groq was removed"));
}

#[test]
fn a_model_without_zdr_endpoint_switches_only_while_zdr_is_on() {
    let mut c = parse(&legacy_config(
        vec![legacy_provider("openrouter")],
        "openrouter-1",
    ));
    let rep = c.migrate_d28();
    assert_eq!(rep.model_from.as_deref(), Some("openai/gpt-transcribe"));
    assert_eq!(rep.model_to.as_deref(), Some(DEFAULT_STT_MODEL));
    assert_eq!(c.cloud.openrouter.stt.model, DEFAULT_STT_MODEL);
    assert!(rep.notice.unwrap().contains("no zero-retention provider"));
    // ZDR off: the user's model stays.
    let mut p = legacy_provider("openrouter");
    p["zero_retention"] = json!(false);
    let mut c = parse(&legacy_config(vec![p], "openrouter-1"));
    let rep = c.migrate_d28();
    assert!(rep.model_from.is_none());
    assert_eq!(c.cloud.openrouter.stt.model, "openai/gpt-transcribe");
    assert!(!c.cloud.openrouter.privacy.zdr);
}

#[test]
fn consent_for_another_host_is_not_carried_over() {
    let mut p = legacy_provider("openrouter");
    p["base_url"] = json!("https://or-proxy.example.com/api/v1");
    p["consent_host"] = json!("or-proxy.example.com");
    let mut c = parse(&legacy_config(vec![p], "openrouter-1"));
    c.migrate_d28();
    assert!(!c.cloud.openrouter.stt.consent_ok());
}

#[test]
fn migration_is_idempotent_and_schema2_is_untouched() {
    let mut c = parse(&legacy_config(
        vec![legacy_provider("openrouter"), legacy_provider("deepgram")],
        "openrouter-1",
    ));
    assert!(c.migrate_d28().migrated);
    let once = c.clone();
    let rep = c.migrate_d28();
    assert_eq!(rep, MigrationReport::default());
    assert_eq!(c, once);
    // Round trip of the migrated JSON stays schema 2.
    let back = parse(&serde_json::to_value(&c).unwrap());
    assert_eq!(back, once);
    let mut d = RouterConfig::default();
    assert!(!d.migrate_d28().migrated);
}

#[test]
fn usage_keeps_the_month_total_for_the_budget() {
    let mut c = parse(&legacy_config(
        vec![
            legacy_provider("openrouter"),
            legacy_provider("groq"),
            legacy_provider("deepgram"),
        ],
        "openrouter-1",
    ));
    let rep = c.migrate_d28();
    let mut u: CloudUsage = serde_json::from_value(json!({
        "month": "2026-10",
        "providers": [
            {"provider_id": "openrouter-1", "requests": 3, "billed_audio_ms": 3000, "cost_usd": 0.5},
            {"provider_id": "groq-1", "requests": 2, "billed_audio_ms": 2000, "cost_usd": 0.25},
            {"provider_id": "deepgram-1", "requests": 1, "billed_audio_ms": 1000, "cost_usd": 0.125, "cost_estimated": true}
        ]
    }))
    .unwrap();
    let before = u.total_cost();
    u.migrate_d28(&rep);
    assert_eq!(u.total_cost(), before);
    assert_eq!(u.providers.len(), 2);
    assert_eq!(u.providers[0].provider_id, "openrouter");
    assert_eq!(u.providers[1].provider_id, "removed");
    assert_eq!(u.providers[1].requests, 3);
    assert!(u.providers[1].cost_estimated);
}

#[test]
fn the_router_migrates_in_memory_and_never_routes_to_a_removed_provider() {
    let r = Router::new(
        parse(&legacy_config(
            vec![legacy_provider("elevenlabs")],
            "elevenlabs-1",
        )),
        None,
        None,
    );
    let c = r.config();
    assert_eq!(c.cloud.schema, 2);
    assert!(!c.allow_cloud);
    assert!(r
        .cloud_notice()
        .unwrap()
        .contains("ElevenLabs Scribe was removed"));
    r.ack_cloud_notice();
    assert!(r.cloud_notice().is_none());
    // A key for the removed provider never enters the router.
    r.set_cloud_key("elevenlabs-1", Some(SecretString::new("sk-el-0000")));
    assert!(!r.has_cloud_key("elevenlabs-1") && !r.has_cloud_key("openrouter"));
    // update_config with legacy JSON migrates too.
    r.update_config(parse(&legacy_config(
        vec![legacy_provider("deepgram")],
        "deepgram-1",
    )));
    assert_eq!(r.config().cloud.schema, 2);
    assert!(r.cloud_notice().unwrap().contains("Deepgram"));
}
