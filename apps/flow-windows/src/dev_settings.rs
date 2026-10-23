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
//! Routes are an ordered list (`route_order`), not a two-way switch; the
//! opt-in cloud provider (D18, `docs/cloud-stt.md`) is the third entry. The
//! router's `prefer` is derived from the order; with the cloud allowed the
//! full list goes to the router as `order`.
//!
//! Cloud providers (`cloud`) are flow-router's `CloudConfig` as is: no API
//! keys (those live in Credential Manager, [`crate::credentials`]); consent
//! is recorded per provider for the host it was given for, and
//! [`reconcile_consent`] voids it when the host or the adapter changes.

use flow_core::config::ParakeetDecoding;
use flow_router::{CloudConfig, CloudProviderConfig};
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

pub const DEFAULT_PORT: u16 = 7313;

/// What this build can do (Cargo features `local*` / `remote` / `cloud`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BuildCaps {
    pub local: bool,
    pub remote: bool,
    /// Third-party cloud STT compiled in (feature `cloud`, D18 C2).
    pub cloud: bool,
}

impl BuildCaps {
    pub const fn current() -> Self {
        BuildCaps {
            local: cfg!(feature = "local-engine"),
            remote: cfg!(feature = "remote"),
            cloud: cfg!(feature = "cloud"),
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
    /// Opt-in third-party provider (router `Cloud`, D18).
    Cloud,
}

impl Route {
    pub const ALL: [Route; 3] = [Route::Server, Route::Client, Route::Cloud];

    pub fn as_str(self) -> &'static str {
        match self {
            Route::Server => "server",
            Route::Client => "client",
            Route::Cloud => "cloud",
        }
    }
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
            connect_timeout_ms: flow_router::DEFAULT_CONNECT_TIMEOUT_MS,
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
    /// Overlay pill shows S/L (+ red dot on fallback). UI only. The cloud
    /// badge `C` shows regardless (C7).
    pub show_engine_badge: bool,
    /// Opt-in third-party cloud STT (D18 C1). Off by default.
    pub allow_cloud: bool,
    /// Cloud providers, without keys (router `RouterConfig.cloud`).
    pub cloud: CloudConfig,
    /// Opt-in debug log with transcripts ([`crate::debug_log`]). Off by
    /// default. Not part of [`DevForm`]: its own toggle saves it at once,
    /// and a form save keeps the stored value.
    pub debug_log: bool,
    /// D22 "Decode while speaking": sessions carry `segment_pause_ms: 800`
    /// (finished phrases are decoded while the user keeps talking). On by
    /// default; per device (prefs.json), never synced.
    pub decode_while_speaking: bool,
    /// Parakeet search (`ModelConfig.parakeet_decoding`): `auto` (default:
    /// beam with hotwords + a greedy recognizer, ≈ +0.6 GB RSS, which a
    /// desktop can spare) or `greedy` (one recognizer, no hotwords).
    /// Whisper ignores it. Changing it reloads the model.
    pub parakeet_decoding: ParakeetDecoding,
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
            allow_cloud: false,
            cloud: CloudConfig::default(),
            debug_log: false,
            decode_while_speaking: true,
            parakeet_decoding: ParakeetDecoding::Auto,
        }
    }

    /// Full preference order: the stored list, then any route it misses.
    pub fn order(&self) -> Vec<Route> {
        let mut v: Vec<Route> = Vec::new();
        for r in self.route_order.iter().chain(Route::ALL.iter()) {
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

    /// The cloud route may be tried (build AND toggle); consent, key and
    /// budget are checked by the router.
    pub fn cloud_effective(&self, caps: BuildCaps) -> bool {
        caps.cloud && self.allow_cloud
    }

    /// The provider the cloud route uses, if any.
    pub fn active_provider(&self) -> Option<&CloudProviderConfig> {
        self.cloud.active_provider()
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
        let mut j = serde_json::json!({
            "allow_server": srv,
            "allow_client": cli,
            "prefer": match self.prefer() { Route::Client => "client", _ => "server" },
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
        });
        // D18: cloud fields only in builds with the cloud client, and (like
        // the router's own serialisation) only while not default, so the
        // D17 fixtures stay byte-identical.
        if caps.cloud {
            if self.allow_cloud {
                j["allow_cloud"] = true.into();
                j["order"] = self
                    .order()
                    .iter()
                    .map(|r| serde_json::Value::from(r.as_str()))
                    .collect();
            }
            if !self.cloud.is_default() {
                j["cloud"] = serde_json::to_value(&self.cloud).expect("CloudConfig serialises");
            }
        }
        j
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

    /// Cloud warnings (§3.1 / §8.3): where audio may go, and what is still
    /// missing before the cloud route can run. `has_key(id)` asks the
    /// credential store.
    pub fn cloud_warnings(&self, caps: BuildCaps, has_key: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut w = Vec::new();
        if !self.cloud_effective(caps) {
            return w;
        }
        let Some(p) = self.active_provider() else {
            w.push("Allow cloud is on but no provider is active: add one below.".into());
            return w;
        };
        let name = format!("{} ({})", p.display_name(), p.host());
        if !p.consent_ok() {
            w.push(format!(
                "Cloud is not used until you accept sending audio to {name}."
            ));
        }
        if !has_key(&p.id) && !keyless_ok(p) {
            w.push(format!(
                "Cloud is not used until an API key for {} is saved.",
                p.display_name()
            ));
        }
        let (srv, cli) = self.effective(caps);
        let allowed = |r: &Route| match r {
            Route::Server => srv,
            Route::Client => cli,
            Route::Cloud => true,
        };
        let order: Vec<Route> = self.order().into_iter().filter(allowed).collect();
        if let Some(i) = order.iter().position(|r| *r == Route::Cloud) {
            match order[..i].last() {
                Some(Route::Server) => w.push(format!(
                    "When your server is unreachable, audio goes to {name}."
                )),
                Some(Route::Client) => w.push(format!(
                    "When no local model is loaded, audio goes to {name}."
                )),
                _ => {}
            }
        }
        w
    }
}

/// A `custom` openai_compat provider on a loopback / LAN / Tailscale host
/// runs without an API key (router, review P3 follow-up): no "key
/// required" warning for it. Same rule as `flow_router`'s `keyless_ok`.
pub fn keyless_ok(p: &CloudProviderConfig) -> bool {
    let host = p.host();
    p.preset == "custom"
        && p.adapter == "openai_compat"
        && !host.is_empty()
        && flow_router::net::classify(&host).plaintext_ok()
}

/// Trim the free-text fields of every provider (validation runs after).
pub fn normalize_cloud(c: &CloudConfig) -> CloudConfig {
    let mut c = c.clone();
    for p in &mut c.providers {
        for f in [
            &mut p.id,
            &mut p.label,
            &mut p.base_url,
            &mut p.model,
            &mut p.language,
            &mut p.consent_host,
        ] {
            *f = f.trim().to_string();
        }
        p.base_url = p.base_url.trim_end_matches('/').to_string();
    }
    c
}

/// Consent rules on Save (§8.1, §8.3). `saved` is what is on disk, `new`
/// the form being saved:
///
/// * a consent newer than the saved one (the dialog was just accepted) is
///   kept only if it names the provider's current host;
/// * otherwise the saved consent carries over (the form cannot forge one),
///   unless the provider's host or adapter changed: then it is cleared and
///   the dialog asks again.
pub fn reconcile_consent(saved: &CloudConfig, new: &mut CloudConfig) {
    for p in &mut new.providers {
        let old = saved.providers.iter().find(|o| o.id == p.id);
        let fresh = p.consent_at_ms > old.map_or(0, |o| o.consent_at_ms);
        let keep = if fresh {
            !p.consent_host.is_empty() && p.consent_host.eq_ignore_ascii_case(&p.host())
        } else {
            match old {
                Some(o) if o.adapter == p.adapter && o.host() == p.host() => {
                    p.consent_host = o.consent_host.clone();
                    p.consent_at_ms = o.consent_at_ms;
                    o.consent_ok()
                }
                _ => false,
            }
        };
        if !keep {
            p.consent_host.clear();
            p.consent_at_ms = 0;
        }
    }
}

/// Providers present in `saved` but gone from `new` (their keys are
/// deleted from Credential Manager).
pub fn removed_providers(saved: &CloudConfig, new: &CloudConfig) -> Vec<String> {
    saved
        .providers
        .iter()
        .filter(|o| !new.providers.iter().any(|p| p.id == o.id))
        .map(|o| o.id.clone())
        .collect()
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
    /// A token is saved but not sent to the page (review S18): an empty
    /// `token` with this set means "keep the saved one".
    pub token_saved: bool,
    pub connect_timeout_ms: i64,
    pub commit_timeout_base_ms: i64,
    pub health_ttl_s: i64,
    pub keep_local_warm: bool,
    pub show_engine_badge: bool,
    pub dev_unlocked: bool,
    pub allow_cloud: bool,
    /// Providers as edited; consent fields are written by the consent
    /// dialog and checked by [`reconcile_consent`] on save. No keys.
    pub cloud: CloudConfig,
    pub decode_while_speaking: bool,
    pub parakeet_decoding: ParakeetDecoding,
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
            token_saved: false,
            connect_timeout_ms: s.server.connect_timeout_ms as i64,
            commit_timeout_base_ms: s.server.commit_timeout_base_ms as i64,
            health_ttl_s: s.server.health_ttl_s as i64,
            keep_local_warm: s.keep_local_warm,
            show_engine_badge: s.show_engine_badge,
            dev_unlocked: s.dev_unlocked,
            allow_cloud: s.allow_cloud,
            cloud: s.cloud.clone(),
            decode_while_speaking: s.decode_while_speaking,
            parakeet_decoding: s.parakeet_decoding,
        }
    }

    /// What the page gets: the saved settings with the token withheld
    /// (only "a token is saved"); the webview never sees it (S18).
    pub fn for_page(s: &DevSettings) -> Self {
        DevForm {
            token_saved: !s.token_dpapi.is_empty(),
            ..DevForm::from_settings(s, "")
        }
    }

    /// The form with the token resolved: an empty field with
    /// `token_saved` keeps `saved`'s token; anything typed replaces it.
    pub fn with_token(&self, saved: &DevSettings) -> DevForm {
        let mut f = self.clone();
        if f.token.trim().is_empty() && f.token_saved {
            f.token = saved.token();
        }
        f.token_saved = false;
        f
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
        let cloud = normalize_cloud(&self.cloud);
        if let Err(m) = cloud.validate() {
            errs.push(err("cloud", m));
        }
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
            allow_cloud: self.allow_cloud && caps.cloud,
            cloud,
            debug_log: false,
            decode_while_speaking: self.decode_while_speaking,
            parakeet_decoding: self.parakeet_decoding,
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

/// §2.6: is plaintext `ws://` refused for this (normalised) host at
/// config time? The router's classifier decides (`flow_router::net`, the
/// same one `ServerConfig::validate` uses), so the form can never disagree
/// with the gate. Private = loopback, RFC 1918, link-local, ULA, tailnet
/// (100.64/10, fd7a:115c:a1e0::/48), `*.ts.net`, `*.local`, `localhost`
/// and single-label names. That is only the form rule: at connect time
/// flow-remote resolves a name and requires every address to be private
/// (review S6), so a `*.ts.net` or LAN name that resolves publicly is
/// refused then ("server needs TLS").
pub fn host_is_public(host: &str) -> bool {
    !flow_router::net::classify(host).plaintext_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: BuildCaps = BuildCaps {
        local: true,
        remote: true,
        cloud: true,
    };

    fn form() -> DevForm {
        DevForm::from_settings(&DevSettings::for_build(BOTH), "")
    }

    #[test]
    fn debug_log_is_off_by_default_and_in_old_files() {
        assert!(!DevSettings::for_build(BOTH).debug_log);
        let old: DevSettings = serde_json::from_str(r#"{"allow_server":true}"#).unwrap();
        assert!(!old.debug_log);
        let on: DevSettings = serde_json::from_str(r#"{"debug_log":true}"#).unwrap();
        assert!(on.debug_log);
        // The Developer form does not carry it (it has its own command):
        // the app keeps the stored value on a form save.
        let (s, _) = form().validate(BOTH).unwrap();
        assert!(!s.debug_log);
    }

    #[test]
    fn decode_while_speaking_is_on_by_default_and_saved_by_the_form() {
        assert!(DevSettings::for_build(BOTH).decode_while_speaking);
        let old: DevSettings = serde_json::from_str(r#"{"allow_server":true}"#).unwrap();
        assert!(old.decode_while_speaking, "pre-D22 file: on");
        let mut f = form();
        assert!(f.decode_while_speaking);
        f.decode_while_speaking = false;
        let (s, _) = f.validate(BOTH).unwrap();
        assert!(!s.decode_while_speaking);
        assert!(!DevForm::for_page(&s).decode_while_speaking);
        // A page from before D22 posts no field: on.
        let f: DevForm = serde_json::from_str(r#"{"allow_client":true}"#).unwrap();
        assert!(f.decode_while_speaking);
    }

    #[test]
    fn parakeet_decoding_defaults_to_auto_and_is_saved_by_the_form() {
        assert_eq!(
            DevSettings::for_build(BOTH).parakeet_decoding,
            ParakeetDecoding::Auto
        );
        let old: DevSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.parakeet_decoding, ParakeetDecoding::Auto);
        let f: DevForm = serde_json::from_str(r#"{"parakeet_decoding":"greedy"}"#).unwrap();
        let (s, _) = f.validate(BOTH).unwrap();
        assert_eq!(s.parakeet_decoding, ParakeetDecoding::Greedy);
        let j = serde_json::to_value(DevForm::for_page(&s)).unwrap();
        assert_eq!(j["parakeet_decoding"], "greedy");
    }

    #[test]
    fn defaults_follow_the_build() {
        let h = DevSettings::for_build(BOTH);
        assert!(h.allow_server && h.allow_client && h.keep_local_warm);
        assert_eq!(h.prefer(), Route::Server);
        let local = DevSettings::for_build(BuildCaps {
            local: true,
            remote: false,
            cloud: false,
        });
        assert!(!local.allow_server && local.allow_client);
        let remote = DevSettings::for_build(BuildCaps {
            local: false,
            remote: true,
            cloud: false,
        });
        assert!(remote.allow_server && !remote.allow_client);
        assert!(remote.tab_visible(BuildCaps {
            local: false,
            remote: true,
            cloud: false
        }));
        assert!(!local.tab_visible(BuildCaps {
            local: true,
            remote: false,
            cloud: false
        }));
        assert!(DevSettings {
            dev_unlocked: true,
            ..local
        }
        .tab_visible(BuildCaps {
            local: true,
            remote: false,
            cloud: false
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
    fn cloud_base_urls_are_parsed_by_the_router() {
        // review S2: the `url`-crate parser, not a hand-rolled split.
        for bad in [
            "http://evil.com\\.local/v1",
            "https://user@api.groq.com/openai/v1",
            "https://api.groq.com /v1",
            "ftp://api.groq.com/v1",
            "https://",
        ] {
            let mut f = form();
            f.cloud.providers = vec![groq(0)];
            f.cloud.providers[0].base_url = bad.into();
            assert!(f.validate(BOTH).is_err(), "{bad:?} must be refused");
        }
        let mut f = form();
        f.cloud.providers = vec![groq(0)];
        f.cloud.providers[0].base_url = "https://API.Groq.com/openai/v1".into();
        let (s, _) = f.validate(BOTH).unwrap();
        assert_eq!(s.cloud.providers[0].host(), "api.groq.com");
    }

    #[test]
    fn keyless_only_for_a_custom_server_on_a_private_host() {
        let mut p = groq(0);
        p.preset = "custom".into();
        p.base_url = "http://192.168.1.20:8000/v1".into();
        assert!(keyless_ok(&p));
        p.base_url = "http://speaches.tail1234.ts.net:8000/v1".into();
        assert!(keyless_ok(&p));
        p.base_url = "https://stt.example.com/v1".into();
        assert!(!keyless_ok(&p), "public host");
        let mut named = groq(0);
        named.base_url = "http://192.168.1.20:8000/v1".into();
        assert!(!keyless_ok(&named), "a named preset always needs a key");
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
                cloud: false,
            })
            .unwrap();
        assert!(!s.allow_server && s.allow_client);
    }

    #[test]
    fn route_order_is_a_deduplicated_complete_list() {
        let mut f = form();
        f.route_order = vec![Route::Client, Route::Client];
        let (s, _) = f.validate(BOTH).unwrap();
        assert_eq!(
            s.route_order,
            vec![Route::Client, Route::Server, Route::Cloud]
        );
        assert_eq!(s.prefer(), Route::Client);
        let d: DevSettings = serde_json::from_str(r#"{"route_order":[]}"#).unwrap();
        assert_eq!(d.order(), vec![Route::Server, Route::Client, Route::Cloud]);
        let d: DevSettings = serde_json::from_str(r#"{"route_order":["cloud"]}"#).unwrap();
        assert_eq!(d.order(), vec![Route::Cloud, Route::Server, Route::Client]);
        assert_eq!(d.prefer(), Route::Server, "prefer ignores the cloud");
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
                           "connect_timeout_ms": 2500, "commit_timeout_base_ms": 3000,
                           "health_ttl_s": 30},
                "keep_local_warm": true
            })
        );
        let j = s.router_config_json(
            BuildCaps {
                local: true,
                remote: false,
                cloud: false,
            },
            "",
        );
        assert_eq!(j["allow_server"], false, "capability AND toggle");
    }

    #[test]
    fn the_page_never_gets_the_token_back() {
        let mut s = DevSettings::for_build(BOTH);
        s.token_dpapi = "blob".into();
        let f = DevForm::for_page(&s);
        assert_eq!(f.token, "");
        assert!(f.token_saved);
        let json = serde_json::to_string(&f).unwrap();
        assert!(!json.contains("blob"));
        // Unsaved form with the field left empty keeps the saved token;
        // a typed token replaces it; "Clear" (token_saved false) drops it.
        let mut typed = f.clone();
        typed.token = "x".repeat(20);
        assert_eq!(typed.with_token(&s).token, "x".repeat(20));
        let mut cleared = f.clone();
        cleared.token_saved = false;
        assert_eq!(cleared.with_token(&s).token, "");
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

    fn groq(consent_at_ms: u64) -> CloudProviderConfig {
        CloudProviderConfig {
            id: "groq-1".into(),
            preset: "groq".into(),
            adapter: "openai_compat".into(),
            label: "Groq".into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            model: "whisper-large-v3-turbo".into(),
            consent_host: if consent_at_ms > 0 {
                "api.groq.com".into()
            } else {
                String::new()
            },
            consent_at_ms,
            ..CloudProviderConfig::default()
        }
    }

    fn cfg(p: CloudProviderConfig) -> CloudConfig {
        CloudConfig {
            active: p.id.clone(),
            providers: vec![p],
            monthly_budget_usd: None,
        }
    }

    #[test]
    fn consent_survives_unrelated_edits() {
        let saved = cfg(groq(1000));
        let mut new = saved.clone();
        new.providers[0].model = "whisper-large-v3".into();
        new.providers[0].label = "Groq fast".into();
        reconcile_consent(&saved, &mut new);
        assert!(new.providers[0].consent_ok());
        assert_eq!(new.providers[0].consent_at_ms, 1000);
    }

    #[test]
    fn consent_is_void_after_a_host_change() {
        let saved = cfg(groq(1000));
        let mut new = saved.clone();
        new.providers[0].base_url = "https://api.example-stt.com/v1".into();
        reconcile_consent(&saved, &mut new);
        assert!(!new.providers[0].consent_ok());
        assert_eq!(new.providers[0].consent_host, "");
        assert_eq!(new.providers[0].consent_at_ms, 0);
    }

    #[test]
    fn consent_is_void_after_an_adapter_change() {
        let saved = cfg(groq(1000));
        let mut new = saved.clone();
        new.providers[0].adapter = "openrouter".into();
        reconcile_consent(&saved, &mut new);
        assert!(
            !new.providers[0].consent_ok(),
            "same host, other wire format: ask again"
        );
    }

    #[test]
    fn a_fresh_consent_for_the_new_host_is_kept() {
        let saved = cfg(groq(1000));
        let mut new = saved.clone();
        new.providers[0].base_url = "https://api.example-stt.com/v1".into();
        new.providers[0].consent_host = "api.example-stt.com".into();
        new.providers[0].consent_at_ms = 2000;
        reconcile_consent(&saved, &mut new);
        assert!(new.providers[0].consent_ok());
        // ...but a "fresh" consent naming another host is not.
        let mut forged = saved.clone();
        forged.providers[0].base_url = "https://api.example-stt.com/v1".into();
        forged.providers[0].consent_at_ms = 3000; // consent_host still api.groq.com
        reconcile_consent(&saved, &mut forged);
        assert!(!forged.providers[0].consent_ok());
    }

    #[test]
    fn the_form_cannot_backdate_a_consent() {
        let saved = cfg(groq(0));
        let mut new = saved.clone();
        new.providers[0].consent_host = "api.groq.com".into();
        new.providers[0].consent_at_ms = 0;
        reconcile_consent(&saved, &mut new);
        assert!(!new.providers[0].consent_ok());
    }

    #[test]
    fn removed_providers_are_reported_for_key_deletion() {
        let saved = cfg(groq(1000));
        assert_eq!(
            removed_providers(&saved, &CloudConfig::default()),
            vec!["groq-1"]
        );
        assert!(removed_providers(&saved, &saved).is_empty());
    }

    #[test]
    fn cloud_is_off_by_default_and_not_in_the_router_json() {
        let d = DevSettings::for_build(BOTH);
        assert!(!d.allow_cloud && d.cloud.is_default());
        let j = d.router_config_json(BOTH, "");
        for k in ["allow_cloud", "order", "cloud"] {
            assert!(j.get(k).is_none(), "{k}");
        }
        // A build without the cloud client never emits cloud fields.
        let on = DevSettings {
            allow_cloud: true,
            cloud: cfg(groq(1000)),
            ..d
        };
        let no_cloud = BuildCaps {
            cloud: false,
            ..BOTH
        };
        let j = on.router_config_json(no_cloud, "");
        for k in ["allow_cloud", "order", "cloud"] {
            assert!(j.get(k).is_none(), "{k}");
        }
        assert_eq!(
            on.router_config_json(BOTH, "")["order"],
            serde_json::json!(["server", "client", "cloud"])
        );
    }

    #[test]
    fn cloud_form_validation_and_build_gate() {
        let mut f = form();
        f.allow_cloud = true;
        f.cloud = cfg(groq(0));
        f.cloud.providers[0].base_url = " https://api.groq.com/openai/v1/ ".into();
        let (s, _) = f.validate(BOTH).unwrap();
        assert!(s.allow_cloud);
        assert_eq!(
            s.cloud.providers[0].base_url,
            "https://api.groq.com/openai/v1"
        );
        let (s, _) = f
            .validate(BuildCaps {
                cloud: false,
                ..BOTH
            })
            .unwrap();
        assert!(!s.allow_cloud, "the build cannot serve it");
        f.cloud.providers[0].base_url = "http://api.groq.com/openai/v1".into();
        assert_eq!(
            f.validate(BOTH).unwrap_err()[0].field,
            "cloud",
            "public http refused"
        );
        f.cloud.providers[0].base_url = "https://api.groq.com/openai/v1".into();
        f.cloud.active = "nope".into();
        assert_eq!(f.validate(BOTH).unwrap_err()[0].field, "cloud");
    }

    #[test]
    fn cloud_warnings_name_the_host_and_what_is_missing() {
        let d = DevSettings {
            allow_cloud: true,
            cloud: cfg(groq(0)),
            route_order: vec![Route::Server, Route::Cloud, Route::Client],
            ..DevSettings::for_build(BOTH)
        };
        let w = d.cloud_warnings(BOTH, &|_| false);
        assert!(
            w.iter()
                .any(|x| x.contains("accept sending audio to Groq (api.groq.com)")),
            "{w:?}"
        );
        assert!(w.iter().any(|x| x.contains("API key")), "{w:?}");
        assert!(
            w.iter().any(|x| x.contains("server is unreachable")),
            "{w:?}"
        );
        let ok = DevSettings {
            cloud: cfg(groq(5)),
            route_order: vec![Route::Cloud, Route::Client],
            ..d
        };
        assert!(ok.cloud_warnings(BOTH, &|_| true).is_empty());
        assert!(DevSettings::for_build(BOTH)
            .cloud_warnings(BOTH, &|_| false)
            .is_empty());
    }
}
