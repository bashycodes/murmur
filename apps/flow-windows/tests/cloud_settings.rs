//! D18 §10 "W" row: Developer cloud settings → prefs.json (no keys) →
//! router JSON == `crates/flow-router/fixtures/router_config_cloud.json`;
//! keys only in the credential store (in-memory stand-in on Linux);
//! consent cleared when the host or adapter changes.

use flow_router::{CloudConfig, RouterConfig};
use flow_windows::credentials::{self, CredentialStore, MemoryStore};
use flow_windows::dev_settings::{reconcile_consent, BuildCaps, DevForm, DevSettings, Route};
use flow_windows::prefs::Prefs;
use std::path::Path;

const ALL: BuildCaps = BuildCaps {
    local: true,
    remote: true,
    cloud: true,
};
const KEY: &str = "sk-or-v1-LEAKCHECK-0123456789abcdef";

fn fixture(name: &str) -> serde_json::Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/flow-router/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn fixture_cloud() -> CloudConfig {
    serde_json::from_value(fixture("router_config_cloud.json")["cloud"].clone()).unwrap()
}

/// What the Developer tab sends on Save (`dev_set`), as JSON.
fn ui_form() -> DevForm {
    serde_json::from_value(serde_json::json!({
        "allow_server": true,
        "allow_client": true,
        "route_order": ["cloud", "server", "client"],
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
        "allow_cloud": true,
        "cloud": fixture("router_config_cloud.json")["cloud"].clone(),
    }))
    .unwrap()
}

#[test]
fn ui_form_maps_to_the_router_cloud_fixture() {
    let (dev, token) = ui_form().validate(ALL).unwrap();
    assert_eq!(
        dev.route_order,
        vec![Route::Cloud, Route::Server, Route::Client]
    );
    let j = dev.router_config_json(ALL, &token);
    assert_eq!(j, fixture("router_config_cloud.json"));
    let cfg: RouterConfig = serde_json::from_value(j).unwrap();
    cfg.validate().unwrap();
    assert!(cfg.cloud.active_provider().unwrap().consent_ok());
}

#[test]
fn prefs_round_trip_without_keys() {
    let store = MemoryStore::new();
    let (mut dev, _) = ui_form().validate(ALL).unwrap();
    dev.token_dpapi = flow_windows::secrets::protect("dev-token-0123456789abcdef").unwrap();
    store.set("openrouter-1", KEY).unwrap();
    let prefs = Prefs {
        developer: dev.clone(),
        ..Prefs::default()
    };
    let dir = std::env::temp_dir().join(format!("flow-win-cloud-prefs-{}", std::process::id()));
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
    // ...while the key is in the store, and only masked for the page.
    assert_eq!(store.get("openrouter-1").unwrap().as_deref(), Some(KEY));
    let ks = credentials::key_state(&*store, "openrouter-1");
    assert!(ks.set && ks.masked == "••••cdef", "{ks:?}");
    assert_eq!(
        store.targets(),
        vec!["flow-clone/cloud/openrouter-1".to_string()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_prefs_without_cloud_load_with_cloud_off() {
    let old: DevSettings =
        serde_json::from_str(r#"{"allow_server":true,"route_order":["client","server"]}"#).unwrap();
    assert!(!old.allow_cloud && old.cloud.is_default());
    assert_eq!(
        old.order(),
        vec![Route::Client, Route::Server, Route::Cloud]
    );
    assert!(old.router_config_json(ALL, "").get("order").is_none());
}

#[test]
fn consent_cleared_on_host_or_adapter_change_via_the_form() {
    let (saved, _) = ui_form().validate(ALL).unwrap();
    // Host change of the active provider (OpenRouter → a lookalike).
    let mut f = ui_form();
    f.cloud.providers[0].base_url = "https://openrouter.example.net/api/v1".into();
    let (mut new, _) = f.validate(ALL).unwrap();
    reconcile_consent(&saved.cloud, &mut new.cloud);
    let p = &new.cloud.providers[0];
    assert!(!p.consent_ok() && p.consent_at_ms == 0 && p.consent_host.is_empty());
    // The router then skips the cloud route (C1) even with allow_cloud.
    let cfg: RouterConfig =
        serde_json::from_value(new.router_config_json(ALL, "dev-token-0123456789abcdef")).unwrap();
    assert!(!cfg.cloud.active_provider().unwrap().consent_ok());
    // Adapter change on the same host.
    let mut f = ui_form();
    f.cloud.providers[0].adapter = "openai_compat".into();
    let (mut new, _) = f.validate(ALL).unwrap();
    reconcile_consent(&saved.cloud, &mut new.cloud);
    assert!(!new.cloud.providers[0].consent_ok());
    // Model edits keep it.
    let mut f = ui_form();
    f.cloud.providers[0].model = "elevenlabs/scribe-v2".into();
    let (mut new, _) = f.validate(ALL).unwrap();
    reconcile_consent(&saved.cloud, &mut new.cloud);
    assert!(new.cloud.providers[0].consent_ok());
}

#[test]
fn presets_cover_the_developer_list() {
    let presets = flow_windows::cloud::presets();
    if !cfg!(feature = "cloud") {
        assert!(presets.is_empty());
        return;
    }
    let ids: Vec<&str> = presets.iter().map(|p| p.preset.as_str()).collect();
    for want in [
        "openrouter",
        "elevenlabs",
        "groq",
        "openai",
        "deepgram",
        "assemblyai",
        "custom",
    ] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    for p in &presets {
        assert_eq!(p.consent_at_ms, 0, "presets carry no consent");
    }
}
