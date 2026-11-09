//! D28 "W5" tests: the OpenRouter page's edits + the Developer form →
//! prefs.json (no key) → router JSON == `crates/flow-router/fixtures/
//! router_config_cloud.json`; the key only in the credential store
//! (in-memory stand-in on Linux); separate consents for audio and text;
//! the one-time migration of every old D18 preset
//! (`crates/flow-router/fixtures/migration/`), including Credential
//! Manager and the usage file.

use flow_router::{
    CleanupRoute, CloudConfig, CloudUsage, MigrationReport, ProviderUsage, RouterConfig, Usd,
};
use flow_windows::credentials::{self, CredentialStore, MemoryStore, OPENROUTER};
use flow_windows::dev_settings::{BuildCaps, DevForm, DevSettings, Route};
use flow_windows::openrouter::{self, CleanupForm, SttForm};
use flow_windows::prefs::Prefs;
use std::path::{Path, PathBuf};

const ALL: BuildCaps = BuildCaps {
    local: true,
    remote: true,
    cloud: true,
};
const KEY: &str = "sk-or-v1-LEAKCHECK-0123456789abcdef";

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/flow-router/fixtures")
        .join(name)
}

fn fixture(name: &str) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(fixture_path(name)).unwrap()).unwrap()
}

fn fixture_cloud() -> CloudConfig {
    serde_json::from_value(fixture("router_config_cloud.json")["cloud"].clone()).unwrap()
}

fn tmp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flow-win-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// What the Developer tab sends on Save (`dev_set`): no OpenRouter fields.
fn ui_form() -> DevForm {
    serde_json::from_value(serde_json::json!({
        "allow_server": true,
        "allow_client": true,
        "route_order": ["server", "client", "cloud"],
        "host": "100.64.0.10",
        "port": 7313,
        "tls": false,
        "tls_pin_sha256": "",
        "token": "dev-token-0123456789abcdef",
        "connect_timeout_ms": 2500,
        "commit_timeout_base_ms": 3000,
        "health_ttl_s": 30,
        "keep_local_warm": true,
        "show_engine_badge": false,
        "dev_unlocked": false,
    }))
    .unwrap()
}

/// The fixture's settings, made the way the user makes them: Developer
/// Save, then the OpenRouter page (STT on, cleanup on, budget).
fn configured() -> (DevSettings, String) {
    let (mut dev, token) = ui_form().validate(ALL).unwrap();
    openrouter::apply_stt(
        &mut dev,
        &SttForm {
            allow: true,
            consent: true,
            send_dictionary: true,
            stt_path: "auto".into(),
            ..Default::default()
        },
        1_792_540_800_000,
    )
    .unwrap();
    openrouter::apply_cleanup(
        &mut dev,
        &CleanupForm {
            allow: true,
            consent: true,
            order: Some(vec![CleanupRoute::Cloud, CleanupRoute::Local]),
            sort: "latency".into(),
            timeout_ms: 2500,
            ..Default::default()
        },
        1_792_540_900_000,
    )
    .unwrap();
    openrouter::apply_budget(&mut dev, Some(5.0)).unwrap();
    (dev, token)
}

#[test]
fn page_edits_map_to_the_router_cloud_fixture() {
    let (dev, token) = configured();
    assert_eq!(
        dev.route_order,
        vec![Route::Cloud, Route::Server, Route::Client],
        "turning STT on puts it first"
    );
    let mut j = dev.router_config_json(ALL, &token);
    // D36: the shell sends the whole cleanup order (the fixture's
    // `[cloud, local]` plus the server right after this PC), all on.
    let mut want = fixture("router_config_cloud.json");
    assert_eq!(want["cleanup_order"], serde_json::json!(["cloud", "local"]));
    assert_eq!(
        j["cleanup_order"],
        serde_json::json!(["cloud", "local", "server"])
    );
    assert!(j.get("cleanup_off").is_none());
    want.as_object_mut().unwrap().remove("cleanup_order");
    j.as_object_mut().unwrap().remove("cleanup_order");
    assert_eq!(j, want);
    let cfg: RouterConfig = serde_json::from_value(dev.router_config_json(ALL, &token)).unwrap();
    cfg.validate().unwrap();
    assert!(cfg.cloud.openrouter.stt.consent_ok());
    assert!(cfg.cloud.openrouter.cleanup.consent_ok());
    assert!(cfg.cloud_cleanup_wanted());
}

#[test]
fn a_developer_save_after_the_page_keeps_openrouter() {
    let (saved, _) = configured();
    let mut f = DevForm::for_page(&saved);
    f.host = "100.64.0.11".into();
    let (mut dev, _) = f.with_token(&saved).validate(ALL).unwrap();
    dev.keep_cloud(&saved);
    assert_eq!(dev.cloud, saved.cloud);
    assert!(dev.allow_cloud && dev.cleanup_on(CleanupRoute::Cloud));
    assert_eq!(dev.cleanup(), saved.cleanup());
    assert_eq!(dev.server.host, "100.64.0.11");
}

#[test]
fn prefs_round_trip_without_keys() {
    let store = MemoryStore::new();
    let (mut dev, _) = configured();
    dev.token_dpapi = flow_windows::secrets::protect("dev-token-0123456789abcdef").unwrap();
    credentials::store_openrouter_key(&*store, KEY).unwrap();
    let prefs = Prefs {
        developer: dev.clone(),
        ..Prefs::default()
    };
    let dir = tmp("cloud-prefs");
    let path = dir.join("prefs.json");
    prefs.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("LEAKCHECK"), "C3: no key in prefs.json");
    assert!(!text.contains("\"key\""), "{text}");
    let (back, _) = Prefs::load(&path);
    assert_eq!(back.developer, dev);
    assert_eq!(back.developer.cloud, fixture_cloud());
    // The prefs' cloud section is exactly the router's CloudConfig.
    let on_disk: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        on_disk["developer"]["cloud"],
        fixture("router_config_cloud.json")["cloud"]
    );
    assert_eq!(on_disk["developer"]["allow_cloud"], true);
    assert_eq!(
        on_disk["developer"]["cleanup_engines"],
        serde_json::json!([
            {"route": "cloud", "enabled": true},
            {"route": "local", "enabled": true},
            {"route": "server", "enabled": true}
        ])
    );
    for k in ["allow_cloud_cleanup", "cleanup_order"] {
        assert!(
            on_disk["developer"].get(k).is_none(),
            "pre-D36 {k} is not written"
        );
    }
    // ...while the key is in the store, and only masked for the page.
    assert_eq!(credentials::openrouter_key(&*store).as_deref(), Some(KEY));
    let ks = credentials::key_state(&*store, OPENROUTER);
    assert!(ks.set && ks.masked == "••••cdef", "{ks:?}");
    assert_eq!(
        store.targets(),
        vec!["flow-clone/cloud/openrouter".to_string()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_prefs_without_cloud_load_with_cloud_off() {
    let old: DevSettings =
        serde_json::from_str(r#"{"allow_server":true,"route_order":["client","server"]}"#).unwrap();
    assert!(!old.allow_cloud && !old.cleanup_on(CleanupRoute::Cloud) && old.cloud.is_default());
    assert!(!old.cloud.is_legacy(), "nothing to migrate");
    assert_eq!(
        old.order(),
        vec![Route::Client, Route::Server, Route::Cloud]
    );
    let j = old.router_config_json(ALL, "");
    assert!(j.get("order").is_none() && j.get("cloud").is_none());
}

#[test]
fn audio_consent_and_text_consent_are_separate_records() {
    let (mut dev, _) = ui_form().validate(ALL).unwrap();
    openrouter::apply_cleanup(
        &mut dev,
        &CleanupForm {
            allow: true,
            consent: true,
            sort: "latency".into(),
            timeout_ms: 2500,
            ..Default::default()
        },
        7,
    )
    .unwrap();
    let cfg: RouterConfig = serde_json::from_value(dev.router_config_json(ALL, "")).unwrap();
    assert!(cfg.cloud_cleanup_wanted());
    assert!(!cfg.allow_cloud, "cleanup never turns STT on");
    assert!(!cfg.cloud.openrouter.stt.consent_ok());
    let refused = openrouter::apply_stt(
        &mut dev,
        &SttForm {
            allow: true,
            ..Default::default()
        },
        8,
    );
    assert!(refused.is_err(), "text consent does not cover audio");
}

// ------------------------------------------------------- D28 migration

/// A D18 prefs `developer` section equivalent to the router fixture `v1`.
fn legacy_dev(v1: &serde_json::Value) -> DevSettings {
    let order: Vec<Route> = serde_json::from_value(
        v1.get("order")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .unwrap();
    serde_json::from_value(serde_json::json!({
        "allow_cloud": v1.get("allow_cloud").cloned().unwrap_or(false.into()),
        "route_order": order,
        "cloud": v1["cloud"],
    }))
    .unwrap()
}

/// Every provider's key, bound to its host + adapter as D18 stored it.
fn legacy_store(v1: &serde_json::Value) -> std::sync::Arc<MemoryStore> {
    let s = MemoryStore::new();
    for p in v1["cloud"]["providers"].as_array().unwrap() {
        let id = p["id"].as_str().unwrap();
        let p: flow_router::CloudProviderConfig = serde_json::from_value(p.clone()).unwrap();
        s.set(
            id,
            &credentials::encode(
                &format!("sk-LEAKCHECK-{id}-0000"),
                &(p.host(), p.adapter.clone()),
            ),
        )
        .unwrap();
    }
    s
}

fn legacy_usage(v1: &serde_json::Value) -> CloudUsage {
    CloudUsage {
        month: flow_router::month_of(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        ),
        providers: v1["cloud"]["providers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| ProviderUsage {
                provider_id: p["id"].as_str().unwrap().into(),
                requests: 2,
                billed_audio_ms: 60_000,
                cost_usd: Usd(0.25),
                ..Default::default()
            })
            .collect(),
    }
}

#[test]
fn every_old_preset_migrates_with_its_keys_and_usage() {
    let dir = fixture_path("migration");
    let mut cases: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().into_string().unwrap();
            n.strip_suffix(".v1.json").map(str::to_string)
        })
        .collect();
    cases.sort();
    assert!(cases.len() >= 8, "{cases:?}");
    for case in cases {
        let v1 = fixture(&format!("migration/{case}.v1.json"));
        let v2 = fixture(&format!("migration/{case}.v2.json"));
        let want: MigrationReport =
            serde_json::from_value(fixture(&format!("migration/{case}.report.json"))).unwrap();
        let mut dev = legacy_dev(&v1);
        assert!(dev.cloud.is_legacy(), "{case}");
        let store = legacy_store(&v1);
        let usage = legacy_usage(&v1);
        let tdir = tmp(&format!("mig-{case}"));
        let upath = tdir.join("cloud_usage.json");
        flow_windows::cloud::save_usage(&upath, &usage).unwrap();

        let m = flow_windows::cloud::migrate_d28(&mut dev, &*store, &upath).expect(&case);
        assert_eq!(m.report, want, "{case}: same report as the router fixture");
        // Settings: the router's v2 cloud section and STT switch.
        assert_eq!(
            serde_json::to_value(&dev.cloud).unwrap(),
            v2["cloud"],
            "{case}"
        );
        assert_eq!(
            dev.allow_cloud,
            v2.get("allow_cloud")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
            "{case}"
        );
        if want.cloud_disabled {
            assert_eq!(
                dev.order().last(),
                Some(&Route::Cloud),
                "{case}: cloud last"
            );
        }
        assert_eq!(
            dev.cloud_notice, want.notice,
            "{case}: notice kept until read"
        );
        assert!(
            !dev.cleanup_on(CleanupRoute::Cloud),
            "{case}: cleanup is never switched on"
        );
        // Keys: only the OpenRouter one survives, under its new id.
        let left = store.targets();
        if want.key_from.is_some() {
            assert_eq!(
                left,
                vec!["flow-clone/cloud/openrouter".to_string()],
                "{case}"
            );
            let k = credentials::openrouter_key(&*store).unwrap();
            assert_eq!(
                k,
                format!("sk-LEAKCHECK-{}-0000", want.key_from.as_deref().unwrap())
            );
        } else {
            assert!(left.is_empty(), "{case}: {left:?}");
        }
        for id in &want.delete_keys {
            assert_eq!(store.get(id).unwrap(), None, "{case}: {id}");
        }
        // Usage: same month total (the budget stays honest).
        let after = flow_windows::cloud::load_usage(&upath).unwrap();
        assert!(
            (after.total_cost() - usage.total_cost()).abs() < 1e-9,
            "{case}: {after:?}"
        );
        assert!(after
            .providers
            .iter()
            .all(|p| p.provider_id == OPENROUTER || p.provider_id == "removed"));
        // prefs.json after the migration: schema 2, no key, no legacy list.
        let text = serde_json::to_string(&dev).unwrap();
        assert!(
            !text.contains("LEAKCHECK") && !text.contains("\"providers\""),
            "{case}"
        );
        // Once only.
        assert!(
            flow_windows::cloud::migrate_d28(&mut dev, &*store, &upath).is_none(),
            "{case}: idempotent"
        );
        // The router accepts the result as is (no second migration).
        let cfg: RouterConfig = serde_json::from_value(dev.router_config_json(ALL, "")).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.cloud.is_legacy(), "{case}");
        let _ = std::fs::remove_dir_all(&tdir);
    }
}
