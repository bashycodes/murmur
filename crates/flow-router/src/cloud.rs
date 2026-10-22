//! Opt-in third-party cloud STT: config, secrets and usage types (D18,
//! `docs/cloud-stt.md` §3, §8). No network code: the provider clients live
//! in `flow-cloud`, injected through [`crate::CloudFactory`]. Client-only
//! builds parse these types but can never reach a provider.

use std::fmt;

use serde::{Deserialize, Serialize};

pub use flow_core::Usd;

use crate::net;

/// One routing step in `RouterConfig::order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Route {
    /// Own flow-server (engine `remote`).
    Server,
    /// On-device engine (engine `local`).
    Client,
    /// Third-party cloud STT (engine `cloud`).
    Cloud,
}

impl Route {
    pub fn engine(self) -> crate::EngineKind {
        match self {
            Route::Server => crate::EngineKind::Remote,
            Route::Client => crate::EngineKind::Local,
            Route::Cloud => crate::EngineKind::Cloud,
        }
    }
}

/// An API key held in memory only. Zeroed on drop; `Debug`/`Display` print
/// `***`; deliberately not `Serialize`, so it cannot end up in config
/// JSON, logs or protocol frames by accident (invariant C3).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    /// The key itself, for the one place that needs it: the HTTP header.
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl Drop for SecretString {
    /// Overwrite the key bytes before the allocation is freed (volatile, so
    /// the store is not optimised away). No `zeroize` dependency: this crate
    /// stays dependency-light for client-only builds.
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8; the length is unchanged.
        let v = unsafe { self.0.as_mut_vec() };
        for b in v.iter_mut() {
            // SAFETY: `b` is a valid, aligned, exclusive reference.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Adapters a provider entry may name (`flow-cloud` implements them).
pub const ADAPTERS: [&str; 5] = [
    "openai_compat",
    "openrouter",
    "elevenlabs",
    "deepgram",
    "assemblyai",
];

pub const MAX_PROVIDERS: usize = 8;

/// `RouterConfig.cloud` (§8.1). Keys are never in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudConfig {
    /// Provider id used by the `cloud` route; `""` = none.
    pub active: String,
    pub providers: Vec<CloudProviderConfig>,
    /// Stop using cloud for the month once this much was spent.
    pub monthly_budget_usd: Option<Usd>,
}

impl CloudConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn active_provider(&self) -> Option<&CloudProviderConfig> {
        (!self.active.is_empty())
            .then(|| self.providers.iter().find(|p| p.id == self.active))
            .flatten()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.providers.len() > MAX_PROVIDERS {
            return Err(format!("at most {MAX_PROVIDERS} cloud providers"));
        }
        for (i, p) in self.providers.iter().enumerate() {
            p.validate()?;
            if self.providers[..i].iter().any(|q| q.id == p.id) {
                return Err(format!("duplicate cloud provider id {:?}", p.id));
            }
        }
        if !self.active.is_empty() && self.active_provider().is_none() {
            return Err(format!(
                "active cloud provider {:?} does not exist",
                self.active
            ));
        }
        if let Some(b) = self.monthly_budget_usd {
            if !(b.0.is_finite() && b.0 >= 0.0) {
                return Err("monthly budget must be a non-negative amount".into());
            }
        }
        Ok(())
    }
}

/// One configured provider (§8.1). `preset` is informational (UI);
/// `adapter` picks the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudProviderConfig {
    /// `[a-z0-9-]{1,32}`, unique; also the secure-storage key name.
    pub id: String,
    pub preset: String,
    pub adapter: String,
    /// openai_compat only: `""` = auto from the model id, `whisper`, `gpt_transcribe`.
    pub dialect: String,
    pub label: String,
    pub base_url: String,
    pub model: String,
    /// `""` = follow `SessionConfig.languages`.
    pub language: String,
    pub send_dictionary: bool,
    pub send_screen_words: bool,
    /// Adapter-specific zero-retention flag (§2): OpenRouter `provider.zdr`,
    /// ElevenLabs `enable_logging=false`, Deepgram `mip_opt_out`,
    /// AssemblyAI delete-after-fetch.
    pub zero_retention: bool,
    /// `""` = adapter default, `"wav"`, `"flac"`.
    pub format_override: String,
    pub timeout_base_ms: u64,
    /// `None` = preset price.
    pub price_per_hour_usd: Option<Usd>,
    /// Host the user consented to send audio to (set by the consent dialog).
    pub consent_host: String,
    pub consent_at_ms: u64,
}

impl Default for CloudProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            preset: "custom".into(),
            adapter: "openai_compat".into(),
            dialect: String::new(),
            label: String::new(),
            base_url: String::new(),
            model: String::new(),
            language: String::new(),
            send_dictionary: true,
            send_screen_words: false,
            zero_retention: true,
            format_override: String::new(),
            timeout_base_ms: 6000,
            price_per_hour_usd: None,
            consent_host: String::new(),
            consent_at_ms: 0,
        }
    }
}

impl CloudProviderConfig {
    /// Lower-cased host of `base_url` (no port, IPv6 unbracketed), or `""`.
    pub fn host(&self) -> String {
        split_url(&self.base_url)
            .map(|(_, h)| h)
            .unwrap_or_default()
    }

    /// The user consented to send audio to this provider's current host.
    /// Changing the base URL to another host voids the consent.
    pub fn consent_ok(&self) -> bool {
        self.consent_at_ms > 0 && !self.consent_host.is_empty() && {
            let h = self.host();
            !h.is_empty() && self.consent_host.eq_ignore_ascii_case(&h)
        }
    }

    pub fn display_name(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let id_ok = (1..=32).contains(&self.id.len())
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok {
            return Err(format!(
                "cloud provider id {:?} must be 1-32 of a-z, 0-9, -",
                self.id
            ));
        }
        if !ADAPTERS.contains(&self.adapter.as_str()) {
            return Err(format!("unknown cloud adapter {:?}", self.adapter));
        }
        if !["", "whisper", "gpt_transcribe"].contains(&self.dialect.as_str()) {
            return Err(format!("unknown dialect {:?}", self.dialect));
        }
        if !["", "wav", "flac"].contains(&self.format_override.as_str()) {
            return Err(format!(
                "unknown format_override {:?}",
                self.format_override
            ));
        }
        let BaseUrl { https, host, .. } = parse_base_url(&self.base_url)?;
        // Names (`*.local`, single-label, `*.ts.net`) pass here; flow-cloud
        // resolves them before an http:// call and requires every address
        // to be private (S6).
        if !https && !net::classify(&host).plaintext_ok() {
            return Err("public base URL: https:// is required".into());
        }
        if self.model.trim().is_empty() {
            return Err("model must be set".into());
        }
        if !(1000..=60_000).contains(&self.timeout_base_ms) {
            return Err("timeout_base_ms must be 1000-60000".into());
        }
        if let Some(p) = self.price_per_hour_usd {
            if !(p.0.is_finite() && p.0 >= 0.0) {
                return Err("price must be a non-negative amount".into());
            }
        }
        Ok(())
    }
}

/// A provider base URL as the HTTP client will see it (review S2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrl {
    pub https: bool,
    /// Lower-case domain, or the IP literal (IPv6 without brackets), exactly
    /// as the `url` crate (reqwest's parser) resolves it: WHATWG IPv4 forms
    /// (`0x7f.1`, `2130706433`) become dotted quads, percent escapes are
    /// decoded, IDNA names become punycode.
    pub host: String,
    pub port: Option<u16>,
    /// `host` as an IP address, when it is one.
    pub ip: Option<std::net::IpAddr>,
}

/// Parse a cloud provider base URL with the `url` crate (the parser
/// reqwest uses, so the host checked here is the host contacted). Refuses
/// everything ambiguous: any `\` (WHATWG reads it as `/`), whitespace or
/// control characters anywhere (WHATWG silently strips tab/newline),
/// userinfo (`user@`, `:pw@`), schemes other than http/https, no host.
pub fn parse_base_url(raw: &str) -> Result<BaseUrl, String> {
    const BAD: &str = "base URL must be http(s)://host[:port][/path]";
    if raw.is_empty() {
        return Err(BAD.into());
    }
    if raw.contains('\\') {
        return Err("base URL must not contain a backslash".into());
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("base URL must not contain spaces or control characters".into());
    }
    let u = url::Url::parse(raw).map_err(|_| BAD.to_string())?;
    let https = match u.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(BAD.into()),
    };
    if !u.username().is_empty() || u.password().is_some() {
        return Err("base URL must not contain credentials (user@)".into());
    }
    let (host, ip) = match u.host() {
        Some(url::Host::Domain(d)) if !d.is_empty() => (d.to_ascii_lowercase(), None),
        Some(url::Host::Ipv4(a)) => (a.to_string(), Some(std::net::IpAddr::V4(a))),
        Some(url::Host::Ipv6(a)) => (a.to_string(), Some(std::net::IpAddr::V6(a))),
        _ => return Err("base URL has no host".into()),
    };
    Ok(BaseUrl {
        https,
        host,
        port: u.port(),
        ip,
    })
}

/// `(is_https, host)`; `None` when [`parse_base_url`] refuses the URL.
fn split_url(url: &str) -> Option<(bool, String)> {
    parse_base_url(url).ok().map(|b| (b.https, b.host))
}

/// Per-provider counters for one calendar month (UTC). Kept in memory by
/// the router; the shell persists them (`cloud_usage.json` / prefs) and
/// restores them with `Router::set_cloud_usage`. Never sent anywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudUsage {
    /// `"YYYY-MM"` (UTC).
    pub month: String,
    pub providers: Vec<ProviderUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderUsage {
    pub provider_id: String,
    /// Dictations that reached the provider (silence never does).
    pub requests: u64,
    pub billed_audio_ms: u64,
    pub cost_usd: Usd,
    /// Some of `cost_usd` is an estimate from the price table.
    pub cost_estimated: bool,
}

impl CloudUsage {
    pub fn total_cost(&self) -> f64 {
        self.providers.iter().map(|p| p.cost_usd.0).sum()
    }

    /// Start a new month when `month` differs (counters reset).
    pub(crate) fn roll(&mut self, month: &str) {
        if self.month != month {
            self.month = month.to_string();
            self.providers.clear();
        }
    }

    pub(crate) fn record(
        &mut self,
        provider_id: &str,
        billed_audio_ms: u32,
        cost: Option<Usd>,
        estimated: bool,
    ) {
        let i = match self
            .providers
            .iter()
            .position(|p| p.provider_id == provider_id)
        {
            Some(i) => i,
            None => {
                self.providers.push(ProviderUsage {
                    provider_id: provider_id.into(),
                    ..Default::default()
                });
                self.providers.len() - 1
            }
        };
        let p = &mut self.providers[i];
        p.requests += 1;
        p.billed_audio_ms += billed_audio_ms as u64;
        if let Some(c) = cost {
            p.cost_usd = Usd(p.cost_usd.0 + c.0);
        }
        p.cost_estimated |= estimated || cost.is_none();
    }
}

/// `"YYYY-MM"` of a Unix time in ms (UTC, proleptic Gregorian).
pub fn month_of(unix_ms: u64) -> String {
    let days = (unix_ms / 86_400_000) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> CloudProviderConfig {
        CloudProviderConfig {
            id: "openrouter-1".into(),
            preset: "openrouter".into(),
            adapter: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-transcribe".into(),
            ..Default::default()
        }
    }

    #[test]
    fn secret_never_prints() {
        let s = SecretString::new("sk-LEAKCHECK-123");
        assert_eq!(format!("{s:?} {s}"), "*** ***");
        assert_eq!(s.expose(), "sk-LEAKCHECK-123");
    }

    #[test]
    fn provider_validation() {
        assert_eq!(p().validate(), Ok(()));
        let bad = |f: &dyn Fn(&mut CloudProviderConfig)| {
            let mut c = p();
            f(&mut c);
            c.validate().unwrap_err()
        };
        assert!(bad(&|c| c.id = "Bad_Id".into()).contains("id"));
        assert!(bad(&|c| c.id = "x".repeat(33)).contains("id"));
        assert!(bad(&|c| c.adapter = "chat".into()).contains("adapter"));
        assert!(bad(&|c| c.base_url = "http://api.example.com/v1".into()).contains("https"));
        assert!(bad(&|c| c.base_url = "ftp://x".into()).contains("base URL"));
        assert!(bad(&|c| c.base_url = "https://u:p@x.com".into()).contains("base URL"));
        assert!(bad(&|c| c.model = " ".into()).contains("model"));
        assert!(bad(&|c| c.timeout_base_ms = 500).contains("timeout"));
        assert!(bad(&|c| c.dialect = "x".into()).contains("dialect"));
        // Self-hosted speaches on the LAN / tailnet may use http.
        let mut c = p();
        c.base_url = "http://100.64.0.10:8000/v1".into();
        assert_eq!(c.validate(), Ok(()));
        c.base_url = "http://[fd7a:115c:a1e0::1]:8000/v1".into();
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn consent_follows_the_host() {
        let mut c = p();
        assert!(!c.consent_ok());
        c.consent_host = "OpenRouter.ai".into();
        c.consent_at_ms = 1;
        assert!(c.consent_ok());
        c.base_url = "https://evil.example.com/v1".into();
        assert!(!c.consent_ok());
    }

    #[test]
    fn config_validation() {
        let mut c = CloudConfig {
            active: "openrouter-1".into(),
            providers: vec![p()],
            monthly_budget_usd: None,
        };
        assert_eq!(c.validate(), Ok(()));
        c.active = "nope".into();
        assert!(c.validate().unwrap_err().contains("active"));
        c.active.clear();
        c.providers.push(p());
        assert!(c.validate().unwrap_err().contains("duplicate"));
        c.providers = (0..9)
            .map(|i| CloudProviderConfig {
                id: format!("p{i}"),
                ..p()
            })
            .collect();
        assert!(c.validate().unwrap_err().contains("at most"));
    }

    #[test]
    fn month_of_unix_ms() {
        assert_eq!(month_of(0), "1970-01");
        assert_eq!(month_of(1_793_000_000_000), "2026-10"); // 2026-10-26
        assert_eq!(month_of(951_782_400_000), "2000-02"); // 2000-02-29
        assert_eq!(month_of(951_868_800_000), "2000-03");
    }

    #[test]
    fn usage_accumulates_and_rolls() {
        let mut u = CloudUsage::default();
        u.roll("2026-10");
        u.record("a", 3000, Some(Usd(0.001)), false);
        u.record("a", 1000, None, false);
        u.record("b", 500, Some(Usd(0.002)), true);
        assert_eq!(u.providers[0].requests, 2);
        assert_eq!(u.providers[0].billed_audio_ms, 4000);
        assert!(u.providers[0].cost_estimated);
        assert!((u.total_cost() - 0.003).abs() < 1e-12);
        u.roll("2026-10");
        assert_eq!(u.providers.len(), 2);
        u.roll("2026-11");
        assert!(u.providers.is_empty());
    }
}
