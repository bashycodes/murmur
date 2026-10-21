//! Server address classification (`docs/hybrid-architecture.md` §2.6).
//!
//! Plain `ws://` is allowed only to addresses that cannot be a third party
//! on the open internet: loopback, RFC 1918, Tailscale/CGNAT 100.64/10,
//! Tailscale IPv6 fd7a:115c:a1e0::/48, ULA fc00::/7, link-local,
//! `*.ts.net`, `*.local` and single-label names. Everything else is
//! public and needs `wss://`. Pure string/IP logic: no DNS, no sockets.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrClass {
    Loopback,
    /// RFC 1918, ULA, link-local.
    Lan,
    /// 100.64.0.0/10, fd7a:115c:a1e0::/48, `*.ts.net`.
    Tailnet,
    /// `*.local`, single-label names (resolved on the LAN).
    LocalName,
    Public,
}

impl AddrClass {
    /// May this address be reached over plain `ws://`?
    pub fn plaintext_ok(self) -> bool {
        self != AddrClass::Public
    }
}

/// Strip optional IPv6 brackets.
pub fn bare_host(host: &str) -> &str {
    let h = host.trim();
    h.strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .unwrap_or(h)
}

pub fn classify(host: &str) -> AddrClass {
    let h = bare_host(host);
    if let Ok(ip) = h.parse::<IpAddr>() {
        return classify_ip(ip);
    }
    let name = h.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        AddrClass::Loopback
    } else if name.ends_with(".ts.net") {
        AddrClass::Tailnet
    } else if name.ends_with(".local") || !name.contains('.') {
        AddrClass::LocalName
    } else {
        AddrClass::Public
    }
}

pub fn classify_ip(ip: IpAddr) -> AddrClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return classify_v4(v4);
            }
            classify_v6(v6)
        }
    }
}

fn classify_v4(ip: Ipv4Addr) -> AddrClass {
    let o = ip.octets();
    if ip.is_loopback() {
        AddrClass::Loopback
    } else if o[0] == 100 && (o[1] & 0xC0) == 64 {
        AddrClass::Tailnet
    } else if ip.is_private() || ip.is_link_local() {
        AddrClass::Lan
    } else {
        AddrClass::Public
    }
}

fn classify_v6(ip: Ipv6Addr) -> AddrClass {
    let s = ip.segments();
    if ip.is_loopback() {
        AddrClass::Loopback
    } else if s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0 {
        AddrClass::Tailnet
    } else if (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 {
        AddrClass::Lan
    } else {
        AddrClass::Public
    }
}

/// Validate a user-entered host (§5): IPv4 dotted quad (no leading zeros),
/// IPv6 literal (brackets optional) or hostname (labels 1-63 of
/// `[A-Za-z0-9-]`, no leading/trailing `-`, total <= 253). No scheme, path,
/// port or whitespace. Returns the normalised host (IPv6 without brackets).
pub fn validate_host(host: &str) -> Result<String, String> {
    let h = host.trim();
    if h.is_empty() {
        return Err("host is empty".into());
    }
    if h.chars().any(char::is_whitespace) {
        return Err("host must not contain whitespace".into());
    }
    if h.contains("://") || h.contains('/') {
        return Err("enter the host only (no scheme or path)".into());
    }
    let bare = bare_host(h);
    if bare.contains(':') {
        return bare
            .parse::<Ipv6Addr>()
            .map(|_| bare.to_string())
            .map_err(|_| "not a valid IPv6 address (and ports go in the port field)".into());
    }
    if bare != h {
        return Err("brackets are only for IPv6 addresses".into());
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        for p in &parts {
            if p.len() > 1 && p.starts_with('0') {
                return Err("IPv4 parts must not have leading zeros".into());
            }
            if p.len() > 3 || p.parse::<u16>().map_or(true, |n| n > 255) {
                return Err("IPv4 parts must be 0-255".into());
            }
        }
        return Ok(h.to_string());
    }
    if h.len() > 253 {
        return Err("hostname longer than 253 characters".into());
    }
    let name = h.strip_suffix('.').unwrap_or(h);
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err("hostname labels must be 1-63 characters".into());
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("hostnames may contain only letters, digits, '-' and '.'".into());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err("hostname labels must not start or end with '-'".into());
        }
    }
    if name
        .split('.')
        .all(|l| l.chars().all(|c| c.is_ascii_digit()))
    {
        return Err("not a valid IPv4 address".into());
    }
    Ok(h.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_table() {
        let t = [
            ("127.0.0.1", AddrClass::Loopback),
            ("::1", AddrClass::Loopback),
            ("[::1]", AddrClass::Loopback),
            ("localhost", AddrClass::Loopback),
            ("10.1.2.3", AddrClass::Lan),
            ("172.16.0.1", AddrClass::Lan),
            ("172.32.0.1", AddrClass::Public),
            ("192.168.1.20", AddrClass::Lan),
            ("169.254.1.1", AddrClass::Lan),
            ("100.64.0.10", AddrClass::Tailnet),
            ("100.127.255.255", AddrClass::Tailnet),
            ("100.128.0.1", AddrClass::Public),
            ("100.63.0.1", AddrClass::Public),
            ("fd7a:115c:a1e0::1", AddrClass::Tailnet),
            ("fd00::1", AddrClass::Lan),
            ("fe80::1", AddrClass::Lan),
            ("2001:4860:4860::8888", AddrClass::Public),
            ("::ffff:192.168.1.1", AddrClass::Lan),
            ("::ffff:8.8.8.8", AddrClass::Public),
            ("8.8.8.8", AddrClass::Public),
            ("box.tail1234.ts.net", AddrClass::Tailnet),
            ("desktop.local", AddrClass::LocalName),
            ("desktop", AddrClass::LocalName),
            ("example.com", AddrClass::Public),
            ("ts.net.example.com", AddrClass::Public),
        ];
        for (h, want) in t {
            assert_eq!(classify(h), want, "{h}");
        }
        assert!(!classify("8.8.8.8").plaintext_ok());
        assert!(classify("100.64.0.10").plaintext_ok());
    }

    #[test]
    fn host_validation() {
        for ok in [
            "192.168.1.2",
            "0.0.0.0",
            "desktop",
            "box.tail1.ts.net",
            "::1",
            "[fd7a:115c:a1e0::1]",
            "a-b.example.com",
        ] {
            assert!(validate_host(ok).is_ok(), "{ok}");
        }
        assert_eq!(validate_host(" [::1] ").unwrap(), "::1");
        for bad in [
            "",
            "ws://x",
            "x/y",
            "a b",
            "192.168.01.2",
            "256.1.1.1",
            "1.2.3",
            "host:7313",
            "-a.com",
            "a-.com",
            "a..b",
            "a_b.com",
            "[desktop]",
        ] {
            assert!(validate_host(bad).is_err(), "{bad}");
        }
        assert!(validate_host(&"a".repeat(64)).is_err());
        assert!(validate_host(&format!("{}.com", vec!["a".repeat(60); 5].join("."))).is_err());
    }
}
