//! Review S2: one URL parser (the `url` crate, which reqwest uses) decides
//! the host for consent, the LAN/VPN class and the cleartext rule.
//! These vectors are the contract the Kotlin mirror (`CloudSettings.kt`)
//! must match.

use murmur_router::cloud::{parse_base_url, BaseUrl};
use murmur_router::*;

fn provider(url: &str) -> CloudProviderConfig {
    CloudProviderConfig {
        id: "p-1".into(),
        base_url: url.into(),
        model: "m".into(),
        ..Default::default()
    }
}

#[test]
fn ambiguous_urls_are_rejected() {
    for bad in [
        // `\` is `/` for WHATWG: the request would go to evil.com.
        "http://evil.com\\.local/v1",
        "https://evil.com\\@api.groq.com/v1",
        "http://evil.com\\x.local",
        // userinfo in any form
        "https://user:pw@api.groq.com/v1",
        "https://user@api.groq.com/v1",
        "https://:pw@api.groq.com/v1",
        "https://api.groq.com@evil.com/v1",
        // whitespace / control characters (url strips tabs and newlines)
        "https://api.groq.com /v1",
        "https://api.\tgroq.com/v1",
        "https://api.groq.com/v1\n",
        "https://api.gr\u{0}oq.com/v1",
        " https://api.groq.com/v1",
        // not http(s), no host
        "ftp://api.groq.com",
        "https://",
        "file:///etc/passwd",
        "api.groq.com/v1",
        "",
    ] {
        assert!(parse_base_url(bad).is_err(), "{bad:?} must be rejected");
        assert!(provider(bad).validate().is_err(), "{bad:?}");
        assert_eq!(provider(bad).host(), "", "{bad:?}");
    }
}

#[test]
fn hosts_come_from_the_parser() {
    let cases: [(&str, bool, &str); 9] = [
        ("HTTPS://API.Groq.com/openai/v1", true, "api.groq.com"),
        ("https://api.groq.com:443/v1", true, "api.groq.com"),
        ("https://api.groq.com#@evil.com", true, "api.groq.com"),
        ("https://api.groq.com?x=@evil.com", true, "api.groq.com"),
        ("http://192.168.1.5:8000/v1", false, "192.168.1.5"),
        ("http://[FD00::1]:8080/v1", false, "fd00::1"),
        // WHATWG IPv4 forms are numbers, not names.
        ("http://0x7f.1/v1", false, "127.0.0.1"),
        ("http://2130706433/v1", false, "127.0.0.1"),
        // percent-encoded host bytes are decoded before classification
        ("http://desk%2elocal/v1", false, "desk.local"),
    ];
    for (u, https, host) in cases {
        let b: BaseUrl = parse_base_url(u).unwrap_or_else(|e| panic!("{u}: {e}"));
        assert_eq!((b.https, b.host.as_str()), (https, host), "{u}");
        assert_eq!(provider(u).host(), host, "{u}");
    }
}

#[test]
fn cleartext_rule_uses_the_parsed_host() {
    // A public host behind a numeric form is still public.
    assert!(provider("http://0x08080808/v1").validate().is_err());
    assert!(provider("http://8.8.8.8/v1").validate().is_err());
    assert!(provider("http://api.example.com/v1").validate().is_err());
    // Private / loopback / VPN literals may use http.
    for ok in [
        "http://0x7f.1/v1",
        "http://10.0.0.2:8000/v1",
        "http://[fd7a:115c:a1e0::1]:8000/v1",
    ] {
        assert_eq!(provider(ok).validate(), Ok(()), "{ok}");
    }
}

#[test]
fn consent_uses_the_parsed_host() {
    let mut p = provider("https://API.groq.com/openai/v1");
    p.consent_host = "api.groq.com".into();
    p.consent_at_ms = 1;
    assert!(p.consent_ok());
    p.base_url = "https://api.groq.com\\@evil.com/v1".into();
    assert!(!p.consent_ok(), "unparseable URL never has consent");
    p.base_url = "https://api.groq.com.evil.com/v1".into();
    assert!(!p.consent_ok());
}
