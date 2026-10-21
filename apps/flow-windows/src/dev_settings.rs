//! Developer settings (hybrid-architecture §5): which engines a dictation
//! may use (local "client" / self-hosted "server"), where the server is,
//! and how patient to be with it.
//!
//! * [`DevSettings`] is what `prefs.json` stores under `"developer"`: the
//!   router config minus the token, which is kept only as a DPAPI blob
//!   (`token_dpapi`, [`crate::secrets`]).
//! * [`DevForm`] is what the Developer tab edits (`dev_get` / `dev_set`):
//!   the same fields with the token in clear and numbers wide enough that
//!   out-of-range input is a validation message, not a parse error.
//! * [`DevSettings::router_config_json`] produces flow-router's
//!   `RouterConfig` JSON (§1.2) — the router fixtures are the spec.
//!
//! Routes are an ordered list (`route_order`), not a two-way switch, so a
//! later route kind (e.g. an opt-in cloud provider) is one more variant and
//! one more list entry. The router's `prefer` is derived from the order.

use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

pub const DEFAULT_PORT: u16 = 7313;

/// What this build can do (Cargo features `local*` / `remote`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BuildCaps {
    pub local: bool,
    pub remote: bool,
}

impl BuildCaps {
    pub const fn current() -> Self {
        BuildCaps {
            local: cfg!(feature = "local-engine"),
            remote: cfg!(feature = "remote"),
        }
    }
}

/// One way to run a dictation. Order in `route_order` = preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Self-hosted flow-server (router `Remote`).
    Server,
    /// On-device flow-core (router `Local`).
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSettings {
    /// IPv4, IPv6 (stored without brackets) or hostname; "" = not configured.
    pub host: String,
    pub port: u16,
    pub tls: bool,
    /// SHA-256 of the server's leaf certificate, 64 lowercase hex.
    pub tls_pin_sha256: String,
    pub connect_timeout_ms: u64,
    pub commit_timeout_base_ms: u64,
    pub health_ttl_s: u64,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            host: String::new(),
            port: DEFAULT_PORT,
            tls: false,
            tls_pin_sha256: String::new(),
            connect_timeout_ms: 800,
            commit_timeout_base_ms: 3000,
            health_ttl_s: 30,
        }
    }
}

/// `prefs.json` → `"developer"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevSettings {
    pub allow_server: bool,
    pub allow_client: bool,
    /// Preference order; routes missing from the list go last.
    pub route_order: Vec<Route>,
    pub server: ServerSettings,
    pub keep_local_warm: bool,
    /// The server token, DPAPI-encrypted ("" = none). Never in clear on disk.
    pub token_dpapi: String,
    /// Client-only builds: the tab was unlocked (7 taps on the version).
    pub dev_unlocked: bool,
    /// Overlay pill shows S/L (+ red dot on fallback). UI only.
    pub show_engine_badge: bool,
}

impl Default for DevSettings {
    fn default() -> Self {
        Self::for_build(BuildCaps::current())
    }
}

impl DevSettings {
    /// §5 defaults: a toggle is on when the build has that engine.
    pub fn for_build(caps: BuildCaps) -> Self {
        DevSettings {
            allow_server: caps.remote,
            allow_client: caps.local,
            route_order: vec![Route::Server, Route::Client],
            server: ServerSettings::default(),
            keep_local_warm: true,
            token_dpapi: String::new(),
            dev_unlocked: false,
            show_engine_badge: false,
        }
    }

    /// Full preference order: the stored list, then any route it misses.
    pub fn order(&self) -> Vec<Route> {
        let mut v: Vec<Route> = Vec::new();
        for r in self
            .route_order
            .iter()
            .chain([Route::Server, Route::Client].iter())
        {
            if !v.contains(r) {
                v.push(*r);
            }
        }
        v
    }

    /// Router `prefer` (server | client): the first of the two in the order.
    pub fn prefer(&self) -> Route {
        self.order()
            .into_iter()
            .find(|r| matches!(r, Route::Server | Route::Client))
            .unwrap_or(Route::Server)
    }

    /// Effective toggles = build capability AND user toggle (§1.3).
    pub fn effective(&self, caps: BuildCaps) -> (bool, bool) {
        (
            caps.remote && self.allow_server,
            caps.local && self.allow_client,
        )
    }

    /// Developer tab visibility (§5 "Access").
    pub fn tab_visible(&self, caps: BuildCaps) -> bool {
        caps.remote || self.dev_unlocked
    }

    /// The token in clear ("" when none or unreadable on this account).
    pub fn token(&self) -> String {
        crate::secrets::unprotect(&self.token_dpapi).unwrap_or_default()
    }

    /// flow-router `RouterConfig` JSON (§1.2). `token` is passed in so the
    /// clear token only exists in memory.
    pub fn router_config_json(&self, caps: BuildCaps, token: &str) -> serde_json::Value {
        let (srv, cli) = self.effective(caps);
        let s = &self.server;
        serde_json::json!({
            "allow_server": srv,
            "allow_client": cli,
            "prefer": match self.prefer() { Route::Server => "server", Route::Client => "client" },
            "server": {
                "host": s.host,
                "port": s.port,
                "tls": s.tls || (!s.host.is_empty() && host_is_public(&s.host)),
                "tls_pin_sha256": if s.tls_pin_sha256.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(s.tls_pin_sha256.clone())
                },
                "token": token,
                "connect_timeout_ms": s.connect_timeout_ms,
                "commit_timeout_base_ms": s.commit_timeout_base_ms,
                "health_ttl_s": s.health_ttl_s,
            },
            "keep_local_warm": self.keep_local_warm,
        })
    }

    /// Warnings shown on the Developer page (valid but probably not meant).
    pub fn warnings(&self, caps: BuildCaps) -> Vec<String> {
        let (srv, cli) = self.effective(caps);
        let mut w = Vec::new();
        if !srv && !cli {
            w.push(NO_ENGINE_MSG.to_string());
        }
        if srv && self.server.host.is_empty() {
            w.push(if cli {
                "Server host is empty: dictation will use this PC.".to_string()
            } else {
                "Server host is empty: dictation will fail until a server is set.".to_string()
            });
        }
        if srv && !self.server.host.is_empty() && !self.server.tls {
            if host_is_public(&self.server.host) {
                w.push("Public address: TLS (wss) is required and turned on.".into());
            } else {
                w.push("Unencrypted: OK on Tailscale (WireGuard), LAN only otherwise.".into());
            }
        }
        if srv && self.token_dpapi.is_empty() {
            w.push("No token set: a server with auth enabled will reject this PC.".into());
        }
        w
    }
}

/// Normal-UI error when both engines are off (§5).
pub const NO_ENGINE_MSG: &str =
    "Dictation disabled — enable server or client in Developer settings";

/// What the Developer tab edits. Numbers are i64 so "0" or "70000" reach
/// validation instead of failing deserialisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevForm {
    pub allow_server: bool,
    pub allow_client: bool,
    pub route_order: Vec<Route>,
    pub host: String,
    pub port: i64,
    pub tls: bool,
    pub tls_pin_sha256: String,
    pub token: String,
    pub connect_timeout_ms: i64,
    pub commit_timeout_base_ms: i64,
    pub health_ttl_s: i64,
    pub keep_local_warm: bool,
    pub show_engine_badge: bool,
    pub dev_unlocked: bool,
}

impl Default for DevForm {
    fn default() -> Self {
        DevForm::from_settings(&DevSettings::default(), "")
    }
}

/// A validation failure for one form field (UI shows it inline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

fn err(field: &'static str, message: impl Into<String>) -> FieldError {
    FieldError {
        field,
        message: message.into(),
    }
}

impl DevForm {
    pub fn from_settings(s: &DevSettings, token: &str) -> Self {
        DevForm {
            allow_server: s.allow_server,
            allow_client: s.allow_client,
            route_order: s.order(),
            host: s.server.host.clone(),
            port: s.server.port.into(),
            tls: s.server.tls,
            tls_pin_sha256: s.server.tls_pin_sha256.clone(),
            token: token.to_string(),
            connect_timeout_ms: s.server.connect_timeout_ms as i64,
            commit_timeout_base_ms: s.server.commit_timeout_base_ms as i64,
            health_ttl_s: s.server.health_ttl_s as i64,
            keep_local_warm: s.keep_local_warm,
            show_engine_badge: s.show_engine_badge,
            dev_unlocked: s.dev_unlocked,
        }
    }

    /// Validate and normalise (§5 table). `Ok` = settings to store (token
    /// still in clear: the caller protects it) — never partially applied.
    pub fn validate(&self, caps: BuildCaps) -> Result<(DevSettings, String), Vec<FieldError>> {
        let mut errs = Vec::new();
        let host = match normalize_host(&self.host) {
            Ok(h) => h,
            Err(m) => {
                errs.push(err("host", m));
                String::new()
            }
        };
        let port = match u16::try_from(self.port) {
            Ok(p) if p >= 1 => p,
            _ => {
                errs.push(err("port", "Port must be a whole number from 1 to 65535"));
                DEFAULT_PORT
            }
        };
        let pin: String = self
            .tls_pin_sha256
            .trim()
            .chars()
            .filter(|c| *c != ':')
            .collect::<String>()
            .to_ascii_lowercase();
        if !pin.is_empty() && (pin.len() != 64 || !pin.chars().all(|c| c.is_ascii_hexdigit())) {
            errs.push(err(
                "tls_pin_sha256",
                "Certificate pin must be 64 hex characters (SHA-256; colons allowed)",
            ));
        }
        let token = self.token.trim().to_string();
        if !token.is_empty() && token.chars().count() < 16 {
            errs.push(err(
                "token",
                "Token must be at least 16 characters (or empty)",
            ));
        }
        if token.chars().any(char::is_whitespace) {
            errs.push(err("token", "Token must not contain spaces"));
        }
        let mut range = |field: &'static str, v: i64, lo: i64, hi: i64, what: &str| -> u64 {
            if (lo..=hi).contains(&v) {
                v as u64
            } else {
                errs.push(err(field, format!("{what} must be from {lo} to {hi}")));
                lo as u64
            }
        };
        let connect = range(
            "connect_timeout_ms",
            self.connect_timeout_ms,
            100,
            10_000,
            "Connect timeout (ms)",
        );
        let commit = range(
            "commit_timeout_base_ms",
            self.commit_timeout_base_ms,
            500,
            60_000,
            "Commit timeout (ms)",
        );
        let ttl = range(
            "health_ttl_s",
            self.health_ttl_s,
            5,
            600,
            "Health cache (s)",
        );
        if !errs.is_empty() {
            return Err(errs);
        }
        // A toggle the build cannot serve stays off (and disabled in the UI).
        let tls = self.tls || (!host.is_empty() && host_is_public(&host));
        let mut order: Vec<Route> = Vec::new();
        for r in &self.route_order {
            if !order.contains(r) {
                order.push(*r);
            }
        }
        let s = DevSettings {
            allow_server: self.allow_server && caps.remote,
            allow_client: self.allow_client && caps.local,
            route_order: order,
            server: ServerSettings {
                host,
                port,
                tls,
                tls_pin_sha256: pin,
                connect_timeout_ms: connect,
                commit_timeout_base_ms: commit,
                health_ttl_s: ttl,
            },
            keep_local_warm: self.keep_local_warm,
            token_dpapi: String::new(),
            dev_unlocked: self.dev_unlocked,
            show_engine_badge: self.show_engine_badge,
        };
        let order = s.order();
        Ok((
            DevSettings {
                route_order: order,
                ..s
            },
            token,
        ))
    }
}

/// §5 host rule: trim; no scheme/path/port/whitespace; IPv4 dotted quad
/// (no leading zeros), IPv6 literal (brackets optional, stored without) or
/// an RFC 1123 hostname. `""` = not configured.
pub fn normalize_host(raw: &str) -> Result<String, String> {
    let h = raw.trim();
    if h.is_empty() {
        return Ok(String::new());
    }
    if h.chars().any(char::is_whitespace) {
        return Err("Host must not contain spaces".into());
    }
    if h.contains("://") {
        return Err("Enter only the host, without ws:// or http://".into());
    }
    if h.contains('/') || h.contains('?') || h.contains('#') {
        return Err("Enter only the host, without a path".into());
    }
    let unbracketed = h
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .unwrap_or(h);
    if unbracketed.contains(':') {
        return match unbracketed.parse::<Ipv6Addr>() {
            Ok(_) if !unbracketed.contains('%') => Ok(unbracketed.to_ascii_lowercase()),
            _ => Err(
                "Not a valid address (put the port in the Port field; IPv6 like fd7a:115c:a1e0::1)"
                    .into(),
            ),
        };
    }
    if h.starts_with('[') || h.ends_with(']') {
        return Err("Brackets are only for IPv6 addresses".into());
    }
    if h.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return parse_ipv4_strict(h)
            .map(|ip| ip.to_string())
            .ok_or_else(|| {
                "Not a valid IPv4 address (four numbers 0-255, e.g. 100.64.0.10)".into()
            });
    }
    let name = h.strip_suffix('.').unwrap_or(h);
    if name.len() > 253 {
        return Err("Host name is too long (max 253 characters)".into());
    }
    for label in name.split('.') {
        let ok = (1..=63).contains(&label.len())
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !ok {
            return Err(format!(
                "Not a valid host name (label \"{label}\": letters, digits and '-', 1-63 chars, \
                 not starting or ending with '-')"
            ));
        }
    }
    Ok(name.to_ascii_lowercase())
}

/// Dotted quad, exactly 4 parts 0-255, no leading zeros ("010" is octal
/// to some resolvers and decimal to others).
fn parse_ipv4_strict(s: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut o = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 3 || (p.len() > 1 && p.starts_with('0')) {
            return None;
        }
        o[i] = p.parse().ok()?;
    }
    Some(Ipv4Addr::from(o))
}

/// §2.6: is plaintext `ws://` refused for this (normalised) host? Private
/// = loopback, RFC 1918, 100.64.0.0/10 (Tailscale/CGNAT), fc00::/7 (incl.
/// Tailscale fd7a:115c:a1e0::/48), link-local, `*.ts.net`, `*.local`,
/// `localhost` and single-label names. Everything else is public.
pub fn host_is_public(host: &str) -> bool {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        let private = ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || (a == 100 && (64..128).contains(&b));
        return !private;
    }
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
        let seg0 = ip.segments()[0];
        let private = ip.is_loopback()
            || (seg0 & 0xfe00) == 0xfc00
            || (seg0 & 0xffc0) == 0xfe80
            || ip
                .to_ipv4_mapped()
                .is_some_and(|v4| !host_is_public(&v4.to_string()));
        return !private;
    }
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    let private = !h.contains('.')
        || h == "localhost"
        || h.ends_with(".ts.net")
        || h.ends_with(".local")
        || h.ends_with(".localhost");
    !private
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: BuildCaps = BuildCaps {
        local: true,
        remote: true,
    };

    fn form() -> DevForm {
        DevForm::from_settings(&DevSettings::for_build(BOTH), "")
    }

    #[test]
    fn defaults_follow_the_build() {
        let h = DevSettings::for_build(BOTH);
        assert!(h.allow_server && h.allow_client && h.keep_local_warm);
        assert_eq!(h.prefer(), Route::Server);
        let local = DevSettings::for_build(BuildCaps {
            local: true,
            remote: false,
        });
        assert!(!local.allow_server && local.allow_client);
        let remote = DevSettings::for_build(BuildCaps {
            local: false,
            remote: true,
        });
        assert!(remote.allow_server && !remote.allow_client);
        assert!(remote.tab_visible(BuildCaps {
            local: false,
            remote: true
        }));
        assert!(!local.tab_visible(BuildCaps {
            local: true,
            remote: false
        }));
        assert!(DevSettings {
            dev_unlocked: true,
            ..local
        }
        .tab_visible(BuildCaps {
            local: true,
            remote: false
        }));
    }

    #[test]
    fn hosts_accepted_and_normalised() {
        for (raw, want) in [
            ("  100.64.0.10 ", "100.64.0.10"),
            ("192.168.1.2", "192.168.1.2"),
            ("[fd7a:115c:a1e0::1]", "fd7a:115c:a1e0::1"),
            ("FD7A:115C:A1E0::1", "fd7a:115c:a1e0::1"),
            ("::1", "::1"),
            ("box.tailnet-abc.ts.net", "box.tailnet-abc.ts.net"),
            ("MyBox", "mybox"),
            ("my-box.local.", "my-box.local"),
            ("", ""),
        ] {
            assert_eq!(normalize_host(raw).as_deref(), Ok(want), "{raw}");
        }
    }

    #[test]
    fn hosts_rejected() {
        for bad in [
            "ws://box",
            "http://1.2.3.4",
            "box/path",
            "box:7313",
            "1.2.3.4:7313",
            "[::1]:7313",
            "1.2.3",
            "1.2.3.4.5",
            "256.1.1.1",
            "01.2.3.4",
            "1..2.3",
            "my box",
            "-box",
            "box-",
            "bo_x",
            "[box]",
            "fe80::1%eth0",
            &"a".repeat(64),
            &format!("{}.com", vec!["a".repeat(60); 5].join(".")),
        ] {
            assert!(normalize_host(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn public_vs_private_classification() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.5",
            "100.64.0.1",
            "100.127.255.254",
            "169.254.1.1",
            "::1",
            "fd7a:115c:a1e0::5",
            "fc00::1",
            "fe80::1",
            "box",
            "localhost",
            "pc.tail1234.ts.net",
            "nas.local",
        ] {
            assert!(!host_is_public(private), "{private}");
        }
        for public in [
            "8.8.8.8",
            "100.128.0.1",
            "172.32.0.1",
            "2001:4860::8888",
            "example.com",
            "box.example.org",
        ] {
            assert!(host_is_public(public), "{public}");
        }
    }

    #[test]
    fn numeric_fields_validated() {
        let mut f = form();
        f.port = 0;
        f.connect_timeout_ms = 50;
        f.commit_timeout_base_ms = 70_000;
        f.health_ttl_s = 601;
        let e = f.validate(BOTH).unwrap_err();
        let fields: Vec<_> = e.iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
            vec![
                "port",
                "connect_timeout_ms",
                "commit_timeout_base_ms",
                "health_ttl_s"
            ]
        );
        f = form();
        f.port = 65_536;
        assert_eq!(f.validate(BOTH).unwrap_err()[0].field, "port");
        f.port = 65_535;
        f.connect_timeout_ms = 10_000;
        f.commit_timeout_base_ms = 500;
        f.health_ttl_s = 5;
        assert!(f.validate(BOTH).is_ok());
    }

    #[test]
    fn token_and_pin_rules() {
        let mut f = form();
        f.token = "short".into();
        assert_eq!(f.validate(BOTH).unwrap_err()[0].field, "token");
        f.token = "  0123456789abcdef  ".into();
        assert_eq!(f.validate(BOTH).unwrap().1, "0123456789abcdef", "trimmed");
        f.tls_pin_sha256 = "AB:".repeat(32);
        let (s, _) = f.validate(BOTH).unwrap();
        assert_eq!(s.server.tls_pin_sha256, "ab".repeat(32));
        f.tls_pin_sha256 = "zz".repeat(32);
        assert_eq!(f.validate(BOTH).unwrap_err()[0].field, "tls_pin_sha256");
    }

    #[test]
    fn public_host_forces_tls() {
        let mut f = form();
        f.host = "example.com".into();
        let (s, _) = f.validate(BOTH).unwrap();
        assert!(s.server.tls);
        f.host = "100.64.0.10".into();
        assert!(!f.validate(BOTH).unwrap().0.server.tls);
    }

    #[test]
    fn toggles_the_build_cannot_serve_stay_off() {
        let f = form();
        let (s, _) = f
            .validate(BuildCaps {
                local: true,
                remote: false,
            })
            .unwrap();
        assert!(!s.allow_server && s.allow_client);
    }

    #[test]
    fn route_order_is_a_deduplicated_complete_list() {
        let mut f = form();
        f.route_order = vec![Route::Client, Route::Client];
        let (s, _) = f.validate(BOTH).unwrap();
        assert_eq!(s.route_order, vec![Route::Client, Route::Server]);
        assert_eq!(s.prefer(), Route::Client);
        let d: DevSettings = serde_json::from_str(r#"{"route_order":[]}"#).unwrap();
        assert_eq!(d.order(), vec![Route::Server, Route::Client]);
    }

    #[test]
    fn router_config_json_shape() {
        let mut f = form();
        f.host = "100.64.0.10".into();
        f.route_order = vec![Route::Client, Route::Server];
        let (s, token) = {
            f.token = "0123456789abcdef0123".into();
            f.validate(BOTH).unwrap()
        };
        let j = s.router_config_json(BOTH, &token);
        assert_eq!(
            j,
            serde_json::json!({
                "allow_server": true, "allow_client": true, "prefer": "client",
                "server": {"host": "100.64.0.10", "port": 7313, "tls": false,
                           "tls_pin_sha256": null, "token": "0123456789abcdef0123",
                           "connect_timeout_ms": 800, "commit_timeout_base_ms": 3000,
                           "health_ttl_s": 30},
                "keep_local_warm": true
            })
        );
        let j = s.router_config_json(
            BuildCaps {
                local: true,
                remote: false,
            },
            "",
        );
        assert_eq!(j["allow_server"], false, "capability AND toggle");
    }

    #[test]
    fn prefs_json_never_holds_the_clear_token() {
        let mut f = form();
        f.token = "super-secret-token-123".into();
        let (mut s, token) = f.validate(BOTH).unwrap();
        s.token_dpapi = crate::secrets::protect(&token).unwrap();
        let on_disk = serde_json::to_string(&s).unwrap();
        assert!(!on_disk.contains("super-secret-token-123"), "{on_disk}");
        assert!(!on_disk.contains("\"token\""), "{on_disk}");
        assert_eq!(s.token(), "super-secret-token-123");
        assert_eq!(DevForm::from_settings(&s, &s.token()).token, token);
    }

    #[test]
    fn warnings() {
        let s = DevSettings {
            allow_server: false,
            allow_client: false,
            ..DevSettings::for_build(BOTH)
        };
        assert_eq!(s.warnings(BOTH), vec![NO_ENGINE_MSG.to_string()]);
        let s = DevSettings::for_build(BOTH);
        assert!(s.warnings(BOTH)[0].contains("host is empty"));
    }
}
