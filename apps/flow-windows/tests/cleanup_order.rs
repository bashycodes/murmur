//! D36 cleanup order on Windows: prefs.json holds all three engines (this
//! PC, your server, OpenRouter) with an on/off flag each; a pre-D36 file
//! (`allow_cloud_cleanup` + `[cloud, local]` / `[local, cloud]`) is
//! migrated on load; the router always gets the whole order with the
//! switched-off entries in `cleanup_off`.

use flow_router::{CleanupRoute, RouterConfig};
use flow_windows::dev_settings::{BuildCaps, CleanupEngine, DevSettings};
use flow_windows::openrouter::{self, CleanupForm};
use flow_windows::prefs::Prefs;
use std::path::PathBuf;
use CleanupRoute::{Cloud, Local, Server};

const ALL: BuildCaps = BuildCaps {
    local: true,
    remote: true,
    cloud: true,
};

fn e(route: CleanupRoute, enabled: bool) -> CleanupEngine {
    CleanupEngine { route, enabled }
}

fn tmp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flow-win-d36-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// prefs.json with this `developer` section and this PC's AI on, loaded the
/// way the app does.
fn load(tag: &str, developer: serde_json::Value) -> Prefs {
    load_with(tag, serde_json::json!({"llm_enabled": true}), developer)
}

/// prefs.json with these top-level prefs and this `developer` section.
fn load_with(tag: &str, top: serde_json::Value, developer: serde_json::Value) -> Prefs {
    let dir = tmp(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prefs.json");
    let mut v = serde_json::json!({"prefs_rev": 3, "developer": developer});
    for (k, x) in top.as_object().unwrap() {
        v[k] = x.clone();
    }
    let text = v.to_string();
    std::fs::write(&path, text).unwrap();
    let (p, _) = Prefs::load(&path);
    let _ = std::fs::remove_dir_all(&dir);
    p
}

fn router(dev: &DevSettings, caps: BuildCaps) -> RouterConfig {
    let cfg: RouterConfig = serde_json::from_value(dev.router_config_json(caps, "")).unwrap();
    cfg.validate().unwrap();
    cfg
}

#[test]
fn defaults_are_local_server_cloud_with_cloud_off() {
    let d = DevSettings::for_build(ALL);
    assert_eq!(
        d.cleanup(),
        vec![e(Local, true), e(Server, true), e(Cloud, false)]
    );
    let cfg = router(&d, ALL);
    assert_eq!(cfg.cleanup_order, vec![Local, Server, Cloud]);
    assert_eq!(cfg.cleanup_off, vec![Cloud]);
    assert!(!cfg.allow_cloud_cleanup && !cfg.cloud_cleanup_wanted());
    assert_eq!(cfg.effective_cleanup_order(), vec![Local, Server]);
}

#[test]
fn old_prefs_migrate_keeping_cloud_vs_local_and_adding_the_server_after_this_pc() {
    let cases = [
        // (pre-D36 section, migrated order)
        (
            serde_json::json!({}),
            vec![e(Local, true), e(Server, true), e(Cloud, false)],
        ),
        (
            serde_json::json!({"allow_cloud_cleanup": true, "cleanup_order": ["cloud", "local"]}),
            vec![e(Cloud, true), e(Local, true), e(Server, true)],
        ),
        (
            serde_json::json!({"allow_cloud_cleanup": true, "cleanup_order": ["local", "cloud"]}),
            vec![e(Local, true), e(Server, true), e(Cloud, true)],
        ),
        // Cloud cleanup was not allowed: it keeps its place, switched off.
        (
            serde_json::json!({"allow_cloud_cleanup": false, "cleanup_order": ["cloud", "local"]}),
            vec![e(Cloud, false), e(Local, true), e(Server, true)],
        ),
        // Allowed but not in the order: it never cleaned, so it stays off.
        (
            serde_json::json!({"allow_cloud_cleanup": true, "cleanup_order": ["local"]}),
            vec![e(Local, true), e(Server, true), e(Cloud, false)],
        ),
        (
            serde_json::json!({"allow_cloud_cleanup": true, "cleanup_order": ["cloud"]}),
            vec![e(Cloud, true), e(Local, true), e(Server, true)],
        ),
    ];
    for (i, (old, want)) in cases.into_iter().enumerate() {
        // Read before the migration ran (e.g. a DevSettings from JSON)...
        let raw: DevSettings = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(raw.cleanup(), want, "case {i}: {old}");
        // ...and as the app loads prefs.json.
        let p = load(&format!("mig{i}"), old.clone());
        assert_eq!(p.developer.cleanup_engines, want, "case {i}: {old}");
        assert!(
            !p.developer.legacy_allow_cloud_cleanup && p.developer.legacy_cleanup_order.is_empty()
        );
        let cloud_on = want.iter().any(|x| x.route == Cloud && x.enabled);
        assert_eq!(p.developer.cleanup_effective(ALL), cloud_on, "case {i}");
        // The saved file is the new format only, and loads back the same.
        let j = serde_json::to_value(&p.developer).unwrap();
        assert!(
            j.get("allow_cloud_cleanup").is_none() && j.get("cleanup_order").is_none(),
            "case {i}"
        );
        let back: DevSettings = serde_json::from_value(j).unwrap();
        assert_eq!(back.cleanup(), want, "case {i}");
        // The router gets the same order and the same cloud switch.
        let cfg = router(&p.developer, ALL);
        assert_eq!(
            cfg.cleanup_order,
            want.iter().map(|x| x.route).collect::<Vec<_>>(),
            "case {i}"
        );
        assert_eq!(cfg.allow_cloud_cleanup, cloud_on, "case {i}");
    }
}

#[test]
fn a_partial_or_repeated_list_on_disk_is_completed() {
    let p = load(
        "partial",
        serde_json::json!({"cleanup_engines": [
            {"route": "server", "enabled": false},
            {"route": "server", "enabled": true},
            {"route": "cloud", "enabled": true}
        ]}),
    );
    assert_eq!(
        p.developer.cleanup_engines,
        vec![e(Server, false), e(Cloud, true), e(Local, true)]
    );
    // The new list wins over leftover pre-D36 fields.
    let p = load(
        "both",
        serde_json::json!({
            "allow_cloud_cleanup": true,
            "cleanup_order": ["cloud", "local"],
            "cleanup_engines": [
                {"route": "local", "enabled": true},
                {"route": "server", "enabled": true},
                {"route": "cloud", "enabled": false}
            ]
        }),
    );
    assert_eq!(
        p.developer.cleanup_engines,
        vec![e(Local, true), e(Server, true), e(Cloud, false)]
    );
}

#[test]
fn switched_off_engines_keep_their_place_for_the_router() {
    let mut d = DevSettings::for_build(ALL);
    openrouter::apply_cleanup(
        &mut d,
        &CleanupForm {
            allow: true,
            consent: true,
            timeout_ms: 2500,
            sort: "latency".into(),
            ..Default::default()
        },
        1,
    )
    .unwrap();
    openrouter::apply_cleanup_engines(&mut d, &[e(Server, false), e(Cloud, true), e(Local, true)])
        .unwrap();
    let cfg = router(&d, ALL);
    assert_eq!(cfg.cleanup_order, vec![Server, Cloud, Local]);
    assert_eq!(cfg.cleanup_off, vec![Server]);
    assert!(cfg.allow_cloud_cleanup && cfg.cloud_cleanup_wanted());
    assert_eq!(cfg.effective_cleanup_order(), vec![Cloud, Local]);

    // Everything off is still the whole list (not the pre-D36 empty order).
    openrouter::apply_cleanup_engines(&mut d, &[e(Server, false), e(Cloud, true), e(Local, false)])
        .unwrap();
    openrouter::apply_cleanup(
        &mut d,
        &CleanupForm {
            allow: false,
            timeout_ms: 2500,
            ..Default::default()
        },
        2,
    )
    .unwrap();
    let cfg = router(&d, ALL);
    assert_eq!(cfg.cleanup_order, vec![Server, Cloud, Local]);
    assert_eq!(cfg.cleanup_off, vec![Server, Cloud, Local]);
    assert!(cfg.effective_cleanup_order().is_empty());
}

#[test]
fn an_engine_the_build_lacks_is_off_for_the_router() {
    let mut d = DevSettings::for_build(ALL);
    d.set_cleanup_on(Cloud, true);
    let client_only = BuildCaps {
        local: true,
        remote: false,
        cloud: false,
    };
    let cfg = router(&d, client_only);
    assert_eq!(cfg.cleanup_order, vec![Local, Server, Cloud]);
    assert_eq!(cfg.cleanup_off, vec![Server, Cloud]);
    assert!(!cfg.allow_cloud_cleanup);
    let server_only = BuildCaps {
        local: false,
        remote: true,
        cloud: false,
    };
    assert_eq!(
        router(&d, server_only).effective_cleanup_order(),
        vec![Server]
    );
}

#[test]
fn an_upgrade_with_ai_cleanup_off_keeps_it_off_but_a_fresh_install_has_the_server_on() {
    use serde_json::json;
    let cases = [
        // (top-level prefs, pre-D36 developer section, server on after the upgrade)
        // This PC's AI off and no cloud cleanup: nothing cleaned, so the server starts off.
        (json!({"llm_enabled": false}), json!({}), false),
        (
            json!({"llm_enabled": false}),
            json!({"allow_cloud_cleanup": false, "cleanup_order": ["cloud", "local"]}),
            false,
        ),
        // "No AI" level: off, whatever engine was on.
        (
            json!({"llm_enabled": true, "cleanup_level": "none"}),
            json!({}),
            false,
        ),
        (
            json!({"llm_enabled": false, "cleanup_level": "none"}),
            json!({"allow_cloud_cleanup": true, "cleanup_order": ["cloud", "local"]}),
            false,
        ),
        // Some AI cleaned before: the server joins after this PC.
        (json!({"llm_enabled": true}), json!({}), true),
        (
            json!({"llm_enabled": false}),
            json!({"allow_cloud_cleanup": true, "cleanup_order": ["cloud", "local"]}),
            true,
        ),
    ];
    for (i, (top, dev, server_on)) in cases.into_iter().enumerate() {
        let p = load_with(&format!("off{i}"), top.clone(), dev.clone());
        assert_eq!(
            p.developer.cleanup_on(Server),
            server_on,
            "case {i}: {top} {dev}"
        );
        // It keeps its place (right after this PC) and the router hears it is off.
        let order: Vec<_> = p
            .developer
            .cleanup_engines
            .iter()
            .map(|x| x.route)
            .collect();
        let at = order.iter().position(|r| *r == Local).unwrap();
        assert_eq!(order[at + 1], Server, "case {i}");
        let cfg = router(&p.developer, ALL);
        assert_eq!(cfg.cleanup_off.contains(&Server), !server_on, "case {i}");
        // Migrated once: turning this PC's AI on later does not flip it.
        let mut again = p.clone();
        again.llm_enabled = true;
        again.migrate();
        assert_eq!(again.developer.cleanup_on(Server), server_on, "case {i}");
    }
    // A fresh install (no prefs.json) has the server on, AI off or not.
    let dir = tmp("fresh");
    let (p, _) = Prefs::load(&dir.join("prefs.json"));
    assert!(!p.llm_enabled && p.developer.cleanup_on(Server));
    // ...and still after it was saved and loaded back.
    std::fs::create_dir_all(&dir).unwrap();
    p.save(&dir.join("prefs.json")).unwrap();
    let (back, _) = Prefs::load(&dir.join("prefs.json"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(back.developer.cleanup_on(Server));
}
