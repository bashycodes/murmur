//! `/etc/flow-server/config.toml` (docs/hybrid-architecture.md §3.2).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use flow_core::config::ModelConfig;
use serde::Deserialize;

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
    pub token_file: Option<PathBuf>,
    pub auth: AuthMode,
    /// In-process use (tests): the token itself instead of `token_file`.
    #[serde(skip)]
    pub token: Option<String>,
    pub workers: usize,
    pub max_queue: usize,
    pub queue_timeout_ms: u64,
    pub idle_timeout_s: u64,
    pub max_session_s: u64,
    pub model: ModelConfig,
    pub tls: Option<TlsConfig>,
    pub log: LogConfig,
    /// `--insecure-bind-any`: allow a wildcard or public bind without TLS
    /// and accept peers outside loopback/LAN/Tailscale. Command line only.
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
            workers: 1,
            max_queue: 2,
            queue_timeout_ms: 30_000,
            idle_timeout_s: 120,
            max_session_s: 1200,
            model: ModelConfig::default(),
            tls: None,
            log: LogConfig::default(),
            insecure_bind_any: false,
        }
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
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
    /// are reachable beyond loopback/LAN/Tailscale (for the startup warning).
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
            "bind {} is a wildcard or public address: plain ws:// would expose audio and the token beyond loopback/LAN/Tailscale. Bind a loopback, LAN or Tailscale address, configure [tls], or pass --insecure-bind-any",
            list.join(", ")
        ))
    }

    /// Resolve the bearer token (`None` = auth disabled) and check the
    /// rules: a token is required unless `auth = "none"` and every bind
    /// address is loopback.
    pub fn resolve_token(&self) -> Result<Option<String>, String> {
        if self.auth == AuthMode::None {
            let addrs = self.bind_addrs()?;
            if addrs.iter().all(|a| a.ip().is_loopback()) {
                return Ok(None);
            }
            return Err(
                "auth = \"none\" is only allowed when every bind address is loopback".into(),
            );
        }
        let t = match (&self.token, &self.token_file) {
            (Some(t), _) => t.clone(),
            (None, Some(p)) => std::fs::read_to_string(p)
                .map_err(|e| format!("token_file {}: {e}", p.display()))?
                .trim()
                .to_string(),
            (None, None) => {
                return Err(
                    "token_file is required (run `flow-server gen-token --out PATH`)".into(),
                )
            }
        };
        if t.len() < flow_protocol::MIN_TOKEN_LEN {
            return Err(format!(
                "token shorter than {} characters",
                flow_protocol::MIN_TOKEN_LEN
            ));
        }
        Ok(Some(t))
    }

    pub fn validate(&self) -> Result<(), String> {
        self.resolve_token()?;
        self.check_bind()?;
        if self.workers == 0 {
            return Err("workers must be >= 1".into());
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
        let c = Config::default();
        assert!(c.resolve_token().is_err());
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
