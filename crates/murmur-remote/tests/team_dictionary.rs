//! The team dictionary client against a real in-process murmur-server: an
//! admin adds entries, a user's app downloads and caches them, the next
//! refresh is a 304, and with the server gone the cached copy is used.
//! Words are made up.

use murmur_core::config::{AsrBackend, ModelConfig};
use murmur_protocol::admin::code;
use murmur_protocol::dictionary::TeamEntry;
use murmur_remote::admin::AdminClient;
use murmur_remote::team_dict::TeamDictCache;
use murmur_router::ServerConfig;
use murmur_server::Config;

const TOKEN: &str = "test-token-0123456789abcdef";

fn e(w: &str, r: Option<&str>) -> TeamEntry {
    TeamEntry {
        word: w.into(),
        replacement: r.map(Into::into),
    }
}

#[test]
fn download_cache_304_and_offline() {
    let d = tempfile::tempdir().unwrap();
    let cfg = Config {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model: ModelConfig {
            asr_backend: AsrBackend::Mock { text: "x".into() },
            ..Default::default()
        },
        state_dir: Some(d.path().join("state")),
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        murmur_server::serve(cfg, vec![l]).await.unwrap()
    });
    let server = |token: &str| ServerConfig {
        host: "127.0.0.1".into(),
        port: handle.addr().port(),
        token: token.into(),
        connect_timeout_ms: 500,
        ..Default::default()
    };
    let admin = AdminClient::new(server(TOKEN), "test");
    let user_secret = admin.create_key("phone", false).unwrap().secret;
    let user = AdminClient::new(server(&user_secret), "test");

    // The admin's app caches what its edit answered.
    let admin_cache = TeamDictCache::new(d.path().join("admin-cache.json"));
    let a = admin.add_team_entry(&e("zephyr", Some("Zephyr"))).unwrap();
    assert_eq!(
        admin_cache.edited(&admin, &a),
        vec![e("zephyr", Some("Zephyr"))]
    );
    admin.add_team_entry(&e("Quillon", None)).unwrap();
    // A user's key can't edit.
    let err = user.add_team_entry(&e("Brixwell", None)).unwrap_err();
    assert_eq!(err.code, code::FORBIDDEN);
    let err = user.remove_team_entry("Quillon").unwrap_err();
    assert_eq!(err.code, code::FORBIDDEN);

    let cache = TeamDictCache::new(d.path().join("user").join("team-dictionary.json"));
    let r = cache.refresh(&user);
    assert_eq!(r.error, None);
    assert!(r.changed);
    assert_eq!(
        r.entries,
        vec![e("zephyr", Some("Zephyr")), e("Quillon", None)]
    );
    // Unchanged on the server: 304, the cached copy.
    assert!(user.team_dictionary(Some("\"stale\"")).unwrap().is_some());
    let r = cache.refresh(&user);
    assert_eq!((r.changed, r.error, r.entries.len()), (false, None, 2));

    // An edit shows up on the next refresh.
    admin.remove_team_entry("ZEPHYR").unwrap();
    let r = cache.refresh(&user);
    assert_eq!(
        (r.changed, r.entries.clone()),
        (true, vec![e("Quillon", None)])
    );

    // The server goes away: the cache is used.
    handle.kill();
    drop(rt);
    let r = cache.refresh(&user);
    assert_eq!(r.entries, vec![e("Quillon", None)]);
    assert_eq!(
        r.error.map(|e| e.code).as_deref(),
        Some(code::CONNECT_FAILED)
    );
    assert_eq!(cache.load(&user.server()), vec![e("Quillon", None)]);
}
