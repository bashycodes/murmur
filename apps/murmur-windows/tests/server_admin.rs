//! D39 Users & keys end to end on the Rust side of the page:
//! `server_admin` (what the `admin_*` commands call) → murmur-remote's admin
//! client → an in-process murmur-server. Covers admin status, list, create
//! (secret once), revoke, the last-admin guard and a non-admin key.

#![cfg(feature = "remote")]

use murmur_windows::dev_settings::{BuildCaps, DevSettings, ServerSettings};
use murmur_windows::server_admin::{self, AdminApi};

const TOKEN: &str = "test-token-0123456789abcdef";

struct Server {
    handle: murmur_server::ServerHandle,
    _rt: tokio::runtime::Runtime,
}

fn start() -> Server {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.block_on(async {
        let cfg = murmur_server::Config {
            bind: vec!["127.0.0.1:0".into()],
            token: Some(TOKEN.into()),
            model: murmur_core::config::ModelConfig {
                asr_backend: murmur_core::config::AsrBackend::Mock { text: "x".into() },
                ..Default::default()
            },
            ..Default::default()
        };
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        murmur_server::serve(cfg, vec![l]).await.unwrap()
    });
    Server { handle, _rt: rt }
}

fn dev(s: &Server, token: &str) -> DevSettings {
    DevSettings {
        allow_server: true,
        server: ServerSettings {
            host: "127.0.0.1".into(),
            port: s.handle.addr().port(),
            connect_timeout_ms: 500,
            ..ServerSettings::default()
        },
        token_dpapi: murmur_windows::secrets::protect(token).unwrap(),
        ..DevSettings::for_build(BuildCaps::current())
    }
}

fn api(d: &DevSettings) -> Box<dyn AdminApi + Send> {
    server_admin::for_settings(d).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn owner_manages_keys_from_the_app() {
    let s = start();
    let owner = dev(&s, TOKEN);
    let st = server_admin::status(&owner);
    assert!(st.configured && st.admin, "{st:?}");
    assert_eq!(st.name, "owner");

    let a = api(&owner);
    let l = a.list().unwrap();
    assert_eq!(l.keys.len(), 1);
    assert!(l.keys[0].current);

    let created = a.create("  phone  ", false).unwrap();
    assert_eq!(created.key.name, "phone", "trimmed");
    assert_eq!(created.secret.len(), 43);
    assert!(a.create("", false).is_err());
    assert!(a
        .create("phone", false)
        .unwrap_err()
        .contains("already exists"));

    // The new key is not an admin: no page, and the API says so.
    let phone = dev(&s, &created.secret);
    let st = server_admin::status(&phone);
    assert!(st.configured && !st.admin && st.error.is_none(), "{st:?}");
    assert!(api(&phone).list().unwrap_err().contains("not an admin key"));

    // Last admin: refused; the message tells what to do.
    let e = a.revoke(&l.keys[0].id).unwrap_err();
    assert!(e.contains("last admin key"), "{e}");

    let r = a.revoke(&created.key.id).unwrap();
    assert!(r.key.revoked.is_some());
    let st = server_admin::status(&phone);
    assert!(
        !st.admin && st.code.as_deref() == Some("unauthorized"),
        "{st:?}"
    );
    assert_eq!(a.list().unwrap().keys.len(), 2);
    s.handle.kill();
}

#[test]
fn server_down_or_unsaved_is_a_status_not_a_panic() {
    let s = start();
    let mut d = dev(&s, TOKEN);
    s.handle.kill();
    let st = server_admin::status(&d);
    assert!(
        st.configured && !st.admin && st.code.as_deref() == Some("connect_failed"),
        "{st:?}"
    );
    d.token_dpapi.clear();
    let st = server_admin::status(&d);
    assert!(!st.configured && st.error.is_some());
}

/// The team dictionary from the app's side: the admin adds words, another
/// key downloads and caches them, the next refresh is a 304, the cached
/// copy is used once the server is gone, and the merged dictation list puts
/// the personal entry first. A non-admin can't edit.
#[test]
fn team_dictionary_reaches_dictation_and_survives_offline() {
    use murmur_windows::team_dictionary as team;
    let s = start();
    let owner = dev(&s, TOKEN);
    let dir = std::env::temp_dir().join(format!("murmur-win-team-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (owner_cache, phone_cache) = (dir.join("owner.json"), dir.join("phone.json"));

    let v = team::add(&owner, &owner_cache, " zephyr ", "ZephyrCo").unwrap();
    assert_eq!(v.entries[0].word, "zephyr");
    assert_eq!(v.entries[0].replacement.as_deref(), Some("ZephyrCo"));
    team::add(&owner, &owner_cache, "Quillon", "").unwrap();
    let e = team::add(
        &owner,
        &owner_cache,
        "one two three four five six seven",
        "",
    )
    .unwrap_err();
    assert!(e.contains("at most 6"), "{e}");

    let secret = api(&owner).create("phone", false).unwrap().secret;
    let phone = dev(&s, &secret);
    assert!(team::cached(&phone, &phone_cache).entries.is_empty());
    let v = team::refresh(&phone, &phone_cache);
    assert!(v.configured && v.error.is_none(), "{v:?}");
    assert_eq!(v.entries.len(), 2);
    assert_eq!(team::refresh(&phone, &phone_cache), v, "304: same list");
    let e = team::add(&phone, &phone_cache, "Brixwell", "").unwrap_err();
    assert!(e.contains("not an admin key"), "{e}");

    // A personal "zephyr" wins over the team's.
    let p = murmur_windows::prefs::Prefs {
        dictionary: vec![murmur_windows::core_api::DictEntry {
            word: "Zephyr".into(),
            replacement: Some("Zephyr Labs".into()),
            is_snippet: false,
        }],
        team_dictionary: team::to_dict(&v.entries),
        ..Default::default()
    };
    let d = p.dictation_dictionary();
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!(d[0].replacement.as_deref(), Some("Zephyr Labs"));
    assert_eq!(d[1].word, "Quillon");

    // Offline: the saved copy, with the reason.
    s.handle.kill();
    let off = team::refresh(&phone, &phone_cache);
    assert_eq!(off.entries, v.entries);
    assert!(off.error.is_some(), "{off:?}");
    assert_eq!(team::cached(&phone, &phone_cache).entries, v.entries);
    let _ = std::fs::remove_dir_all(&dir);
}
