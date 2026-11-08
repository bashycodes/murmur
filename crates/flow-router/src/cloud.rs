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

/// Adapters compiled in (D28: OpenRouter is the only cloud provider).
pub const ADAPTERS: [&str; 1] = ["openrouter"];

/// Legacy (schema 1) limit on provider entries; read only by the migration.
pub const MAX_PROVIDERS: usize = 8;

/// `CloudConfig::schema` of a D28 config.
pub const CLOUD_SCHEMA: u32 = 2;
/// The single provider id (secure-storage key name).
pub const OPENROUTER_ID: &str = "openrouter";
/// Constant, not user-editable (D28 §3 host pinning).
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
/// The host every key and consent is bound to.
pub const OPENROUTER_HOST: &str = "openrouter.ai";
/// D28 §2.3 recommended defaults.
pub const DEFAULT_STT_MODEL: &str = "openai/whisper-large-v3-turbo";
pub const DEFAULT_CLEANUP_MODEL: &str = "meta-llama/llama-3.3-70b-instruct";
/// Cloud cleanup timeout default / range (ms, D28 §2.5).
pub const DEFAULT_CLEANUP_TIMEOUT_MS: u64 = 2500;
pub const CLEANUP_TIMEOUT_RANGE_MS: (u64, u64) = (800, 8000);
/// Transcription models with no Zero-Data-Retention endpoint in OpenRouter's
/// `/api/v1/endpoints/zdr` on 2026-10-24 (spec §1.1). Used once, by the
/// migration: a migrated model from this list is replaced by the default
/// while ZDR is on. Runtime checks use the live catalogue (flow-cloud).
pub const NO_ZDR_STT_MODELS_2026_10_24: [&str; 7] = [
    "openai/gpt-transcribe",
    "openai/gpt-4o-transcribe",
    "openai/gpt-4o-mini-transcribe",
    "openai/whisper-1",
    "google/gemini-3.5-transcribe",
    "x-ai/grok-stt-1.0",
    "meta/muse-voice-transcribe-1.0",
];

/// One step of `RouterConfig::cleanup_order` (D28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CleanupRoute {
    /// On-device LLM (when loaded).
    Local,
    /// OpenRouter chat completions.
    Cloud,
    /// The configured flow-server's LLM (D36, text-only `clean`), whatever
    /// route transcribed the dictation.
    Server,
}

/// `RouterConfig.cloud` (schema 2, D28 §2.6). Keys are never in it.
///
/// A config without `schema` (written before D28) deserialises with
/// `schema = 1` and its `active` / `providers`; `RouterConfig::migrate_d28`
/// turns it into schema 2 (`crate::migrate`). Those two legacy fields are
/// never written once empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudConfig {
    /// 2 = D28. Missing in JSON = 1 (legacy).
    #[serde(default = "legacy_schema")]
    pub schema: u32,
    pub openrouter: OpenRouterConfig,
    /// Stop using the cloud (STT and cleanup) for the month once this much
    /// was spent.
    pub monthly_budget_usd: Option<Usd>,
    /// D78: the one Cloud provider, OpenRouter or LiteLLM, that every cloud
    /// use goes to. Cleanup calls it; cloud speech-to-text runs only while
    /// it is OpenRouter (no LiteLLM speech path yet). Stored under its D52
    /// name `cleanup_provider`, so saved settings read unchanged; omitted
    /// while OpenRouter.
    #[serde(
        rename = "cleanup_provider",
        skip_serializing_if = "CloudProvider::is_openrouter"
    )]
    pub provider: CloudProvider,
    /// D52/D78: the LiteLLM proxy (or any OpenAI-compatible server) used
    /// while [`Self::provider`] is LiteLLM. Omitted while default.
    #[serde(skip_serializing_if = "CustomCleanupConfig::is_default")]
    pub custom_cleanup: CustomCleanupConfig,
    /// Legacy (schema 1): active provider id. Read by the migration only.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub active: String,
    /// Legacy (schema 1): provider entries. Read by the migration only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<CloudProviderConfig>,
}

fn legacy_schema() -> u32 {
    1
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            schema: CLOUD_SCHEMA,
            openrouter: OpenRouterConfig::default(),
            monthly_budget_usd: None,
            provider: CloudProvider::OpenRouter,
            custom_cleanup: CustomCleanupConfig::default(),
            active: String::new(),
            providers: Vec::new(),
        }
    }
}

impl CloudConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Needs `migrate_d28` (schema < 2 or legacy fields present).
    pub fn is_legacy(&self) -> bool {
        self.schema < CLOUD_SCHEMA || !self.active.is_empty() || !self.providers.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.is_legacy() {
            // Migrated (in memory) before use; nothing here is routed.
            return Ok(());
        }
        if self.schema != CLOUD_SCHEMA {
            return Err(format!("unknown cloud schema {}", self.schema));
        }
        self.openrouter.validate()?;
        if self.provider == CloudProvider::LiteLlm {
            self.custom_cleanup.validate()?;
        }
        if let Some(b) = self.monthly_budget_usd {
            if !(b.0.is_finite() && b.0 >= 0.0) {
                return Err("monthly budget must be a non-negative amount".into());
            }
        }
        Ok(())
    }

    /// The cleanup consent for the selected provider: openrouter.ai, or
    /// the host of the custom server's current base URL (D52).
    pub fn cleanup_consent_ok(&self) -> bool {
        match self.provider {
            CloudProvider::OpenRouter => self.openrouter.cleanup.consent_ok(),
            CloudProvider::LiteLlm => self.custom_cleanup.consent_ok(),
        }
    }

    /// Model id the selected cleanup provider is asked for.
    pub fn cleanup_model(&self) -> &str {
        match self.provider {
            CloudProvider::OpenRouter => &self.openrouter.cleanup.model,
            CloudProvider::LiteLlm => &self.custom_cleanup.model,
        }
    }

    /// Hard cleanup timeout of the selected provider (ms).
    pub fn cleanup_timeout_ms(&self) -> u64 {
        match self.provider {
            CloudProvider::OpenRouter => self.openrouter.cleanup.timeout_ms,
            CloudProvider::LiteLlm => self.custom_cleanup.timeout_ms,
        }
    }

    /// What a cloud cleanup prompt may contain. One set of switches for
    /// both providers (stored in `openrouter.cleanup`, its pre-D52 home).
    pub fn cleanup_scope(&self) -> flow_core::cleanup::PromptScope {
        self.openrouter.cleanup.scope()
    }

    /// The secure-storage / `set_cloud_key` id of the cleanup provider's key.
    pub fn cleanup_key_id(&self) -> &'static str {
        match self.provider {
            CloudProvider::OpenRouter => OPENROUTER_ID,
            CloudProvider::LiteLlm => CUSTOM_CLEANUP_ID,
        }
    }

    /// The selected provider has what a call needs besides consent and
    /// key: always for OpenRouter, address and model for a custom server
    /// (D52: Custom is saved when picked, before they are set).
    pub fn cleanup_ready(&self) -> bool {
        match self.provider {
            CloudProvider::OpenRouter => true,
            CloudProvider::LiteLlm => self.custom_cleanup.ready(),
        }
    }

    /// OpenRouter needs a key; a custom server may run without one (D52).
    pub fn cleanup_needs_key(&self) -> bool {
        self.provider.is_openrouter()
    }

    /// The monthly budget gates OpenRouter cleanup only; the custom server
    /// is the owner's own (D52).
    pub fn cleanup_budgeted(&self) -> bool {
        self.provider.is_openrouter()
    }

    /// The custom server's host when it is the cleanup provider, else `""`
    /// (the status page names the server it sends text to).
    pub fn cleanup_host(&self) -> String {
        match self.provider {
            CloudProvider::OpenRouter => String::new(),
            CloudProvider::LiteLlm => self.custom_cleanup.host(),
        }
    }

    /// The part of the config the cleanup gate depends on (a change resets
    /// its circuit and rebuilds the cleaner; STT edits do not).
    pub fn cleanup_view(&self) -> CleanupView {
        CleanupView {
            provider: self.provider,
            privacy: self.openrouter.privacy.clone(),
            openrouter: self.openrouter.cleanup.clone(),
            custom: self.custom_cleanup.clone(),
        }
    }
}

/// [`CloudConfig::cleanup_view`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupView {
    pub provider: CloudProvider,
    pub privacy: Privacy,
    pub openrouter: CleanupConfig,
    pub custom: CustomCleanupConfig,
}

/// The Cloud provider (D78; D52's cleanup provider): who the `cloud`
/// cleanup route calls. The Windows pages call both "Cloud" and keep one
/// provider for every cloud use; cloud speech-to-text itself always runs on
/// OpenRouter, whatever this says (Android still pairs the two).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CloudProvider {
    /// OpenRouter (default; D28): speech to text and cleanup.
    #[default]
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// The user's LiteLLM proxy or other OpenAI-compatible server
    /// ([`CustomCleanupConfig`], D52): cleanup only. On the wire it keeps
    /// its D52 id `custom` (settings files, key id, usage rows, shell
    /// JSON); `litellm` is read too.
    #[serde(rename = "custom", alias = "litellm")]
    LiteLlm,
}

impl CloudProvider {
    pub fn is_openrouter(&self) -> bool {
        *self == CloudProvider::OpenRouter
    }

    /// Cloud speech-to-text can run on this provider (OpenRouter only).
    /// The router does not enforce it (Android pairs LiteLLM cleanup with
    /// OpenRouter speech); the Windows settings do (D78).
    pub fn offers_stt(self) -> bool {
        self.is_openrouter()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CloudProvider::OpenRouter => "openrouter",
            CloudProvider::LiteLlm => "custom",
        }
    }
}

/// `set_cloud_key` id and usage-counter id of the custom cleanup server.
pub const CUSTOM_CLEANUP_ID: &str = "custom";
/// Custom server cleanup timeout default / range (ms): a self-hosted model
/// may be slower than OpenRouter's fastest provider.
pub const DEFAULT_CUSTOM_CLEANUP_TIMEOUT_MS: u64 = 4000;
pub const CUSTOM_CLEANUP_TIMEOUT_RANGE_MS: (u64, u64) = (800, 30_000);

/// D52: cleanup through the user's own OpenAI-compatible server (LiteLLM
/// proxy, vLLM, Ollama, …). Plain `http://` is allowed to any host by the
/// owner's choice: the key and the dictated text then cross the network
/// unencrypted. The key, optional, lives in secure storage under
/// [`CUSTOM_CLEANUP_ID`], never here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomCleanupConfig {
    /// As typed: `http(s)://host[:port][/path]`; see [`normalize_custom_base_url`].
    pub base_url: String,
    /// Free text (whatever the server's `/v1/models` lists).
    pub model: String,
    pub timeout_ms: u64,
    pub connect_timeout_ms: u64,
    /// Host the user consented to send dictated text to; changing the base
    /// URL to another host voids the consent.
    pub consent_host: String,
    pub consent_at_ms: u64,
}

impl Default for CustomCleanupConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: String::new(),
            timeout_ms: DEFAULT_CUSTOM_CLEANUP_TIMEOUT_MS,
            connect_timeout_ms: 1500,
            consent_host: String::new(),
            consent_at_ms: 0,
        }
    }
}

impl CustomCleanupConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Canonical host of `base_url` ([`custom_server_address`]), or `""`
    /// when the URL is unusable.
    pub fn host(&self) -> String {
        custom_server_address(&self.base_url)
            .map(|a| a.host)
            .unwrap_or_default()
    }

    /// The user consented to send dictated text to this server's host.
    pub fn consent_ok(&self) -> bool {
        self.consent_at_ms > 0 && !self.consent_host.is_empty() && {
            let h = self.host();
            !h.is_empty() && self.consent_host.eq_ignore_ascii_case(&h)
        }
    }

    /// The API root requests go to (`…/chat/completions`, `…/models`).
    pub fn api_base(&self) -> Result<String, String> {
        normalize_custom_base_url(&self.base_url)
    }

    /// Whether the saved key may be sent to `base_url` (the settings
    /// page's "Load models" / "Test connection" with an empty key field):
    /// only when that URL's host is the host the user consented to. A key
    /// typed into the field may go anywhere the user points it.
    pub fn saved_key_allowed_for(&self, base_url: &str) -> bool {
        self.consent_at_ms > 0
            && !self.consent_host.is_empty()
            && custom_server_address(base_url)
                .is_ok_and(|a| a.host.eq_ignore_ascii_case(&self.consent_host))
    }

    /// Address and model are set: the server can be called. A selected
    /// custom server without them yet is saved but not ready, so cloud
    /// cleanup is skipped like an unconfigured engine.
    pub fn ready(&self) -> bool {
        self.api_base().is_ok() && valid_custom_model_id(self.model.trim())
    }

    /// An empty address or model is allowed: the page saves Custom as soon
    /// as it is picked, the rest is filled in later ([`Self::ready`]).
    pub fn validate(&self) -> Result<(), String> {
        if !self.base_url.trim().is_empty() {
            self.api_base()?;
        }
        if !self.model.trim().is_empty() && !valid_custom_model_id(self.model.trim()) {
            return Err(format!("invalid custom cleanup model id {:?}", self.model));
        }
        let (lo, hi) = CUSTOM_CLEANUP_TIMEOUT_RANGE_MS;
        if !(lo..=hi).contains(&self.timeout_ms) {
            return Err(format!("custom cleanup timeout_ms must be {lo}-{hi}"));
        }
        if !(200..=10_000).contains(&self.connect_timeout_ms) {
            return Err("custom cleanup connect_timeout_ms must be 200-10000".into());
        }
        Ok(())
    }
}

/// A custom server's API root: [`parse_base_url`]'s checks (http or https,
/// any host; no credentials, spaces or backslashes), no query or fragment,
/// no trailing slash, and `/v1` added when the URL has no path (LiteLLM
/// and most OpenAI-compatible servers serve the API there). A URL with a
/// path is kept as typed (`…/v1`, `…/api/v1`).
pub fn normalize_custom_base_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    parse_base_url(raw)?;
    let mut u = url::Url::parse(raw).map_err(|_| "base URL does not parse".to_string())?;
    if u.query().is_some() || u.fragment().is_some() {
        return Err("base URL must not contain ? or #".into());
    }
    if u.path().trim_end_matches('/').is_empty() {
        u.set_path("/v1");
    }
    Ok(u.as_str().trim_end_matches('/').to_string())
}

/// A custom server address as every shell shows and stores it: the
/// normalised API root and the canonical host (lower case, IPv6
/// compressed, IDN in punycode) that consent is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomServerAddress {
    pub url: String,
    pub host: String,
}

/// [`normalize_custom_base_url`] plus the canonical host.
pub fn custom_server_address(raw: &str) -> Result<CustomServerAddress, String> {
    let url = normalize_custom_base_url(raw)?;
    let host = parse_base_url(&url)?.host;
    Ok(CustomServerAddress { url, host })
}

/// Custom server model ids are free text (`gpt-4o`, `Qwen/Qwen2.5-7B`,
/// `ollama/llama3:8b`): 1-256 characters, no whitespace or control
/// characters.
pub fn valid_custom_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Request privacy, shared by both uses (D28 §2.6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    /// `provider.zdr` (zero-retention endpoints only). Default on.
    pub zdr: bool,
    /// `provider.data_collection`: `"deny"` (default) or `"allow"`.
    pub data_collection: String,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            zdr: true,
            data_collection: "deny".into(),
        }
    }
}

impl Privacy {
    pub fn data_collection_deny(&self) -> bool {
        self.data_collection != "allow"
    }
}

/// Cloud speech-to-text use (D28 §2.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    pub model: String,
    /// `"auto"` (from the catalogue), `"transcriptions"`, `"chat_audio"`.
    pub stt_path: String,
    /// `""` = follow `SessionConfig.languages`.
    pub language: String,
    pub send_dictionary: bool,
    /// `""` = adapter default, `"wav"`, `"flac"`.
    pub format_override: String,
    pub timeout_base_ms: u64,
    /// Host the user consented to send audio to (consent dialog).
    pub consent_host: String,
    pub consent_at_ms: u64,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_STT_MODEL.into(),
            stt_path: "auto".into(),
            language: String::new(),
            send_dictionary: true,
            format_override: String::new(),
            timeout_base_ms: 6000,
            consent_host: String::new(),
            consent_at_ms: 0,
        }
    }
}

/// Cloud text cleanup use (D28 §2.5). Its consent is separate from STT's.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupConfig {
    pub model: String,
    /// `provider.sort`: `latency` | `throughput` | `price` | `""` (load balancing).
    pub sort: String,
    /// Hard cleanup timeout (800-8000 ms); the commit then uses the
    /// deterministic text.
    pub timeout_ms: u64,
    pub connect_timeout_ms: u64,
    /// Prompt scope toggles (all off by default: only the transcript,
    /// category/style and dictionary leave the device).
    pub send_surrounding_text: bool,
    pub send_screen_words: bool,
    pub send_app_name: bool,
    /// Host the user consented to send dictated text to.
    pub consent_host: String,
    pub consent_at_ms: u64,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_CLEANUP_MODEL.into(),
            sort: "latency".into(),
            timeout_ms: DEFAULT_CLEANUP_TIMEOUT_MS,
            connect_timeout_ms: 1500,
            send_surrounding_text: false,
            send_screen_words: false,
            send_app_name: false,
            consent_host: String::new(),
            consent_at_ms: 0,
        }
    }
}

fn consent_ok(host: &str, at_ms: u64) -> bool {
    at_ms > 0 && host.eq_ignore_ascii_case(OPENROUTER_HOST)
}

/// `^[a-z0-9._~/-]+(:[a-z]+)*$`, at most 128 chars (spec §2.2).
pub fn valid_model_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    let mut parts = id.split(':');
    let head = parts.next().unwrap_or("");
    !head.is_empty()
        && head
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._~/-".contains(&b))
        && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()))
}

impl SttConfig {
    /// The user consented to send audio to openrouter.ai.
    pub fn consent_ok(&self) -> bool {
        consent_ok(&self.consent_host, self.consent_at_ms)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_model_id(&self.model) {
            return Err(format!("invalid STT model id {:?}", self.model));
        }
        if !["auto", "transcriptions", "chat_audio"].contains(&self.stt_path.as_str()) {
            return Err(format!("unknown stt_path {:?}", self.stt_path));
        }
        if !["", "wav", "flac"].contains(&self.format_override.as_str()) {
            return Err(format!(
                "unknown format_override {:?}",
                self.format_override
            ));
        }
        if !(1000..=60_000).contains(&self.timeout_base_ms) {
            return Err("timeout_base_ms must be 1000-60000".into());
        }
        Ok(())
    }
}

impl CleanupConfig {
    /// The user consented to send dictated text to openrouter.ai.
    pub fn consent_ok(&self) -> bool {
        consent_ok(&self.consent_host, self.consent_at_ms)
    }

    pub fn scope(&self) -> flow_core::cleanup::PromptScope {
        flow_core::cleanup::PromptScope {
            surrounding_text: self.send_surrounding_text,
            screen_words: self.send_screen_words,
            app_id: self.send_app_name,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_model_id(&self.model) {
            return Err(format!("invalid cleanup model id {:?}", self.model));
        }
        if !["", "latency", "throughput", "price"].contains(&self.sort.as_str()) {
            return Err(format!("unknown sort {:?}", self.sort));
        }
        let (lo, hi) = CLEANUP_TIMEOUT_RANGE_MS;
        if !(lo..=hi).contains(&self.timeout_ms) {
            return Err(format!("cleanup timeout_ms must be {lo}-{hi}"));
        }
        if !(200..=10_000).contains(&self.connect_timeout_ms) {
            return Err("cleanup connect_timeout_ms must be 200-10000".into());
        }
        Ok(())
    }
}

/// The single OpenRouter account entry with its two uses (D28 §2.6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRouterConfig {
    pub privacy: Privacy,
    pub stt: SttConfig,
    pub cleanup: CleanupConfig,
}

impl OpenRouterConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !["deny", "allow"].contains(&self.privacy.data_collection.as_str()) {
            return Err(format!(
                "data_collection must be deny or allow, not {:?}",
                self.privacy.data_collection
            ));
        }
        self.stt.validate()?;
        self.cleanup.validate()
    }
}

/// Legacy (schema 1, D18) provider entry: read only by `migrate_d28`.
/// `preset` was informational (UI); `adapter` picked the wire format.
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
        if !LEGACY_ADAPTERS.contains(&self.adapter.as_str()) {
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

/// Adapters a schema-1 provider entry may name (migration input only).
pub const LEGACY_ADAPTERS: [&str; 5] = [
    "openai_compat",
    "openrouter",
    "elevenlabs",
    "deepgram",
    "assemblyai",
];

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
    /// D28: `"stt"` (also `""`, older counters) or `"cleanup"`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    /// Dictations that reached the provider (silence never does).
    pub requests: u64,
    pub billed_audio_ms: u64,
    pub cost_usd: Usd,
    /// Some of `cost_usd` is an estimate from the price table.
    pub cost_estimated: bool,
}

/// `ProviderUsage::purpose` values (D28 usage split).
pub const PURPOSE_STT: &str = "stt";
pub const PURPOSE_CLEANUP: &str = "cleanup";
/// Counter id that keeps the cost of providers removed by `migrate_d28`
/// (the month's budget stays honest).
pub const REMOVED_PROVIDERS_ID: &str = "removed";

impl CloudUsage {
    /// Both uses, every provider, the custom cleanup server included.
    pub fn total_cost(&self) -> f64 {
        self.providers.iter().map(|p| p.cost_usd.0).sum()
    }

    /// What the monthly budget counts (D52): OpenRouter spend only (STT and
    /// OpenRouter cleanup, plus removed providers' cost). The custom
    /// cleanup server is the owner's own and never counts against it.
    pub fn budget_cost(&self) -> f64 {
        self.providers
            .iter()
            .filter(|p| p.provider_id != CUSTOM_CLEANUP_ID)
            .map(|p| p.cost_usd.0)
            .sum()
    }

    /// Cost the custom cleanup server reported this month (its
    /// `x-litellm-response-cost` header), shown apart from the budget.
    pub fn custom_cost(&self) -> f64 {
        self.providers
            .iter()
            .filter(|p| p.provider_id == CUSTOM_CLEANUP_ID)
            .map(|p| p.cost_usd.0)
            .sum()
    }

    /// Cost of one use ("Speech $x · Cleanup $y").
    pub fn cost_for(&self, purpose: &str) -> f64 {
        self.providers
            .iter()
            .filter(|p| p.purpose == purpose || (purpose == PURPOSE_STT && p.purpose.is_empty()))
            .map(|p| p.cost_usd.0)
            .sum()
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
        self.record_for(provider_id, PURPOSE_STT, billed_audio_ms, cost, estimated)
    }

    pub(crate) fn record_for(
        &mut self,
        provider_id: &str,
        purpose: &str,
        billed_audio_ms: u32,
        cost: Option<Usd>,
        estimated: bool,
    ) {
        let same = |p: &ProviderUsage| {
            p.provider_id == provider_id
                && (p.purpose == purpose || (purpose == PURPOSE_STT && p.purpose.is_empty()))
        };
        let i = match self.providers.iter().position(same) {
            Some(i) => i,
            None => {
                self.providers.push(ProviderUsage {
                    provider_id: provider_id.into(),
                    purpose: purpose.into(),
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
        // Legacy entries of every old adapter still parse for the migration.
        for a in LEGACY_ADAPTERS {
            let mut c = p();
            c.adapter = a.into();
            assert_eq!(c.validate(), Ok(()), "{a}");
        }
        assert!(bad(&|c| c.base_url = "http://api.example.com/v1".into()).contains("https"));
        assert!(bad(&|c| c.base_url = "ftp://x".into()).contains("base URL"));
        assert!(bad(&|c| c.base_url = "https://u:p@x.com".into()).contains("base URL"));
        assert!(bad(&|c| c.model = " ".into()).contains("model"));
        assert!(bad(&|c| c.timeout_base_ms = 500).contains("timeout"));
        assert!(bad(&|c| c.dialect = "x".into()).contains("dialect"));
        // Self-hosted speaches on the LAN / a VPN may use http.
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
    fn only_openrouter_is_compiled_in() {
        assert_eq!(ADAPTERS, ["openrouter"]);
        assert_eq!(OPENROUTER_BASE_URL, "https://openrouter.ai/api/v1");
        assert_eq!(
            parse_base_url(OPENROUTER_BASE_URL).unwrap().host,
            OPENROUTER_HOST
        );
    }

    #[test]
    fn schema2_defaults_match_the_spec() {
        let c = CloudConfig::default();
        assert_eq!(c.schema, 2);
        assert!(!c.is_legacy());
        assert_eq!(c.validate(), Ok(()));
        let o = &c.openrouter;
        assert!(o.privacy.zdr && o.privacy.data_collection_deny());
        assert_eq!(o.stt.model, DEFAULT_STT_MODEL);
        assert_eq!(o.cleanup.model, DEFAULT_CLEANUP_MODEL);
        assert_eq!(o.cleanup.timeout_ms, 2500);
        assert_eq!(o.cleanup.scope(), flow_core::cleanup::PromptScope::MINIMAL);
        assert!(!o.stt.consent_ok() && !o.cleanup.consent_ok());
        // Spec §2.6 JSON shape.
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(j["schema"], 2);
        assert_eq!(j["openrouter"]["privacy"]["data_collection"], "deny");
        assert_eq!(j["openrouter"]["stt"]["stt_path"], "auto");
        assert_eq!(j["openrouter"]["cleanup"]["sort"], "latency");
        assert!(j.get("providers").is_none() && j.get("active").is_none());
    }

    #[test]
    fn json_without_schema_is_legacy() {
        let c: CloudConfig = serde_json::from_str(r#"{"active":"x","providers":[]}"#).unwrap();
        assert_eq!(c.schema, 1);
        assert!(c.is_legacy());
        let c: CloudConfig = serde_json::from_str(r#"{"schema":2}"#).unwrap();
        assert!(!c.is_legacy());
    }

    #[test]
    fn schema2_validation() {
        let bad = |f: &dyn Fn(&mut OpenRouterConfig)| {
            let mut c = CloudConfig::default();
            f(&mut c.openrouter);
            c.validate().unwrap_err()
        };
        assert!(bad(&|o| o.stt.model = "Bad Model".into()).contains("model"));
        assert!(bad(&|o| o.cleanup.model = String::new()).contains("model"));
        assert!(bad(&|o| o.stt.stt_path = "x".into()).contains("stt_path"));
        assert!(bad(&|o| o.cleanup.timeout_ms = 700).contains("timeout"));
        assert!(bad(&|o| o.cleanup.timeout_ms = 9000).contains("timeout"));
        assert!(bad(&|o| o.cleanup.sort = "random".into()).contains("sort"));
        assert!(bad(&|o| o.privacy.data_collection = "maybe".into()).contains("data_collection"));
        assert!(valid_model_id("openai/whisper-large-v3-turbo"));
        assert!(valid_model_id("meta-llama/llama-3.1-8b-instruct:nitro"));
        assert!(!valid_model_id("x:"));
        assert!(!valid_model_id(&"a".repeat(129)));
    }

    #[test]
    fn consent_is_per_use_and_bound_to_openrouter() {
        let mut o = OpenRouterConfig::default();
        o.stt.consent_host = "OpenRouter.ai".into();
        o.stt.consent_at_ms = 1;
        assert!(o.stt.consent_ok());
        assert!(
            !o.cleanup.consent_ok(),
            "STT consent must not grant cleanup"
        );
        o.cleanup.consent_host = "api.example.com".into();
        o.cleanup.consent_at_ms = 1;
        assert!(!o.cleanup.consent_ok());
    }

    #[test]
    fn custom_base_url_is_normalised() {
        let n = |s: &str| normalize_custom_base_url(s);
        assert_eq!(
            n("http://10.0.0.5:4000").unwrap(),
            "http://10.0.0.5:4000/v1"
        );
        assert_eq!(
            n(" http://10.0.0.5:4000/ ").unwrap(),
            "http://10.0.0.5:4000/v1"
        );
        assert_eq!(
            n("http://10.0.0.5:4000/v1/").unwrap(),
            "http://10.0.0.5:4000/v1"
        );
        assert_eq!(
            n("https://LLM.Example.org/api/v1").unwrap(),
            "https://llm.example.org/api/v1"
        );
        // Plain http to a public name is the owner's choice (D52).
        assert_eq!(
            n("http://llm.example.org:4000").unwrap(),
            "http://llm.example.org:4000/v1"
        );
        for bad in [
            "",
            "10.0.0.5:4000",
            "ftp://x",
            "http://u:p@x:4000",
            "http://x:4000/v1?k=1",
            "http://x/v1#f",
            "http://x /v1",
        ] {
            assert!(n(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn custom_server_address_has_the_canonical_host() {
        let a = custom_server_address("http://[2001:DB8:0:0::1]:4000").unwrap();
        assert_eq!(a.url, "http://[2001:db8::1]:4000/v1");
        assert_eq!(a.host, "2001:db8::1");
        let a = custom_server_address("https://LLM.Example.org/api/v1/").unwrap();
        assert_eq!(
            (a.url.as_str(), a.host.as_str()),
            ("https://llm.example.org/api/v1", "llm.example.org")
        );
        let a = custom_server_address("http://bücher.example:4000").unwrap();
        assert_eq!(a.host, "xn--bcher-kva.example");
        assert!(custom_server_address("http://x/v1?q").is_err());
    }

    #[test]
    fn saved_custom_key_goes_only_to_the_consented_host() {
        let c = CustomCleanupConfig {
            base_url: "http://10.0.0.5:4000".into(),
            consent_host: "10.0.0.5".into(),
            consent_at_ms: 1,
            ..Default::default()
        };
        assert!(c.saved_key_allowed_for("http://10.0.0.5:4000/v1"));
        assert!(c.saved_key_allowed_for(" http://10.0.0.5:8080 "));
        assert!(!c.saved_key_allowed_for("http://10.0.0.6:4000"));
        assert!(!c.saved_key_allowed_for("https://evil.example/10.0.0.5"));
        assert!(!c.saved_key_allowed_for("not a url"));
        let unconsented = CustomCleanupConfig {
            consent_at_ms: 0,
            ..c.clone()
        };
        assert!(!unconsented.saved_key_allowed_for("http://10.0.0.5:4000"));
        let no_host = CustomCleanupConfig {
            consent_host: String::new(),
            ..c
        };
        assert!(!no_host.saved_key_allowed_for("http://10.0.0.5:4000"));
    }

    #[test]
    fn custom_model_ids_are_loose() {
        for ok in [
            "gpt-4o",
            "Qwen/Qwen2.5-7B-Instruct",
            "ollama/llama3:8b",
            "A_b.c",
        ] {
            assert!(valid_custom_model_id(ok), "{ok}");
        }
        for bad in ["", "a b", "a\tb", "a\nb", &"a".repeat(257)] {
            assert!(!valid_custom_model_id(bad), "{bad:?}");
        }
    }

    /// D78: settings saved before "Cloud" (D52's `cleanup_provider` /
    /// `custom`, or none = OpenRouter) read as the same provider and are
    /// written back byte-for-byte, so an OpenRouter user's file is unchanged
    /// and a D52 custom server becomes the LiteLLM provider.
    #[test]
    fn pre_cloud_settings_read_and_write_unchanged() {
        let or_json = serde_json::json!({
            "schema": 2,
            "openrouter": serde_json::to_value(OpenRouterConfig::default()).unwrap(),
            "monthly_budget_usd": 5.0,
        });
        let c: CloudConfig = serde_json::from_value(or_json.clone()).unwrap();
        assert_eq!(c.provider, CloudProvider::OpenRouter);
        assert!(c.provider.offers_stt());
        assert_eq!(serde_json::to_value(&c).unwrap(), or_json);

        let d52 = serde_json::json!({
            "schema": 2,
            "openrouter": serde_json::to_value(OpenRouterConfig::default()).unwrap(),
            "monthly_budget_usd": null,
            "cleanup_provider": "custom",
            "custom_cleanup": {
                "base_url": "http://10.0.0.5:4000/v1", "model": "local-cleanup",
                "timeout_ms": 4000, "connect_timeout_ms": 1500,
                "consent_host": "10.0.0.5", "consent_at_ms": 7
            },
        });
        let c: CloudConfig = serde_json::from_value(d52.clone()).unwrap();
        assert_eq!(c.provider, CloudProvider::LiteLlm);
        assert!(!c.provider.offers_stt());
        assert_eq!(
            c.cleanup_key_id(),
            CUSTOM_CLEANUP_ID,
            "the saved key keeps its id"
        );
        assert!(c.cleanup_consent_ok());
        assert_eq!(serde_json::to_value(&c).unwrap(), d52);

        let named: CloudConfig =
            serde_json::from_str(r#"{"schema":2,"cleanup_provider":"litellm"}"#).unwrap();
        assert_eq!(named.provider, CloudProvider::LiteLlm);
    }

    #[test]
    fn custom_cleanup_config() {
        let mut c = CloudConfig::default();
        assert_eq!(c.provider, CloudProvider::OpenRouter);
        assert_eq!(c.cleanup_key_id(), OPENROUTER_ID);
        // Not serialised while default: older readers see the same JSON.
        let j = serde_json::to_value(&c).unwrap();
        assert!(j.get("cleanup_provider").is_none() && j.get("custom_cleanup").is_none());
        // Selecting Custom validates what is set; an empty address or model
        // is saved but not ready (the page saves Custom when picked).
        c.provider = CloudProvider::LiteLlm;
        assert_eq!(c.validate(), Ok(()));
        assert!(!c.cleanup_ready());
        c.custom_cleanup.base_url = "ftp://10.0.0.5".into();
        assert!(c.validate().is_err());
        c.custom_cleanup.base_url = "http://10.0.0.5:4000".into();
        assert_eq!(c.validate(), Ok(()));
        assert!(!c.cleanup_ready());
        c.custom_cleanup.model = "two words".into();
        assert!(c.validate().unwrap_err().contains("model"));
        c.custom_cleanup.model = "Qwen/Qwen2.5-7B".into();
        assert_eq!(c.validate(), Ok(()));
        assert!(c.cleanup_ready());
        c.custom_cleanup.timeout_ms = 40_000;
        assert!(c.validate().unwrap_err().contains("timeout"));
        c.custom_cleanup.timeout_ms = 4000;
        assert_eq!(c.cleanup_model(), "Qwen/Qwen2.5-7B");
        assert_eq!(c.cleanup_timeout_ms(), 4000);
        assert_eq!(c.cleanup_key_id(), CUSTOM_CLEANUP_ID);
        // Consent is bound to the typed host, not openrouter.ai.
        assert!(!c.cleanup_consent_ok());
        c.openrouter.cleanup.consent_host = OPENROUTER_HOST.into();
        c.openrouter.cleanup.consent_at_ms = 1;
        assert!(
            !c.cleanup_consent_ok(),
            "OpenRouter consent is not custom consent"
        );
        c.custom_cleanup.consent_host = "10.0.0.5".into();
        c.custom_cleanup.consent_at_ms = 1;
        assert!(c.cleanup_consent_ok());
        c.custom_cleanup.base_url = "http://10.0.0.6:4000".into();
        assert!(!c.cleanup_consent_ok(), "another host voids the consent");
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(j["cleanup_provider"], "custom");
        assert_eq!(j["custom_cleanup"]["model"], "Qwen/Qwen2.5-7B");
        let back: CloudConfig = serde_json::from_value(j).unwrap();
        assert_eq!(back, c);
        // A custom entry kept while OpenRouter is selected is not checked.
        let mut o = CloudConfig::default();
        o.custom_cleanup.base_url = "not a url".into();
        assert_eq!(o.validate(), Ok(()));
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
        u.record_for("a", PURPOSE_CLEANUP, 0, Some(Usd(0.01)), false);
        assert!((u.cost_for(PURPOSE_CLEANUP) - 0.01).abs() < 1e-12);
        assert!((u.cost_for(PURPOSE_STT) - 0.003).abs() < 1e-12);
        u.providers.pop();
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
