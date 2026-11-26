//! Team dictionary API against a real in-process murmur-server: any key
//! reads (with ETag / 304), admin keys add, replace and remove, non-admins
//! get 403, strangers 401, bad entries 400, a full list `too_many`, the
//! D112 throttle, `auth = "none"`, and the list surviving a restart. Words
//! are made up.

mod common;

use std::net::SocketAddr;

use common::*;
use murmur_protocol::admin::{code, AdminError, CreatedKey};
use murmur_protocol::dictionary::{TeamDictionary, TeamEntry, CODE_TOO_MANY, MAX_ENTRIES};
use murmur_server::team_dict::TEAM_DICTIONARY_FILE;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Answer {
    status: u16,
    etag: Option<String>,
    retry_after: bool,
    cache: Option<String>,
    body: String,
}

async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
    if_none_match: Option<&str>,
) -> Answer {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let inm = if_none_match.map_or(String::new(), |t| format!("If-None-Match: {t}\r\n"));
    let body = body.unwrap_or("");
    s.write_all(
        format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}{inm}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    let header = |name: &str| {
        head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    Answer {
        status: out[9..12].parse().unwrap(),
        etag: header("etag"),
        retry_after: header("retry-after").is_some(),
        cache: header("cache-control"),
        body: body.to_string(),
    }
}

fn list(a: &Answer) -> Vec<TeamEntry> {
    serde_json::from_str::<TeamDictionary>(&a.body)
        .unwrap_or_else(|e| panic!("{e}: {}", a.body))
        .entries
}

fn err(a: &Answer) -> String {
    serde_json::from_str::<AdminError>(&a.body)
        .unwrap_or_else(|e| panic!("{e}: {}", a.body))
        .code
}

fn e(w: &str, r: Option<&str>) -> TeamEntry {
    TeamEntry {
        word: w.into(),
        replacement: r.map(Into::into),
    }
}

fn body(w: &str, r: Option<&str>) -> String {
    serde_json::to_string(&e(w, r)).unwrap()
}

async fn add(a: SocketAddr, token: &str, w: &str, r: Option<&str>) -> Answer {
    http(
        a,
        "POST",
        "/v1/admin/dictionary",
        Some(token),
        Some(&body(w, r)),
        None,
    )
    .await
}

async fn remove(a: SocketAddr, token: &str, w: &str) -> Answer {
    let b = format!(r#"{{"word":{}}}"#, serde_json::to_string(w).unwrap());
    http(
        a,
        "DELETE",
        "/v1/admin/dictionary",
        Some(token),
        Some(&b),
        None,
    )
    .await
}

async fn get(a: SocketAddr, token: Option<&str>, inm: Option<&str>) -> Answer {
    http(a, "GET", "/v1/dictionary", token, None, inm).await
}

async fn user_key(a: SocketAddr) -> String {
    let b = r#"{"name":"phone","admin":false}"#;
    let r = http(a, "POST", "/v1/admin/keys", Some(TOKEN), Some(b), None).await;
    assert_eq!(r.status, 201, "{}", r.body);
    serde_json::from_str::<CreatedKey>(&r.body).unwrap().secret
}

#[tokio::test(flavor = "multi_thread")]
async fn any_key_reads_with_an_etag_and_304() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    let user = user_key(a).await;
    let r = get(a, Some(&user), None).await;
    assert_eq!((r.status, list(&r)), (200, vec![]));
    assert_eq!(r.cache.as_deref(), Some("no-cache"));
    let empty_tag = r.etag.clone().expect("etag");

    let r = add(a, TOKEN, "Quillon", None).await;
    assert_eq!((r.status, list(&r)), (200, vec![e("Quillon", None)]));
    let tag = r.etag.clone().expect("edit answers carry the new etag");
    assert_ne!(tag, empty_tag);

    // The user sees the change; the old tag is stale, the new one is 304.
    let r = get(a, Some(&user), Some(&empty_tag)).await;
    assert_eq!((r.status, list(&r)), (200, vec![e("Quillon", None)]));
    assert_eq!(r.etag.as_deref(), Some(tag.as_str()));
    let r = get(a, Some(&user), Some(&tag)).await;
    assert_eq!((r.status, r.body.as_str()), (304, ""));
    assert_eq!(r.etag.as_deref(), Some(tag.as_str()));
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_adds_replaces_and_removes() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    assert_eq!(add(a, TOKEN, "zephyr", Some("Zephyr")).await.status, 200);
    assert_eq!(
        add(a, TOKEN, "acme corp", Some("AcmeCorp")).await.status,
        200
    );
    // Same word, other case and spacing: replaced in place, not added.
    let r = add(a, TOKEN, "  ZEPHYR ", Some("ZephyrCo")).await;
    assert_eq!(
        list(&r),
        vec![
            e("ZEPHYR", Some("ZephyrCo")),
            e("acme corp", Some("AcmeCorp"))
        ]
    );
    // An empty replacement is a plain word.
    let r = add(a, TOKEN, "acme corp", Some(" ")).await;
    assert_eq!(list(&r)[1], e("acme corp", None));

    let r = remove(a, TOKEN, "Acme  Corp").await;
    assert_eq!(
        (r.status, list(&r)),
        (200, vec![e("ZEPHYR", Some("ZephyrCo"))])
    );
    let r = remove(a, TOKEN, "acme corp").await;
    assert_eq!((r.status, err(&r).as_str()), (404, code::NOT_FOUND));
    assert!(r.body.contains("not in the team dictionary"), "{}", r.body);
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn non_admins_get_403_and_strangers_401() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    add(a, TOKEN, "Quillon", None).await;
    let user = user_key(a).await;
    let r = add(a, &user, "Brixwell", None).await;
    assert_eq!((r.status, err(&r).as_str()), (403, code::FORBIDDEN));
    let r = remove(a, &user, "Quillon").await;
    assert_eq!((r.status, err(&r).as_str()), (403, code::FORBIDDEN));
    // Nothing changed.
    assert_eq!(
        list(&get(a, Some(&user), None).await),
        vec![e("Quillon", None)]
    );

    for token in [None, Some("wrong-token-0123456789")] {
        let r = get(a, token, None).await;
        assert_eq!((r.status, err(&r).as_str()), (401, code::UNAUTHORIZED));
        let r = http(
            a,
            "POST",
            "/v1/admin/dictionary",
            token,
            Some("not json"),
            None,
        )
        .await;
        assert_eq!(r.status, 401, "auth before the body is read");
    }
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_entries_are_refused_with_a_reason() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    for (w, r, says) in [
        ("", None, "empty"),
        ("<b>Quillon</b>", None, "markup"),
        ("two\nlines", None, "line break"),
        ("one two three four five six seven", None, "at most 6"),
        ("ok", Some("{{template}}"), "the replacement"),
    ] {
        let ans = add(a, TOKEN, w, r).await;
        assert_eq!(
            (ans.status, err(&ans).as_str()),
            (400, code::BAD_REQUEST),
            "{w:?}"
        );
        let m = serde_json::from_str::<AdminError>(&ans.body)
            .unwrap()
            .message;
        assert!(m.contains(says), "{w:?}: {m}");
    }
    let long = "x".repeat(65);
    let ans = add(a, TOKEN, &long, None).await;
    assert_eq!(ans.status, 400);
    let ans = http(
        a,
        "POST",
        "/v1/admin/dictionary",
        Some(TOKEN),
        Some("not json"),
        None,
    )
    .await;
    assert_eq!((ans.status, err(&ans).as_str()), (400, code::BAD_REQUEST));
    let big = body(&"y".repeat(5000), None);
    let ans = http(
        a,
        "POST",
        "/v1/admin/dictionary",
        Some(TOKEN),
        Some(&big),
        None,
    )
    .await;
    assert_eq!(ans.status, 413);
    assert!(list(&get(a, Some(TOKEN), None).await).is_empty());
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_is_capped() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    let store = s.team_dictionary().unwrap();
    for i in 0..MAX_ENTRIES {
        store.upsert(&e(&format!("term{i}"), None)).unwrap();
    }
    let r = add(a, TOKEN, "Brixwell", None).await;
    assert_eq!((r.status, err(&r).as_str()), (400, CODE_TOO_MANY));
    assert_eq!(list(&get(a, Some(TOKEN), None).await).len(), MAX_ENTRIES);
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_survives_a_restart_in_the_state_dir() {
    let d = tempfile::tempdir().unwrap();
    let cfg = || {
        let mut c = mock_cfg("hello");
        c.state_dir = Some(d.path().join("state"));
        c
    };
    let s = start(cfg()).await;
    let r = add(s.addr(), TOKEN, "Velloria", Some("Velloria Inc")).await;
    let tag = r.etag.unwrap();
    s.kill();
    assert!(d.path().join("state").join(TEAM_DICTIONARY_FILE).is_file());
    let s = start(cfg()).await;
    let r = get(s.addr(), Some(TOKEN), None).await;
    assert_eq!(list(&r), vec![e("Velloria", Some("Velloria Inc"))]);
    assert_eq!(r.etag, Some(tag), "same list, same etag across a restart");
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_list_file_is_served_empty_and_never_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let state = d.path().join("state");
    let cfg = || {
        let mut c = mock_cfg("hello");
        c.state_dir = Some(state.clone());
        c
    };
    let s = start(cfg()).await;
    s.kill();
    std::fs::write(state.join(TEAM_DICTIONARY_FILE), "{ damaged").unwrap();
    let s = start(cfg()).await;
    let r = get(s.addr(), Some(TOKEN), None).await;
    assert_eq!((r.status, list(&r)), (200, vec![]));
    let r = add(s.addr(), TOKEN, "Velloria", None).await;
    assert_eq!((r.status, err(&r)), (500, "store_corrupt".to_string()));
    let r = remove(s.addr(), TOKEN, "Velloria").await;
    assert_eq!((r.status, err(&r)), (500, "store_corrupt".to_string()));
    assert_eq!(
        std::fs::read_to_string(state.join(TEAM_DICTIONARY_FILE)).unwrap(),
        "{ damaged"
    );
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn throttled_peers_get_429_before_the_key_is_checked() {
    let mut c = mock_cfg("hello");
    c.auth_throttle.max_failures = 2;
    c.auth_throttle.exempt_loopback = false;
    let s = start(c).await;
    let a = s.addr();
    assert_eq!(get(a, Some("wrong-0"), None).await.status, 401);
    assert_eq!(get(a, Some("wrong-1"), None).await.status, 401);
    let r = get(a, Some(TOKEN), None).await;
    assert_eq!(
        (r.status, err(&r).as_str(), r.retry_after),
        (429, code::THROTTLED, true)
    );
    let r = add(a, TOKEN, "Quillon", None).await;
    assert_eq!(r.status, 429);
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_off_has_no_team_dictionary() {
    let mut c = mock_cfg("hello");
    c.token = None;
    c.auth = murmur_server::config::AuthMode::None;
    let s = start(c).await;
    assert!(s.team_dictionary().is_none());
    let r = get(s.addr(), None, None).await;
    assert_eq!((r.status, err(&r).as_str()), (404, code::AUTH_OFF));
    let r = add(s.addr(), TOKEN, "Quillon", None).await;
    assert_eq!((r.status, err(&r).as_str()), (404, code::AUTH_OFF));
    s.kill();
}
