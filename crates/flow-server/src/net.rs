//! Address classes for the bind policy and the peer filter (S10, D17:
//! "LAN/Tailscale only"). Mirrors the private classes the client accepts
//! for plain `ws://` (docs/protocol.md §1).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrClass {
    Loopback,
    /// RFC 1918, 100.64/10 (Tailscale / CGNAT), link-local, IPv6 ULA.
    Private,
    /// 0.0.0.0 / `::`: every interface, including public ones.
    Wildcard,
    Public,
}

impl AddrClass {
    /// Loopback or private: reachable only from the machine, the LAN or
    /// the tailnet.
    pub fn is_local(self) -> bool {
        matches!(self, AddrClass::Loopback | AddrClass::Private)
    }
}

fn v4(ip: Ipv4Addr) -> AddrClass {
    let o = ip.octets();
    if ip.is_loopback() {
        AddrClass::Loopback
    } else if ip.is_unspecified() {
        AddrClass::Wildcard
    } else if ip.is_private() || ip.is_link_local() || (o[0] == 100 && (o[1] & 0xc0) == 64) {
        AddrClass::Private
    } else {
        AddrClass::Public
    }
}

fn v6(ip: Ipv6Addr) -> AddrClass {
    if let Some(m) = ip.to_ipv4_mapped() {
        return v4(m);
    }
    let s = ip.segments()[0];
    if ip.is_loopback() {
        AddrClass::Loopback
    } else if ip.is_unspecified() {
        AddrClass::Wildcard
    } else if (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80 {
        AddrClass::Private
    } else {
        AddrClass::Public
    }
}

pub fn classify(ip: IpAddr) -> AddrClass {
    match ip {
        IpAddr::V4(a) => v4(a),
        IpAddr::V6(a) => v6(a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> AddrClass {
        classify(s.parse().unwrap())
    }

    #[test]
    fn classes() {
        use AddrClass::*;
        for (a, want) in [
            ("127.0.0.1", Loopback),
            ("127.8.9.1", Loopback),
            ("::1", Loopback),
            ("10.0.0.1", Private),
            ("172.16.0.1", Private),
            ("172.31.255.1", Private),
            ("192.168.0.9", Private),
            ("100.64.0.1", Private),
            ("100.127.255.254", Private),
            ("169.254.1.1", Private),
            ("fd7a:115c:a1e0::1", Private),
            ("fc00::1", Private),
            ("fe80::1", Private),
            ("::ffff:192.168.1.1", Private),
            ("0.0.0.0", Wildcard),
            ("::", Wildcard),
            ("8.8.8.8", Public),
            ("172.32.0.1", Public),
            ("100.128.0.1", Public),
            ("100.63.255.255", Public),
            ("2001:db8::1", Public),
            ("::ffff:8.8.8.8", Public),
        ] {
            assert_eq!(c(a), want, "{a}");
        }
    }
}
