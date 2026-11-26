//! Send to webhook (D105): a dictation made with the webhook shortcut is
//! POSTed as JSON to the URL set in Settings instead of being pasted.
//!
//! Consent (D105, like D52): the user typed the URL and set the shortcut,
//! and Settings says what is sent there; that is the consent for this
//! route, which bypasses the router's cloud gate.
//!
//! Addresses (D116, like the user's own server since D111): `https://` or
//! plain `http://`, to any address, with no opt-in. The URL is the user's
//! choice; [`check_url`] checks only its shape. A name is resolved the
//! usual way when sending.
//!
//! One attempt, at most [`TIMEOUT`], no redirects, no retry. The URL is
//! user config, not a secret, but its query string may carry a token:
//! messages and logs only ever show [`redact`]ed URLs.

use serde::Serialize;
use std::time::Duration;

/// The whole request (connect, send, answer) gives up after this.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Webhooks need network code: not in the client-only (`local,app`) build.
pub const CAN_WEBHOOK: bool = cfg!(feature = "webhook");

/// Overlay text after a successful send.
pub const SENT: &str = "Sent to webhook";
/// Overlay text when Esc came too late: the request had already gone out.
pub const ALREADY_SENT: &str = "Already sent to webhook";

/// The JSON body: `{text, raw, app, time}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Payload {
    /// The cleaned text (what would have been pasted).
    pub text: String,
    /// The transcript before cleanup.
    pub raw: String,
    /// The foreground app the dictation targeted (exe name, `""` unknown).
    pub app: String,
    /// When the dictation was sent, ISO 8601 UTC.
    pub time: String,
}

/// Why a send failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The server answered with a non-2xx status.
    Status(u16),
    /// No answer: connection, timeout, bad URL.
    Other(String),
}

/// What became of one webhook send (`Input::WebhookFinished`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A 2xx answer.
    Sent,
    /// A non-2xx answer.
    Status(u16),
    /// No answer (connection, timeout, bad URL).
    Error(String),
    /// Esc came before the request went out: nothing was sent.
    Cancelled,
}

impl Outcome {
    /// The pill text of a failure (`Webhook failed (HTTP 500)`); `None`
    /// for a send or a cancel.
    pub fn failure(&self) -> Option<String> {
        match self {
            Outcome::Status(code) => Some(format!("Webhook failed (HTTP {code})")),
            Outcome::Error(why) => Some(format!("Webhook failed: {why}")),
            Outcome::Sent | Outcome::Cancelled => None,
        }
    }
}

impl From<Result<u16, Failure>> for Outcome {
    fn from(r: Result<u16, Failure>) -> Self {
        match r {
            Ok(_) => Outcome::Sent,
            Err(Failure::Status(code)) => Outcome::Status(code),
            Err(Failure::Other(why)) => Outcome::Error(why),
        }
    }
}

const SCHEME_ERR: &str = "the webhook URL must start with https:// or http://";

/// Check a webhook URL's shape (Settings save, and again before each
/// send). Errors never contain the query string.
pub fn check_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.chars().any(char::is_whitespace) {
        return Err("the webhook URL must not contain spaces".into());
    }
    let (scheme, rest) = url.split_once("://").ok_or(SCHEME_ERR)?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http") {
        return Err(SCHEME_ERR.into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("the webhook URL must not contain a user name or password".into());
    }
    let (host, port) = split_host_port(authority)?;
    murmur_router::net::validate_host(host).map_err(|e| format!("the webhook URL's host: {e}"))?;
    if let Some(p) = port {
        p.parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("the webhook URL's port must be 1-65535")?;
    }
    Ok(())
}

/// `host[:port]` or `[v6][:port]` → (host, port).
fn split_host_port(authority: &str) -> Result<(&str, Option<&str>), String> {
    if authority.is_empty() {
        return Err("the webhook URL has no host".into());
    }
    if let Some(after) = authority.strip_prefix('[') {
        let (v6, tail) = after
            .split_once(']')
            .ok_or("the webhook URL's IPv6 address has no closing ]")?;
        return match tail {
            "" => Ok((v6, None)),
            t => t
                .strip_prefix(':')
                .map(|p| (v6, Some(p)))
                .ok_or_else(|| "the webhook URL's port is not valid".into()),
        };
    }
    Ok(match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    })
}

/// The URL without its query string and fragment: safe to show and log.
pub fn redact(url: &str) -> String {
    let url = url.trim();
    let cut = url.find(['?', '#']).unwrap_or(url.len());
    let mut s = url[..cut].to_string();
    if cut < url.len() {
        s.push_str("?…");
    }
    s
}

/// The body for `text` (cleaned) and `raw`, dictated into `app`, sent at
/// `now_ms` (Unix ms).
pub fn body(text: &str, raw: &str, app: &str, now_ms: u64) -> String {
    let p = Payload {
        text: text.into(),
        raw: raw.into(),
        app: app.into(),
        time: crate::debug_log::iso8601_ms(now_ms),
    };
    serde_json::to_string(&p).expect("strings always serialize")
}

/// POST `body` (JSON) to `url`: `Ok(status)` on a 2xx answer.
#[cfg(feature = "webhook")]
pub fn post(url: &str, body: &str) -> Result<u16, Failure> {
    post_with(url, body, &|b| b)
}

/// [`post`] with a last say over the client (tests: point a public name
/// at a local listener).
#[cfg(feature = "webhook")]
fn post_with(
    url: &str,
    body: &str,
    configure: &dyn Fn(reqwest::blocking::ClientBuilder) -> reqwest::blocking::ClientBuilder,
) -> Result<u16, Failure> {
    check_url(url).map_err(Failure::Other)?;
    let builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connect_timeout(TIMEOUT)
        .timeout(TIMEOUT)
        .user_agent(concat!("murmur/", env!("CARGO_PKG_VERSION")));
    let client = configure(builder)
        .build()
        .map_err(|e| Failure::Other(e.without_url().to_string()))?;
    let r = client
        .post(url.trim())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .map_err(|e| {
            Failure::Other(if e.is_timeout() {
                timeout_message()
            } else if e.is_connect() {
                "could not connect".into()
            } else {
                e.without_url().to_string()
            })
        })?;
    let code = r.status().as_u16();
    if r.status().is_success() {
        Ok(code)
    } else {
        Err(Failure::Status(code))
    }
}

/// The failure text of a send that got no answer in time.
pub fn timeout_message() -> String {
    format!("no answer within {} s", TIMEOUT.as_secs())
}

/// Without the `webhook` feature there is no network code to send with.
#[cfg(not(feature = "webhook"))]
pub fn post(_url: &str, _body: &str) -> Result<u16, Failure> {
    Err(Failure::Other("this build cannot send to a webhook".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_and_http_to_any_host() {
        for u in [
            "https://hooks.example.com/in?token=abc",
            "HTTP://hooks.example.com:8080/in",
            "http://127.0.0.1:8080/hook",
            "http://localhost/hook",
            "http://192.168.1.20/hook",
            "http://10.0.0.5:5678/webhook/x",
            "http://100.64.0.10/hook",
            "http://[fd00::1]:9000/",
            "http://my-nas/hook",
            // D116: public addresses too, by IP or by name.
            "http://203.0.113.9/hook",
            "http://[2001:db8::1]:8080/hook",
            "http://hooks.example.com/in?token=abc",
        ] {
            assert!(check_url(u).is_ok(), "{u}");
        }
    }

    #[test]
    fn bad_urls_are_refused_without_echoing_the_query() {
        for u in [
            "",
            "hooks.example.com/in",
            "ftp://example.com/x",
            "https://",
            "https://a b.example.com/",
            "https://user:pw@example.com/",
            "https://example.com:0/",
            "https://example.com:99999/",
            "https://[fd00::1/",
        ] {
            assert!(check_url(u).is_err(), "{u}");
        }
        let e = check_url("ftp://203.0.113.9/hook?token=s3cret").unwrap_err();
        assert!(!e.contains("s3cret"), "{e}");
        assert_eq!(e, SCHEME_ERR);
    }

    #[test]
    fn redact_drops_the_query_and_fragment() {
        assert_eq!(
            redact("https://hooks.example.com/in?token=s3cret"),
            "https://hooks.example.com/in?…"
        );
        assert_eq!(
            redact("https://h.example.com/a#frag"),
            "https://h.example.com/a?…"
        );
        assert_eq!(
            redact(" https://h.example.com/a "),
            "https://h.example.com/a"
        );
    }

    #[test]
    fn body_has_text_raw_app_and_an_iso_time() {
        let b = body(
            "Hello there.",
            "hello there",
            "notepad.exe",
            1_790_000_000_123,
        );
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["text"], "Hello there.");
        assert_eq!(v["raw"], "hello there");
        assert_eq!(v["app"], "notepad.exe");
        assert_eq!(v["time"], "2026-09-21T14:13:20.123Z");
    }

    #[test]
    fn outcomes_name_the_http_status() {
        assert_eq!(
            Outcome::from(Err(Failure::Status(503)))
                .failure()
                .as_deref(),
            Some("Webhook failed (HTTP 503)")
        );
        assert_eq!(
            Outcome::from(Err(Failure::Other("could not connect".into())))
                .failure()
                .as_deref(),
            Some("Webhook failed: could not connect")
        );
        assert_eq!(Outcome::from(Ok(204)), Outcome::Sent);
        assert_eq!(Outcome::Sent.failure(), None);
        assert_eq!(Outcome::Cancelled.failure(), None);
    }

    /// A one-request HTTP server on loopback: answers `status`, after
    /// `delay`, and hands back the request it read.
    #[cfg(feature = "webhook")]
    fn mock(status: &'static str, delay: Duration) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            // Headers, then Content-Length bytes of body.
            loop {
                let n = s.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end]
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if req.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            std::thread::sleep(delay);
            let _ = write!(
                s,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            String::from_utf8_lossy(&req).to_string()
        });
        (format!("http://127.0.0.1:{port}/hook?token=abc"), h)
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn posts_the_json_body_to_a_local_server() {
        let (url, h) = mock("200 OK", Duration::ZERO);
        let b = body("Hi.", "hi", "notepad.exe", 0);
        assert_eq!(post(&url, &b), Ok(200));
        let req = h.join().unwrap();
        assert!(
            req.starts_with("POST /hook?token=abc HTTP/1.1\r\n"),
            "{req}"
        );
        assert!(req
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        assert!(req.ends_with(&b), "{req}");
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn a_non_2xx_answer_is_a_failure_with_its_status() {
        let (url, h) = mock("500 Internal Server Error", Duration::ZERO);
        assert_eq!(post(&url, "{}"), Err(Failure::Status(500)));
        h.join().unwrap();
        let (url, h) = mock("302 Found", Duration::ZERO);
        assert_eq!(
            post(&url, "{}"),
            Err(Failure::Status(302)),
            "no redirect follow"
        );
        h.join().unwrap();
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn a_silent_server_times_out_after_ten_seconds_without_retrying() {
        let (url, h) = mock("200 OK", TIMEOUT + Duration::from_secs(2));
        let t = std::time::Instant::now();
        let r = post(&url, "{}");
        let took = t.elapsed();
        assert_eq!(r, Err(Failure::Other(timeout_message())));
        assert_eq!(timeout_message(), "no answer within 10 s");
        assert!(
            took >= TIMEOUT && took < TIMEOUT + Duration::from_secs(2),
            "{took:?}"
        );
        // One request only: the mock served exactly one and exits.
        h.join().unwrap();
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn nothing_listening_is_a_connect_failure_without_the_query() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let r = post(&format!("http://127.0.0.1:{port}/h?token=s3cret"), "{}");
        let Err(Failure::Other(why)) = r else {
            panic!("{r:?}")
        };
        assert!(!why.contains("s3cret"), "{why}");
    }

    #[cfg(feature = "webhook")]
    #[test]
    fn cleartext_to_a_public_name_is_sent() {
        // D116: plain http:// to a public name goes out like any other
        // send. The name is pointed at the local mock: no real DNS.
        let (local, h) = mock("204 No Content", Duration::ZERO);
        let addr: std::net::SocketAddr = local
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let url = format!("http://hooks.example.com:{}/in?token=abc", addr.port());
        let b = body("Hi.", "hi", "notepad.exe", 0);
        let r = post_with(&url, &b, &|c| c.resolve("hooks.example.com", addr));
        assert_eq!(r, Ok(204));
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /in?token=abc HTTP/1.1\r\n"), "{req}");
        assert!(
            req.to_ascii_lowercase()
                .contains(&format!("host: hooks.example.com:{}", addr.port())),
            "{req}"
        );
        assert!(req.ends_with(&b), "{req}");
    }
}
