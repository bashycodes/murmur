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
//! * [`DevSettings::router_config_json`] produces murmur-router's
//!   `RouterConfig` JSON (§1.2) — the router fixtures are the spec.
//!
//! Routes are an ordered list (`route_order`), not a two-way switch; the
//! opt-in cloud speech-to-text (OpenRouter, D18/D28, `docs/cloud-stt.md`) is
//! the third entry. The router's `prefer` is derived from the order; with
//! the cloud allowed the full list goes to the router as `order`.
//!
//! OpenRouter (`cloud`) is murmur-router's schema-2 `CloudConfig` as is: one
//! fixed host (`openrouter.ai`), two uses (speech-to-text, text cleanup),
//! each with its own consent; no API key (Credential Manager,
//! [`crate::credentials`]). The Developer form does not carry it: the
//! OpenRouter page edits it with its own commands ([`crate::openrouter`]),
//! and a form save keeps the stored values ([`DevSettings::keep_cloud`]).
//! A D18 provider list on disk is migrated once ([`DevSettings::migrate_d28`]).
//!
//! AI cleanup (D36) is its own ordered list of all three engines
//! ([`CleanupEngine`]: on-device LLM, your server, OpenRouter), each with an
//! on/off flag; switched-off entries keep their place. The pre-D36 pair
//! (`allow_cloud_cleanup` + a `[cloud, local]` order) is migrated once
//! ([`DevSettings::migrate_d36`]). The router always gets the whole list
//! (`cleanup_order` + `cleanup_off`).
//!
//! Meeting transcription (D47) has its own route order ([`MeetingRoute`]:
//! this PC, your server, OpenRouter, each with an on/off flag), used only
//! by the Notetaker's per-job engine ([`DevSettings::for_meetings`]).
//! Missing in the file = dictation's order and switches, followed live
//! until the user edits the meeting list.

use murmur_core::config::ParakeetDecoding;
use murmur_router::{CleanupRoute, CloudConfig, MigrationReport};
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

/// One entry of the D36 cleanup order: an engine and whether the user
/// switched it on. OpenRouter's flag is the cloud-cleanup switch (it needs
/// its own consent, [`crate::openrouter::apply_cleanup`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupEngine {
    pub route: CleanupRoute,
    pub enabled: bool,
}

/// The D36 default: this PC → your server → OpenRouter (off until the
/// user turns it on and consents).
pub const DEFAULT_CLEANUP: [CleanupEngine; 3] = [
    CleanupEngine {
        route: CleanupRoute::Local,
        enabled: true,
    },
    CleanupEngine {
        route: CleanupRoute::Server,
        enabled: true,
    },
    CleanupEngine {
        route: CleanupRoute::Cloud,
        enabled: false,
    },
];

/// `list` as a full cleanup order: duplicates dropped, then each missing
/// engine added (`server` right after `local`, the others at the end).
pub fn full_cleanup_order(list: &[CleanupRoute]) -> Vec<CleanupRoute> {
    let mut v: Vec<CleanupRoute> = Vec::new();
    for r in list {
        if DEFAULT_CLEANUP.iter().any(|e| e.route == *r) && !v.contains(r) {
            v.push(*r);
        }
    }
    if !v.contains(&CleanupRoute::Local) {
        v.push(CleanupRoute::Local);
    }
    if !v.contains(&CleanupRoute::Server) {
        let at = v
            .iter()
            .position(|r| *r == CleanupRoute::Local)
            .map_or(v.len(), |i| i + 1);
        v.insert(at, CleanupRoute::Server);
    }
    if !v.contains(&CleanupRoute::Cloud) {
        v.push(CleanupRoute::Cloud);
    }
    v
}

/// One entry of the D47 meeting transcription order: a route and whether
/// meetings may use it. Independent of dictation's switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingRoute {
    pub route: Route,
    pub enabled: bool,
}

/// One way to run a dictation. Order in `route_order` = preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Self-hosted murmur-server (router `Remote`).
    Server,
    /// On-device murmur-core (router `Local`).
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

    pub fn to_router(self) -> murmur_router::Route {
        match self {
            Route::Server => murmur_router::Route::Server,
            Route::Client => murmur_router::Route::Client,
            Route::Cloud => murmur_router::Route::Cloud,
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
            connect_timeout_ms: murmur_router::DEFAULT_CONNECT_TIMEOUT_MS,
            commit_timeout_base_ms: 3000,
            health_ttl_s: 30,
        }
    }
}

/// D78/D85: shown once when [`DevSettings::one_cloud_provider`] turns cloud
/// speech off.
pub const LITELLM_STT_OFF_NOTICE: &str = "Speech to text in the cloud is now off: your Cloud provider is LiteLLM and no speech model is set for your proxy. Speech stays on this PC and your server.";

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
    /// Opt-in cloud speech-to-text (D18 C1). Off by default; off while
    /// LiteLLM is the Cloud provider without a speech model (D78, D85).
    pub allow_cloud: bool,
    /// OpenRouter settings, schema 2, without the key (router
    /// `RouterConfig.cloud`). A schema-1 (D18 provider list) value is
    /// migrated at startup.
    pub cloud: CloudConfig,
    /// D36: who cleans the text, in the order tried, each engine once with
    /// its on/off flag ([`DevSettings::cleanup`] reads it). OpenRouter's
    /// flag is the opt-in cloud cleanup (D28, own consent). Empty = a file
    /// from before D36 ([`DevSettings::migrate_d36`]): missing in the file
    /// reads as empty (not the default list), so the old fields count.
    #[serde(default)]
    pub cleanup_engines: Vec<CleanupEngine>,
    /// D47: where meetings are transcribed, in the order tried, each route
    /// once with its own flag ([`DevSettings::meeting_order`] reads it).
    /// Empty (missing in the file) = dictation's order and switches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meeting_routes: Vec<MeetingRoute>,
    /// Pre-D36 `allow_cloud_cleanup`: read by the migration, never written.
    #[serde(rename = "allow_cloud_cleanup", skip_serializing)]
    pub legacy_allow_cloud_cleanup: bool,
    /// Pre-D36 `cleanup_order` (`[cloud, local]` / `[local, cloud]`): read
    /// by the migration, never written.
    #[serde(rename = "cleanup_order", skip_serializing)]
    pub legacy_cleanup_order: Vec<CleanupRoute>,
    /// The one-time D28 migration notice, until the user dismissed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_notice: Option<String>,
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
            cleanup_engines: DEFAULT_CLEANUP.to_vec(),
            meeting_routes: Vec::new(),
            legacy_allow_cloud_cleanup: false,
            legacy_cleanup_order: Vec::new(),
            cloud_notice: None,
            debug_log: false,
            decode_while_speaking: true,
            parakeet_decoding: ParakeetDecoding::Auto,
        }
    }

    /// The OpenRouter part of `saved` (both uses, consents, budget, the
    /// pending notice), the D36 cleanup order and the D47 meeting order: a
    /// Developer form save keeps them.
    pub fn keep_cloud(&mut self, saved: &DevSettings) {
        self.allow_cloud = saved.allow_cloud;
        self.cloud = saved.cloud.clone();
        self.cleanup_engines = saved.cleanup();
        self.meeting_routes = saved.meeting_routes.clone();
        self.legacy_allow_cloud_cleanup = false;
        self.legacy_cleanup_order = Vec::new();
        self.cloud_notice = saved.cloud_notice.clone();
    }

    /// One-time D28 migration of a D18 provider list (openrouter-only-cloud.md §2.1, rules in
    /// `murmur_router::migrate`). Idempotent: a schema-2 config returns an
    /// empty report. The caller applies the key moves / deletions
    /// ([`crate::credentials::apply_migration`]) and persists. When the
    /// cloud route was switched off it also moves `cloud` to the end of the
    /// route order (rule 3: "remove cloud from order").
    pub fn migrate_d28(&mut self) -> MigrationReport {
        if !self.cloud.is_legacy() {
            return MigrationReport::default();
        }
        let mut rc = murmur_router::RouterConfig {
            allow_cloud: self.allow_cloud,
            order: if self.allow_cloud {
                self.order().into_iter().map(Route::to_router).collect()
            } else {
                Vec::new()
            },
            cloud: std::mem::take(&mut self.cloud),
            ..murmur_router::RouterConfig::default()
        };
        let rep = rc.migrate_d28();
        self.cloud = rc.cloud;
        self.allow_cloud = rc.allow_cloud;
        if rep.cloud_disabled {
            let mut order: Vec<Route> = self
                .order()
                .into_iter()
                .filter(|r| *r != Route::Cloud)
                .collect();
            order.push(Route::Cloud);
            self.route_order = order;
        }
        if rep.notice.is_some() {
            self.cloud_notice = rep.notice.clone();
        }
        rep
    }

    /// D78/D85: one Cloud provider at a time. A saved file with LiteLLM as
    /// the provider, no speech model for it and cloud speech on (made
    /// before D78) has that switch turned off, with a one-time notice.
    /// `true` when something changed (the caller saves prefs.json).
    pub fn one_cloud_provider(&mut self) -> bool {
        if self.cloud.offers_stt() || !self.allow_cloud {
            return false;
        }
        self.allow_cloud = false;
        self.cloud_notice = Some(LITELLM_STT_OFF_NOTICE.into());
        true
    }

    /// The cleanup order, always all three engines once (D36). A file from
    /// before D36 (no `cleanup_engines`) reads as migrated: the user's
    /// cloud-vs-local order is kept, the server goes right after this PC,
    /// and OpenRouter is on only if cloud cleanup was allowed.
    pub fn cleanup(&self) -> Vec<CleanupEngine> {
        if self.cleanup_engines.is_empty() {
            let cloud = self.legacy_allow_cloud_cleanup
                && self.legacy_cleanup_order.contains(&CleanupRoute::Cloud);
            return full_cleanup_order(&self.legacy_cleanup_order)
                .into_iter()
                .map(|route| CleanupEngine {
                    route,
                    enabled: route != CleanupRoute::Cloud || cloud,
                })
                .collect();
        }
        let order: Vec<CleanupRoute> = self.cleanup_engines.iter().map(|e| e.route).collect();
        full_cleanup_order(&order)
            .into_iter()
            .map(|route| {
                let stored = self.cleanup_engines.iter().find(|e| e.route == route);
                let default = DEFAULT_CLEANUP.iter().find(|e| e.route == route);
                CleanupEngine {
                    route,
                    enabled: stored.or(default).is_some_and(|e| e.enabled),
                }
            })
            .collect()
    }

    /// Whether the user switched `route` on in the cleanup order.
    pub fn cleanup_on(&self, route: CleanupRoute) -> bool {
        self.cleanup().iter().any(|e| e.route == route && e.enabled)
    }

    /// Set one engine's flag (it keeps its place).
    pub fn set_cleanup_on(&mut self, route: CleanupRoute, on: bool) {
        let mut v = self.cleanup();
        for e in v.iter_mut().filter(|e| e.route == route) {
            e.enabled = on;
        }
        self.cleanup_engines = v;
    }

    /// One-time D36 migration of the pre-D36 cleanup pair to the full
    /// list ([`DevSettings::cleanup`]); idempotent. Runs on every load.
    /// `cleanup_was_on` = the pre-D36 prefs ran AI cleanup at all; when they
    /// did not, the new server entry starts off so an upgrade never turns
    /// cleanup on (user decision). Already-migrated lists are left alone.
    pub fn migrate_d36(&mut self, cleanup_was_on: bool) {
        let legacy = self.cleanup_engines.is_empty();
        self.cleanup_engines = self.cleanup();
        if legacy && !cleanup_was_on {
            for e in self
                .cleanup_engines
                .iter_mut()
                .filter(|e| e.route == CleanupRoute::Server)
            {
                e.enabled = false;
            }
        }
        self.legacy_allow_cloud_cleanup = false;
        self.legacy_cleanup_order = Vec::new();
    }

    /// Dictation's switch for `route` (the user toggle, not the build).
    fn dictation_on(&self, route: Route) -> bool {
        match route {
            Route::Server => self.allow_server,
            Route::Client => self.allow_client,
            Route::Cloud => self.allow_cloud,
        }
    }

    /// D47: the meeting transcription order, always all three routes once.
    /// Nothing stored = dictation's order and switches (live); a stored
    /// list missing a route gets it at the end with dictation's switch.
    pub fn meeting_order(&self) -> Vec<MeetingRoute> {
        let mut v: Vec<MeetingRoute> = Vec::new();
        for e in &self.meeting_routes {
            if !v.iter().any(|x| x.route == e.route) {
                v.push(*e);
            }
        }
        for route in self.order() {
            if !v.iter().any(|x| x.route == route) {
                v.push(MeetingRoute {
                    route,
                    enabled: self.dictation_on(route),
                });
            }
        }
        v
    }

    /// The meeting order still follows dictation's (never edited).
    pub fn meeting_follows_dictation(&self) -> bool {
        self.meeting_routes.is_empty()
    }

    /// Store `list` as the meeting order (completed as
    /// [`DevSettings::meeting_order`]). Turning OpenRouter on needs the
    /// speech-to-text consent for openrouter.ai: meeting audio goes there.
    pub fn set_meeting_order(&mut self, list: &[MeetingRoute]) -> Result<(), String> {
        let mut next = self.clone();
        next.meeting_routes = list.to_vec();
        let full = next.meeting_order();
        let cloud_on = full.iter().any(|e| e.route == Route::Cloud && e.enabled);
        let cloud_was_on = self
            .meeting_order()
            .iter()
            .any(|e| e.route == Route::Cloud && e.enabled);
        if cloud_on && !cloud_was_on && !self.cloud.openrouter.stt.consent_ok() {
            return Err(MEETING_CLOUD_CONSENT_MSG.into());
        }
        self.meeting_routes = full;
        Ok(())
    }

    /// D47: these settings as meetings see them: the meeting order as the
    /// route order and its flags as the route switches. Everything else
    /// (server, OpenRouter settings and consents) is dictation's.
    pub fn for_meetings(&self) -> DevSettings {
        let order = self.meeting_order();
        let on = |r: Route| order.iter().any(|e| e.route == r && e.enabled);
        DevSettings {
            allow_server: on(Route::Server),
            allow_client: on(Route::Client),
            allow_cloud: on(Route::Cloud),
            route_order: order.iter().map(|e| e.route).collect(),
            meeting_routes: Vec::new(),
            ..self.clone()
        }
    }

    /// The meeting routes this build can try, in order (the user's flag
    /// AND the build; consent, key and reachability are the router's).
    pub fn meeting_routes_usable(&self, caps: BuildCaps) -> Vec<Route> {
        self.meeting_order()
            .into_iter()
            .filter(|e| {
                e.enabled
                    && match e.route {
                        Route::Server => caps.remote,
                        Route::Client => caps.local,
                        Route::Cloud => caps.cloud,
                    }
            })
            .map(|e| e.route)
            .collect()
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

    /// Cloud cleanup is switched on (build AND OpenRouter's entry in the
    /// cleanup order); consent, key, budget and circuit are the router's.
    pub fn cleanup_effective(&self, caps: BuildCaps) -> bool {
        caps.cloud && self.cleanup_on(CleanupRoute::Cloud)
    }

    /// Developer tab visibility (§5 "Access").
    pub fn tab_visible(&self, caps: BuildCaps) -> bool {
        caps.remote || self.dev_unlocked
    }

    /// The token in clear ("" when none or unreadable on this account).
    pub fn token(&self) -> String {
        crate::secrets::unprotect(&self.token_dpapi).unwrap_or_default()
    }

    /// murmur-router `RouterConfig` JSON (§1.2). `token` is passed in so the
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
        // D36: always the whole cleanup order, so the router never falls
        // back to the pre-D36 rule; an engine the user or the build switched
        // off keeps its place in `cleanup_off`.
        let cleanup = self.cleanup();
        let built = |r: CleanupRoute| match r {
            CleanupRoute::Local => caps.local,
            CleanupRoute::Server => caps.remote,
            _ => caps.cloud,
        };
        j["cleanup_order"] =
            serde_json::to_value(cleanup.iter().map(|e| e.route).collect::<Vec<_>>())
                .expect("cleanup order serialises");
        let off: Vec<CleanupRoute> = cleanup
            .iter()
            .filter(|e| !e.enabled || !built(e.route))
            .map(|e| e.route)
            .collect();
        if !off.is_empty() {
            j["cleanup_off"] = serde_json::to_value(off).expect("cleanup off serialises");
        }
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
            if self.cleanup_on(CleanupRoute::Cloud) {
                j["allow_cloud_cleanup"] = true.into();
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
                w.push("Unencrypted: OK only on your LAN or over an encrypted VPN.".into());
            }
        }
        if srv && self.token_dpapi.is_empty() {
            w.push("No token set: a server with auth enabled will reject this PC.".into());
        }
        w
    }

    /// OpenRouter warnings (§3.1 / D28): where audio and text may go, and
    /// what is still missing before a use can run. `has_key` = an
    /// OpenRouter key is in the credential store.
    pub fn cloud_warnings(&self, caps: BuildCaps, has_key: bool) -> Vec<String> {
        let mut w = Vec::new();
        let or = &self.cloud.openrouter;
        if self.cloud_effective(caps) {
            if !or.stt.consent_ok() {
                w.push(format!(
                    "Cloud speech-to-text is not used until you accept sending audio to {OPENROUTER_NAME}."
                ));
            }
            if !has_key {
                w.push(
                    "Cloud speech-to-text is not used until an OpenRouter API key is saved.".into(),
                );
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
                        "When your server is unreachable, audio goes to {OPENROUTER_NAME}."
                    )),
                    Some(Route::Client) => w.push(format!(
                        "When no local model is loaded, audio goes to {OPENROUTER_NAME}."
                    )),
                    _ => {}
                }
            }
        }
        let custom = !self.cloud.provider.is_openrouter();
        if self.cleanup_effective(caps) && custom {
            // D52: the user's own server; its key is optional.
            let cc = &self.cloud.custom_cleanup;
            if !cc.consent_ok() {
                let host = cc.host();
                w.push(format!(
                    "Cloud text cleanup is not used until you accept sending text to {}.",
                    if host.is_empty() {
                        "your server"
                    } else {
                        &host
                    }
                ));
            }
            if cc
                .base_url
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("http://")
            {
                w.push(CUSTOM_PLAIN_HTTP_MSG.into());
            }
        } else if self.cleanup_effective(caps) {
            if !or.cleanup.consent_ok() {
                w.push(format!(
                    "Cloud text cleanup is not used until you accept sending text to {OPENROUTER_NAME}."
                ));
            }
            if !has_key {
                w.push(
                    "Cloud text cleanup is not used until an OpenRouter API key is saved.".into(),
                );
            }
        }
        if (self.cloud_effective(caps) || (self.cleanup_effective(caps) && !custom))
            && !or.privacy.zdr
        {
            w.push("Zero-retention routing is off: some providers may keep or train on your audio or text.".into());
        }
        w
    }
}

/// D52: a custom cleanup server reached over plain `http://`.
pub const CUSTOM_PLAIN_HTTP_MSG: &str =
    "Your cleanup server uses http://: the key and your dictated text are sent unencrypted.";

/// How the page names the one cloud host.
pub const OPENROUTER_NAME: &str = "OpenRouter (openrouter.ai)";

/// Turning the cloud on for meetings before its audio consent (D47).
pub const MEETING_CLOUD_CONSENT_MSG: &str =
    "Accept sending audio to OpenRouter first: turn Cloud on under Engines above";

/// A meeting job when no route is on for meetings (D47).
pub const NO_MEETING_ROUTE_MSG: &str =
    "Meeting transcription is off: turn on an engine under Settings → Speech to text → Meetings";

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
            // OpenRouter is not part of the form: the caller keeps the
            // stored values (`keep_cloud`).
            allow_cloud: false,
            cloud: CloudConfig::default(),
            cleanup_engines: DEFAULT_CLEANUP.to_vec(),
            meeting_routes: Vec::new(),
            legacy_allow_cloud_cleanup: false,
            legacy_cleanup_order: Vec::new(),
            cloud_notice: None,
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
/// config time? The router's classifier decides (`murmur_router::net`, the
/// same one `ServerConfig::validate` uses), so the form can never disagree
/// with the gate. Private = loopback, RFC 1918, link-local, ULA, VPN
/// (100.64/10, fd7a:115c:a1e0::/48), `*.ts.net`, `*.local`, `localhost`
/// and single-label names. That is only the form rule: at connect time
/// murmur-remote resolves a name and requires every address to be private
/// (review S6), so a `*.ts.net` or LAN name that resolves publicly is
/// refused then ("server needs TLS").
pub fn host_is_public(host: &str) -> bool {
    !murmur_router::net::classify(host).plaintext_ok()
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
                "keep_local_warm": true,
                "cleanup_order": ["local", "server", "cloud"],
                "cleanup_off": ["cloud"]
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

    fn openrouter_on() -> DevSettings {
        let mut d = DevSettings::for_build(BOTH);
        d.allow_cloud = true;
        d.route_order = vec![Route::Cloud, Route::Server, Route::Client];
        d.cloud.openrouter.stt.consent_host = "openrouter.ai".into();
        d.cloud.openrouter.stt.consent_at_ms = 1000;
        d.cleanup_engines = full_cleanup_order(&[CleanupRoute::Cloud, CleanupRoute::Local])
            .into_iter()
            .map(|route| CleanupEngine {
                route,
                enabled: true,
            })
            .collect();
        d.cloud.openrouter.cleanup.consent_host = "openrouter.ai".into();
        d.cloud.openrouter.cleanup.consent_at_ms = 2000;
        d
    }

    #[test]
    fn cloud_is_off_by_default_and_not_in_the_router_json() {
        let d = DevSettings::for_build(BOTH);
        assert!(!d.allow_cloud && !d.cleanup_on(CleanupRoute::Cloud) && d.cloud.is_default());
        assert_eq!(d.cloud.schema, 2);
        let j = d.router_config_json(BOTH, "");
        for k in ["allow_cloud", "order", "cloud", "allow_cloud_cleanup"] {
            assert!(j.get(k).is_none(), "{k}");
        }
        // D36: the whole cleanup order always goes, OpenRouter switched off.
        assert_eq!(
            j["cleanup_order"],
            serde_json::json!(["local", "server", "cloud"])
        );
        assert_eq!(j["cleanup_off"], serde_json::json!(["cloud"]));
        // A build without the cloud client never emits cloud fields.
        let on = openrouter_on();
        let no_cloud = BuildCaps {
            cloud: false,
            ..BOTH
        };
        let j = on.router_config_json(no_cloud, "");
        for k in ["allow_cloud", "order", "cloud", "allow_cloud_cleanup"] {
            assert!(j.get(k).is_none(), "{k}");
        }
        assert_eq!(
            j["cleanup_order"],
            serde_json::json!(["cloud", "local", "server"])
        );
        assert_eq!(
            j["cleanup_off"],
            serde_json::json!(["cloud"]),
            "not in this build"
        );
        let j = on.router_config_json(BOTH, "");
        assert_eq!(j["order"], serde_json::json!(["cloud", "server", "client"]));
        assert_eq!(j["allow_cloud_cleanup"], true);
        assert_eq!(
            j["cleanup_order"],
            serde_json::json!(["cloud", "local", "server"])
        );
        assert!(j.get("cleanup_off").is_none());
        assert_eq!(j["cloud"]["schema"], 2);
        let cfg: murmur_router::RouterConfig = serde_json::from_value(j).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.cloud_cleanup_wanted());
        assert!(cfg.cloud.openrouter.stt.consent_ok() && cfg.cloud.openrouter.cleanup.consent_ok());
    }

    #[test]
    fn a_developer_form_save_keeps_the_openrouter_settings() {
        let saved = openrouter_on();
        let f = DevForm::from_settings(&saved, "");
        let j = serde_json::to_value(&f).unwrap();
        for k in ["cloud", "allow_cloud", "allow_cloud_cleanup"] {
            assert!(j.get(k).is_none(), "the form never carries {k}");
        }
        let (mut s, _) = f.validate(BOTH).unwrap();
        assert!(!s.allow_cloud, "validate alone knows nothing of OpenRouter");
        s.keep_cloud(&saved);
        assert_eq!(s.cloud, saved.cloud);
        assert!(s.allow_cloud && s.cleanup_on(CleanupRoute::Cloud));
        assert_eq!(s.cleanup(), saved.cleanup());
        // A page that still posts the old fields cannot change them.
        let old: DevForm = serde_json::from_value(serde_json::json!({
            "allow_cloud": true,
            "cloud": {"active": "x", "providers": []}
        }))
        .unwrap();
        let (s, _) = old.validate(BOTH).unwrap();
        assert!(!s.allow_cloud && s.cloud.is_default());
    }

    #[test]
    fn migration_runs_once_and_keeps_an_openrouter_entry() {
        let mut d: DevSettings = serde_json::from_value(serde_json::json!({
            "allow_cloud": true,
            "route_order": ["cloud", "client", "server"],
            "cloud": {
                "active": "openrouter-1",
                "monthly_budget_usd": 2.0,
                "providers": [{
                    "id": "openrouter-1", "preset": "openrouter", "adapter": "openrouter",
                    "label": "OpenRouter", "base_url": "https://openrouter.ai/api/v1",
                    "model": "openai/whisper-large-v3", "consent_host": "openrouter.ai",
                    "consent_at_ms": 5, "zero_retention": true
                }]
            }
        }))
        .unwrap();
        assert!(d.cloud.is_legacy());
        let rep = d.migrate_d28();
        assert!(rep.migrated);
        assert_eq!(rep.key_from.as_deref(), Some("openrouter-1"));
        assert!(!rep.cloud_disabled && d.allow_cloud);
        assert_eq!(d.route_order[0], Route::Cloud);
        assert_eq!(d.cloud.schema, 2);
        assert_eq!(d.cloud.openrouter.stt.model, "openai/whisper-large-v3");
        assert!(d.cloud.openrouter.stt.consent_ok());
        assert!(
            !d.cloud.openrouter.cleanup.consent_ok(),
            "STT consent is not cleanup consent"
        );
        assert_eq!(d.cloud.monthly_budget_usd, Some(murmur_router::Usd(2.0)));
        assert_eq!(d.cloud_notice, None, "nothing user-visible changed");
        let again = d.clone().migrate_d28();
        assert!(!again.migrated, "idempotent");
        let j = serde_json::to_value(&d).unwrap();
        assert!(j["cloud"].get("providers").is_none() && j["cloud"].get("active").is_none());
    }

    #[test]
    fn warnings_name_openrouter_and_what_is_missing() {
        let mut d = openrouter_on();
        d.route_order = vec![Route::Server, Route::Cloud, Route::Client];
        d.cloud.openrouter.stt.consent_at_ms = 0;
        d.cloud.openrouter.cleanup.consent_at_ms = 0;
        let w = d.cloud_warnings(BOTH, false);
        let has = |s: &str| w.iter().any(|x| x.contains(s));
        assert!(
            has("accept sending audio to OpenRouter (openrouter.ai)"),
            "{w:?}"
        );
        assert!(
            has("accept sending text to OpenRouter (openrouter.ai)"),
            "{w:?}"
        );
        assert!(has("API key"), "{w:?}");
        assert!(has("server is unreachable"), "{w:?}");
        let ok = DevSettings {
            route_order: vec![Route::Cloud, Route::Client],
            ..openrouter_on()
        };
        assert!(
            ok.cloud_warnings(BOTH, true).is_empty(),
            "{:?}",
            ok.cloud_warnings(BOTH, true)
        );
        let mut no_zdr = ok.clone();
        no_zdr.cloud.openrouter.privacy.zdr = false;
        assert!(no_zdr.cloud_warnings(BOTH, true)[0].contains("Zero-retention"));
        assert!(DevSettings::for_build(BOTH)
            .cloud_warnings(BOTH, false)
            .is_empty());
    }

    fn mr(route: Route, enabled: bool) -> MeetingRoute {
        MeetingRoute { route, enabled }
    }

    /// D47 migration: a file without `meeting_routes` gets dictation's
    /// order and switches, and keeps following them until edited.
    #[test]
    fn meeting_order_missing_in_the_file_is_dictations() {
        let mut d: DevSettings = serde_json::from_str(
            r#"{"allow_server":true,"allow_client":false,"route_order":["client","server"]}"#,
        )
        .unwrap();
        assert!(d.meeting_follows_dictation());
        assert_eq!(
            d.meeting_order(),
            vec![
                mr(Route::Client, false),
                mr(Route::Server, true),
                mr(Route::Cloud, false)
            ]
        );
        d.allow_client = true;
        d.route_order = vec![Route::Server, Route::Client];
        assert_eq!(
            d.meeting_order()[..2],
            [mr(Route::Server, true), mr(Route::Client, true)],
            "follows dictation live"
        );
        let j = serde_json::to_value(&d).unwrap();
        assert!(
            j.get("meeting_routes").is_none(),
            "not written until edited"
        );
        // Fresh install: the same order as dictation's.
        let fresh = DevSettings::for_build(BOTH);
        assert_eq!(
            fresh
                .meeting_order()
                .iter()
                .map(|e| e.route)
                .collect::<Vec<_>>(),
            fresh.order()
        );
    }

    #[test]
    fn meeting_order_is_its_own_list() {
        let mut d = DevSettings::for_build(BOTH);
        d.set_meeting_order(&[mr(Route::Client, true), mr(Route::Client, false)])
            .unwrap();
        assert_eq!(
            d.meeting_order(),
            vec![
                mr(Route::Client, true),
                mr(Route::Server, true),
                mr(Route::Cloud, false)
            ],
            "deduplicated, completed with dictation's switches"
        );
        assert!(!d.meeting_follows_dictation());
        // Dictation is unaffected, and dictation edits no longer move it.
        assert_eq!(d.order()[0], Route::Server);
        d.allow_server = false;
        d.route_order = vec![Route::Cloud];
        assert_eq!(d.meeting_order()[1], mr(Route::Server, true));
        // Switched off keeps its place; persists.
        d.set_meeting_order(&[
            mr(Route::Client, true),
            mr(Route::Server, false),
            mr(Route::Cloud, false),
        ])
        .unwrap();
        let back: DevSettings = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(back.meeting_order(), d.meeting_order());
        assert_eq!(back.meeting_order()[1], mr(Route::Server, false));
    }

    #[test]
    fn meeting_cloud_needs_the_audio_consent() {
        let mut d = DevSettings::for_build(BOTH);
        let e = d.set_meeting_order(&[mr(Route::Cloud, true)]).unwrap_err();
        assert_eq!(e, MEETING_CLOUD_CONSENT_MSG);
        assert!(d.meeting_follows_dictation(), "nothing stored on error");
        let mut ok = openrouter_on();
        ok.set_meeting_order(&[mr(Route::Cloud, true), mr(Route::Client, true)])
            .unwrap();
        assert_eq!(ok.meeting_routes_usable(BOTH)[0], Route::Cloud);
    }

    /// The router config a meeting job gets: the meeting order and flags;
    /// dictation's config does not change.
    #[test]
    fn for_meetings_is_the_meeting_order_as_routes() {
        let mut d = DevSettings::for_build(BOTH);
        d.server.host = "100.64.0.10".into();
        let dictation = d.router_config_json(BOTH, "");
        d.set_meeting_order(&[mr(Route::Client, true), mr(Route::Server, false)])
            .unwrap();
        assert_eq!(
            d.router_config_json(BOTH, ""),
            dictation,
            "dictation unaffected"
        );
        let m = d.for_meetings().router_config_json(BOTH, "");
        assert_eq!(m["prefer"], "client");
        assert_eq!(m["allow_client"], true);
        assert_eq!(m["allow_server"], false);
        assert_eq!(dictation["prefer"], "server");
        assert_eq!(
            d.meeting_routes_usable(BOTH),
            vec![Route::Client],
            "off and unbuilt routes are not usable"
        );
        assert_eq!(
            d.meeting_routes_usable(BuildCaps {
                local: false,
                remote: true,
                cloud: false
            }),
            Vec::<Route>::new()
        );
    }

    #[test]
    fn a_developer_form_save_keeps_the_meeting_order() {
        let mut saved = DevSettings::for_build(BOTH);
        saved.set_meeting_order(&[mr(Route::Client, true)]).unwrap();
        let (mut next, _) = form().validate(BOTH).unwrap();
        assert!(next.meeting_follows_dictation());
        next.keep_cloud(&saved);
        assert_eq!(next.meeting_order(), saved.meeting_order());
    }
}
