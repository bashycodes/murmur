//! `/etc/flow-server/config.toml` (docs/hybrid-architecture.md §3.2).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use flow_core::config::ModelConfig;
use serde::Deserialize;

/// `[model] llm_draft` when the config sets none (D69): the cleanup
/// model guesses up to this many tokens ahead from the transcript and
/// checks them in one decode step. On the home server's 4-core CPU this
/// halves the decode time of a long dictation; 0 in the config turns it off.
pub const DEFAULT_LLM_DRAFT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    Token,
    /// Only allowed when every bind address is loopback.
    None,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
    /// Print transcripts at debug level. Off by default; startup warns.
    pub log_text: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            log_text: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bind: Vec<String>,
    /// The old single token (D38: imported once as the admin key `owner`;
    /// regenerating the file moves that key to the new secret).
    pub token_file: Option<PathBuf>,
    pub auth: AuthMode,
    /// In-process use (tests): the token itself instead of `token_file`.
    /// Without `state_dir`, the keys then live only in memory.
    #[serde(skip)]
    pub token: Option<String>,
    /// Where `keys.json` and `key-usage.json` live (D38). Default:
    /// [`crate::capture::state_dir`] (systemd's `$STATE_DIRECTORY`, else
    /// the XDG state dir); the CLI falls back to `/var/lib/flow-server`
    /// for a config under `/etc` (see `main.rs`).
    pub state_dir: Option<PathBuf>,
    pub workers: usize,
    pub max_queue: usize,
    pub queue_timeout_ms: u64,
    pub idle_timeout_s: u64,
    pub max_session_s: u64,
    /// TLS handshake and HTTP request-header deadline (S7): a peer that
    /// has not finished its upgrade request by then is dropped.
    pub handshake_timeout_ms: u64,
    /// Open TCP connections, total and per peer IP (S7).
    pub max_connections: usize,
    pub max_connections_per_ip: usize,
    /// WebSocket ping interval, and how long a connection may stay silent
    /// (not even a pong) before it is dropped with its session (C5;
    /// protocol.md §1: 15 s / 45 s).
    pub ws_ping_ms: u64,
    pub ws_dead_ms: u64,
    /// LLM cleanup budget per character of the transcript, added to the
    /// session's `llm_timeout_ms` when the client sets no rate of its own
    /// (`SessionConfig::llm_budget_ms`): a long or Hebrew dictation on a
    /// CPU box needs more than a fixed 3 s. 0 = off (fixed budget).
    pub llm_timeout_per_char_ms: u32,
    /// Cap on that scaled budget (≤ the 120 s server limit).
    pub llm_timeout_max_ms: u64,
    pub model: ModelConfig,
    pub tls: Option<TlsConfig>,
    pub log: LogConfig,
    /// `[debug]`: opt-in capture of received audio + results (beta).
    pub debug: crate::capture::DebugConfig,
    /// `[library]`: model files served to clients (D40).
    pub library: crate::library::LibraryConfig,
    /// `--insecure-bind-any`: allow a wildcard or public bind without TLS
    /// and accept peers outside loopback/LAN/VPN. Command line only.
    #[serde(skip)]
    pub insecure_bind_any: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: vec![format!("127.0.0.1:{}", flow_protocol::DEFAULT_PORT)],
            token_file: None,
            auth: AuthMode::Token,
            token: None,
            state_dir: None,
            workers: 1,
            max_queue: 2,
            queue_timeout_ms: 30_000,
            idle_timeout_s: 120,
            max_session_s: 1200,
            handshake_timeout_ms: 10_000,
            max_connections: 64,
            max_connections_per_ip: 16,
            ws_ping_ms: flow_protocol::WS_PING_S * 1000,
            ws_dead_ms: flow_protocol::WS_DEAD_S * 1000,
            llm_timeout_per_char_ms: 15,
            llm_timeout_max_ms: flow_core::config::DEFAULT_LLM_TIMEOUT_MAX_MS,
            model: ModelConfig {
                llm_draft: Some(DEFAULT_LLM_DRAFT),
                ..ModelConfig::default()
            },
            tls: None,
            log: LogConfig::default(),
            debug: Default::default(),
            library: Default::default(),
            insecure_bind_any: false,
        }
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self, String> {
        let mut c: Self = toml::from_str(s).map_err(|e| e.to_string())?;
        c.model.llm_draft.get_or_insert(DEFAULT_LLM_DRAFT);
        Ok(c)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_toml(&s)
    }

    /// The server's per-session audio cap (`max_session_s` at 16 kHz).
    pub fn max_session_samples(&self) -> usize {
        (self.max_session_s as usize).saturating_mul(flow_core::config::SAMPLE_RATE)
    }

    pub fn bind_addrs(&self) -> Result<Vec<SocketAddr>, String> {
        if self.bind.is_empty() {
            return Err("bind: at least one address".into());
        }
        self.bind
            .iter()
            .map(|b| b.parse().map_err(|e| format!("bind {b:?}: {e}")))
            .collect()
    }

    /// S10: a wildcard (`0.0.0.0`, `::`) or public bind is refused without
    /// `[tls]` unless `insecure_bind_any` is set. Returns the addresses that
    /// are reachable beyond loopback/LAN/VPN (for the startup warning).
    pub fn check_bind(&self) -> Result<Vec<SocketAddr>, String> {
        let wide: Vec<SocketAddr> = self
            .bind_addrs()?
            .into_iter()
            .filter(|a| !crate::net::classify(a.ip()).is_local())
            .collect();
        if wide.is_empty() || self.tls.is_some() || self.insecure_bind_any {
            return Ok(wide);
        }
        let list: Vec<String> = wide.iter().map(|a| a.to_string()).collect();
        Err(format!(
            "bind {} is a wildcard or public address: plain ws:// would expose audio and the token beyond loopback/LAN/VPN. Bind a loopback, LAN or VPN (100.64/10) address, configure [tls], or pass --insecure-bind-any",
            list.join(", ")
        ))
    }

    /// `auth = "none"` is only allowed when every bind address is loopback.
    /// Returns whether auth is on.
    fn auth_on(&self) -> Result<bool, String> {
        if self.auth == AuthMode::None {
            let addrs = self.bind_addrs()?;
            if addrs.iter().all(|a| a.ip().is_loopback()) {
                return Ok(false);
            }
            return Err(
                "auth = \"none\" is only allowed when every bind address is loopback".into(),
            );
        }
        Ok(true)
    }

    /// The old single token (`token`, else `token_file`), if configured;
    /// `None` too when auth is off.
    pub fn resolve_token(&self) -> Result<Option<String>, String> {
        if !self.auth_on()? {
            return Ok(None);
        }
        let t = match (&self.token, &self.token_file) {
            (Some(t), _) => t.clone(),
            (None, Some(p)) => std::fs::read_to_string(p)
                .map_err(|e| format!("token_file {}: {e}", p.display()))?
                .trim()
                .to_string(),
            (None, None) => return Ok(None),
        };
        if t.len() < flow_protocol::MIN_TOKEN_LEN {
            return Err(format!(
                "token shorter than {} characters",
                flow_protocol::MIN_TOKEN_LEN
            ));
        }
        Ok(Some(t))
    }

    /// The keys directory: `state_dir`, else the default state dir.
    pub fn keys_dir(&self) -> Option<PathBuf> {
        self.state_dir.clone().or_else(crate::capture::state_dir)
    }

    /// Check the auth rules without writing anything: auth needs a token
    /// to import or an existing keys file.
    pub fn check_auth(&self) -> Result<(), String> {
        if self.resolve_token()?.is_some() || !self.auth_on()? {
            return Ok(());
        }
        match self.keys_dir() {
            Some(d) if d.join(crate::keys::KEYS_FILE).is_file() => Ok(()),
            _ => Err(NO_KEYS.into()),
        }
    }

    /// Open the key store (`None` = auth off). An in-process `token`
    /// without `state_dir` gives an in-memory store; otherwise the store
    /// lives in [`Config::keys_dir`] and imports the token file.
    pub fn open_keys(&self) -> Result<Option<crate::keys::KeyStore>, String> {
        use crate::keys::KeyStore;
        let token = self.resolve_token()?;
        if !self.auth_on()? {
            return Ok(None);
        }
        if let (Some(t), None, None) = (&self.token, &self.state_dir, &self.token_file) {
            return Ok(Some(KeyStore::in_memory(t)));
        }
        let dir = self.keys_dir().ok_or(
            "no state directory for the keys file (set state_dir in the config)".to_string(),
        )?;
        if token.is_none() && !dir.join(crate::keys::KEYS_FILE).is_file() {
            return Err(NO_KEYS.into());
        }
        if let Some(d) = &self.state_dir {
            if !d.is_absolute() {
                return Err(format!("state_dir {} must be absolute", d.display()));
            }
        }
        KeyStore::open(&dir, token.as_deref()).map(Some)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.check_auth()?;
        self.check_bind()?;
        if self.workers == 0 {
            return Err("workers must be >= 1".into());
        }
        if self.max_connections == 0 || self.max_connections_per_ip == 0 {
            return Err("max_connections and max_connections_per_ip must be >= 1".into());
        }
        self.debug.validate()?;
        if let Some(d) = self.library.dir.as_ref().filter(|d| !d.is_absolute()) {
            return Err(format!("[library] dir {} must be absolute", d.display()));
        }
        if let Some(t) = &self.tls {
            if !cfg!(feature = "tls") {
                return Err("[tls] set but built without the `tls` feature".into());
            }
            if !t.cert.is_file() || !t.key.is_file() {
                return Err("[tls] cert and key must be existing files".into());
            }
        }
        Ok(())
    }
}

const NO_KEYS: &str = "no keys: set token_file (`flow-server gen-token --out PATH`) or add one with `flow-server keys add <name> --admin`";

/// 32 random bytes, base64url without padding (43 chars).
pub fn gen_token() -> String {
    use base64::Engine;
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("OS randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Write a new token to `path` with mode 0600 (refuses to overwrite).
pub fn write_token(path: &Path) -> Result<String, String> {
    let t = gen_token();
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    use std::io::Write;
    let mut f = o
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(format!("{t}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let s = include_str!("../dist/config.example.toml");
        let c = Config::from_toml(s).unwrap();
        assert_eq!(c.workers, 1);
        assert_eq!(c.model.threads, 4);
        assert!(!c.log.log_text);
        assert!(!c.debug.capture, "capture is off unless asked for");
        assert_eq!((c.debug.max_files, c.debug.max_age_h), (30, 72));
    }

    #[test]
    fn colleague_bundle_config_parses() {
        // tools/colleague-bundle.sh ships this as the bundle's config.toml.
        let s = include_str!("../../../tools/bundle/server-config.toml");
        let c = Config::from_toml(s).unwrap();
        assert_eq!(c.model.asr_backend, flow_core::config::AsrBackend::Parakeet);
        assert!(c.model.llm_path.is_some() && c.model.lid_path.is_some());
        let he = &c.model.languages["he"];
        assert_eq!(he.asr_backend, flow_core::config::AsrBackend::SherpaOnline);
        assert!(!c.log.log_text);
    }

    #[test]
    fn example_hebrew_block_parses_when_uncommented() {
        // The commented `[model.languages.he]` block of the example config,
        // uncommented up to its alternative, must be valid as shipped.
        let s = include_str!("../dist/config.example.toml");
        let mut out = String::new();
        let mut on = false;
        for l in s.lines() {
            if l.starts_with("# [model.languages.he]") {
                on = true;
            } else if l.starts_with("# Alternative") || l.starts_with("# [tls]") {
                on = false;
            }
            match l.strip_prefix("# ").filter(|_| on) {
                Some(u) => out.push_str(u),
                None => out.push_str(l),
            }
            out.push('\n');
        }
        let c = Config::from_toml(&out).unwrap();
        assert_eq!(c.model.asr_backend, flow_core::config::AsrBackend::Parakeet);
        let he = &c.model.languages["he"];
        assert_eq!(he.asr_backend, flow_core::config::AsrBackend::SherpaOnline);
        assert!(he.asr_path.ends_with("/zipformer2-66m-streaming-he-int8"));
        assert_eq!(he.threads, Some(4));
        assert_eq!(c.model.language_route(Some("he-IL")), Some("he"));
        assert_eq!(c.model.language_route(Some("auto")), None);
        assert!(Config::from_toml(s).unwrap().model.languages.is_empty());
    }

    #[test]
    fn cleanup_drafting_is_on_unless_the_config_turns_it_off() {
        assert_eq!(Config::default().model.llm_draft, Some(DEFAULT_LLM_DRAFT));
        let c = Config::from_toml("[model]\nasr_path = \"m\"\n").unwrap();
        assert_eq!(c.model.llm_draft, Some(DEFAULT_LLM_DRAFT));
        assert_eq!(
            Config::from_toml("").unwrap().model.llm_draft,
            Some(DEFAULT_LLM_DRAFT)
        );
        let off = Config::from_toml("[model]\nllm_draft = 0\n").unwrap();
        assert_eq!(off.model.llm_draft, Some(0));
    }

    #[test]
    fn debug_section_parses_and_validates() {
        let ok = "auth = \"none\"\n[debug]\ncapture = true\nmax_files = 5\nmax_age_h = 24\ndir = \"/var/lib/flow-server/debug\"\n";
        let c = Config::from_toml(ok).unwrap();
        assert!(c.debug.capture);
        assert_eq!((c.debug.max_files, c.debug.max_age_h), (5, 24));
        assert!(c.validate().is_ok());
        let c = Config::from_toml("auth = \"none\"\n[debug]\nmax_files = 0\n").unwrap();
        assert!(c.validate().unwrap_err().contains("max_files"));
        assert!(
            Config::from_toml("[debug]\ncaptur = true\n").is_err(),
            "a misspelt key is an error, not a silent no-op"
        );
    }

    #[test]
    fn mock_model_in_toml() {
        let c = Config::from_toml(
            "auth = \"none\"\n[model]\nasr_engine = { mock = { text = \"hi\" } }\n",
        )
        .unwrap();
        assert!(matches!(
            c.model.asr_backend,
            flow_core::config::AsrBackend::Mock { .. }
        ));
        assert_eq!(c.resolve_token().unwrap(), None);
    }

    #[test]
    fn hebrew_zipformer_route_in_toml() {
        let c = Config::from_toml(
            "auth = \"none\"\n[model]\nasr_engine = \"parakeet\"\nasr_path = \"/m/en\"\n\
             [model.languages.he]\nasr_backend = \"zipformer\"\nasr_path = \"/m/zip-he\"\n",
        )
        .unwrap();
        let he = &c.model.languages["he"];
        assert_eq!(he.asr_backend, flow_core::config::AsrBackend::SherpaOnline);
        assert_eq!(he.asr_path, "/m/zip-he");
    }

    #[test]
    fn auth_none_needs_loopback_only() {
        let c =
            Config::from_toml("auth = \"none\"\nbind = [\"127.0.0.1:1\", \"100.64.0.10:7313\"]")
                .unwrap();
        assert!(c.resolve_token().is_err());
        let c = Config::from_toml("bind = [\"[::1]:7313\"]\nauth = \"none\"").unwrap();
        assert!(c.resolve_token().is_ok());
    }

    #[test]
    fn wildcard_bind_needs_tls_or_the_override() {
        let c = Config::from_toml("bind = [\"0.0.0.0:7313\"]").unwrap();
        assert!(c.check_bind().unwrap_err().contains("--insecure-bind-any"));
        let c = Config {
            tls: Some(TlsConfig::default()),
            ..c
        };
        assert_eq!(c.check_bind().unwrap().len(), 1);
        let c = Config {
            tls: None,
            insecure_bind_any: true,
            ..c
        };
        assert!(c.check_bind().is_ok());
        assert!(Config::default().check_bind().unwrap().is_empty());
    }

    #[test]
    fn insecure_bind_any_cannot_come_from_the_file() {
        let c = Config::from_toml("insecure_bind_any = true\nbind = [\"0.0.0.0:1\"]");
        assert!(c.map_or(true, |c| !c.insecure_bind_any));
    }

    #[test]
    fn token_required_and_long_enough() {
        let d = tempfile::tempdir().unwrap();
        let c = Config {
            state_dir: Some(d.path().into()),
            ..Default::default()
        };
        assert!(c.check_auth().unwrap_err().contains("keys add"));
        assert!(c.open_keys().is_err(), "no token and no keys file");
        let c = Config {
            token: Some("short".into()),
            ..Default::default()
        };
        assert!(c.resolve_token().is_err());
        let c = Config {
            token: Some("0123456789abcdef".into()),
            ..Default::default()
        };
        assert_eq!(c.resolve_token().unwrap().unwrap(), "0123456789abcdef");
    }

    #[test]
    fn keys_file_alone_is_enough_and_token_lives_in_memory_in_process() {
        let d = tempfile::tempdir().unwrap();
        crate::keys::KeyStore::open(d.path(), None)
            .unwrap()
            .add("phone", false)
            .unwrap();
        let c = Config {
            state_dir: Some(d.path().into()),
            ..Default::default()
        };
        assert!(c.check_auth().is_ok());
        assert_eq!(c.open_keys().unwrap().unwrap().active(), 1);
        // An in-process token without state_dir never touches the disk.
        let c = Config {
            token: Some("0123456789abcdef".into()),
            ..Default::default()
        };
        assert!(c.open_keys().unwrap().unwrap().keys_path().is_none());
        let c = Config::from_toml("auth = \"none\"").unwrap();
        assert!(c.open_keys().unwrap().is_none());
    }

    #[test]
    fn gen_token_is_43_chars_and_written_0600() {
        let t = gen_token();
        assert_eq!(t.len(), 43);
        assert_ne!(t, gen_token());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("token");
        let w = write_token(&p).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), w);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(write_token(&p).is_err(), "never overwrites");
        let c = Config {
            token_file: Some(p),
            ..Default::default()
        };
        assert_eq!(c.resolve_token().unwrap().unwrap(), w);
    }
}
