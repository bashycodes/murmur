//! murmur-cloud's own view of the OpenRouter settings (D28,
//! `docs/proposals/openrouter-only-cloud.md` §2.6). Field names match the
//! router's schema-2 JSON (`cloud.openrouter.{privacy,stt,cleanup}`); the
//! glue in `lib.rs` converts from the router's config types. Keys are
//! never part of these types.

use serde::{Deserialize, Serialize};

/// Default STT model: ZDR on Groq + DeepInfra, $0.012-0.04 per hour (§2.3).
pub const DEFAULT_STT_MODEL: &str = "openai/whisper-large-v3-turbo";
/// Default cleanup model: non-reasoning, ZDR on Groq / SambaNova (§2.3).
pub const DEFAULT_CLEANUP_MODEL: &str = "meta-llama/llama-3.3-70b-instruct";

/// Routing privacy shared by both uses. Default: the most private routing
/// OpenRouter offers, `provider: {zdr: true, data_collection: "deny"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub zdr: bool,
    /// `"deny"` (default) or `"allow"`. Anything but `"allow"` is deny.
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

    /// The privacy part of a request's `provider` object.
    pub fn provider_json(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        if self.zdr {
            m.insert("zdr".into(), true.into());
        }
        if self.data_collection_deny() {
            m.insert("data_collection".into(), "deny".into());
        }
        m
    }
}

/// Which OpenRouter API transcribes (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttPath {
    /// Path A: `POST /audio/transcriptions` (dedicated ASR models).
    Transcriptions,
    /// Path B: `POST /chat/completions` with an `input_audio` part (chat
    /// models; may paraphrase, so a sanity check guards the output).
    ChatAudio,
}

impl SttPath {
    pub fn as_str(self) -> &'static str {
        match self {
            SttPath::Transcriptions => "transcriptions",
            SttPath::ChatAudio => "chat_audio",
        }
    }
}

/// The config's `stt_path`: `"auto"` picks from the catalogue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttPathChoice {
    #[default]
    Auto,
    Transcriptions,
    ChatAudio,
}

impl SttPathChoice {
    /// `"auto" | "transcriptions" | "chat_audio"`; anything else is auto.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "transcriptions" => SttPathChoice::Transcriptions,
            "chat_audio" => SttPathChoice::ChatAudio,
            _ => SttPathChoice::Auto,
        }
    }
    pub fn forced(self) -> Option<SttPath> {
        match self {
            SttPathChoice::Auto => None,
            SttPathChoice::Transcriptions => Some(SttPath::Transcriptions),
            SttPathChoice::ChatAudio => Some(SttPath::ChatAudio),
        }
    }
}

/// Cloud speech-to-text (`cloud.openrouter.stt`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SttSettings {
    pub model: String,
    pub stt_path: SttPathChoice,
    /// ISO-639-1; empty = the session language, else provider auto-detect.
    pub language: String,
    /// Send dictionary terms as biasing hints (Groq whisper / path B only).
    pub send_dictionary: bool,
    /// `"wav"` forces WAV on path A (default FLAC).
    pub format_override: String,
    pub timeout_base_ms: u64,
    pub privacy: Privacy,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            model: DEFAULT_STT_MODEL.into(),
            stt_path: SttPathChoice::Auto,
            language: String::new(),
            send_dictionary: true,
            format_override: String::new(),
            timeout_base_ms: 6000,
            privacy: Privacy::default(),
        }
    }
}

/// What the cleanup prompt may contain beyond the transcript, category,
/// style and dictionary (§2.5 "context minimisation"). Default: nothing.
/// Mirrors `murmur_core::cleanup::PromptScope`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupScope {
    pub surrounding_text: bool,
    pub screen_words: bool,
    pub app_id: bool,
}

/// Opt-in cloud text cleanup (`cloud.openrouter.cleanup`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupSettings {
    pub model: String,
    /// `latency | throughput | price | ""` (`""` = OpenRouter load balancing).
    pub sort: String,
    pub timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub send_surrounding_text: bool,
    pub send_screen_words: bool,
    pub send_app_name: bool,
    /// `provider.allow_fallbacks` (OpenRouter default true; fallbacks still
    /// honour the ZDR filter).
    pub allow_fallbacks: bool,
    pub privacy: Privacy,
}

impl Default for CleanupSettings {
    fn default() -> Self {
        Self {
            model: DEFAULT_CLEANUP_MODEL.into(),
            sort: "latency".into(),
            timeout_ms: 2500,
            connect_timeout_ms: 1500,
            send_surrounding_text: false,
            send_screen_words: false,
            send_app_name: false,
            allow_fallbacks: true,
            privacy: Privacy::default(),
        }
    }
}

impl CleanupSettings {
    pub fn scope(&self) -> CleanupScope {
        CleanupScope {
            surrounding_text: self.send_surrounding_text,
            screen_words: self.send_screen_words,
            app_id: self.send_app_name,
        }
    }
}

// ---- schema-2 router config → murmur-cloud settings --------------------------

impl From<&murmur_router::cloud::Privacy> for Privacy {
    fn from(p: &murmur_router::cloud::Privacy) -> Self {
        Privacy {
            zdr: p.zdr,
            data_collection: if p.data_collection_deny() {
                "deny".into()
            } else {
                "allow".into()
            },
        }
    }
}

impl From<&murmur_router::OpenRouterConfig> for SttSettings {
    fn from(o: &murmur_router::OpenRouterConfig) -> Self {
        let s = &o.stt;
        SttSettings {
            model: if s.model.trim().is_empty() {
                DEFAULT_STT_MODEL.into()
            } else {
                s.model.trim().to_string()
            },
            stt_path: SttPathChoice::parse(&s.stt_path),
            language: s.language.clone(),
            send_dictionary: s.send_dictionary,
            format_override: s.format_override.clone(),
            timeout_base_ms: s.timeout_base_ms,
            privacy: (&o.privacy).into(),
        }
    }
}

impl From<&murmur_router::OpenRouterConfig> for CleanupSettings {
    fn from(o: &murmur_router::OpenRouterConfig) -> Self {
        let c = &o.cleanup;
        CleanupSettings {
            model: if c.model.trim().is_empty() {
                DEFAULT_CLEANUP_MODEL.into()
            } else {
                c.model.trim().to_string()
            },
            sort: c.sort.clone(),
            timeout_ms: c.timeout_ms,
            connect_timeout_ms: c.connect_timeout_ms,
            send_surrounding_text: c.send_surrounding_text,
            send_screen_words: c.send_screen_words,
            send_app_name: c.send_app_name,
            allow_fallbacks: true,
            privacy: (&o.privacy).into(),
        }
    }
}

/// Free-text model id check (§2.2): `^[a-z0-9._~/-]+(:[a-z]+)*$`, ≤ 128.
pub fn valid_model_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    let mut parts = id.split(':');
    let base = parts.next().unwrap_or("");
    !base.is_empty()
        && base
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && base
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._~/-".contains(&b))
        && parts.all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_defaults_to_the_most_private_routing() {
        let p = Privacy::default();
        assert_eq!(
            serde_json::Value::Object(p.provider_json()),
            serde_json::json!({"zdr": true, "data_collection": "deny"})
        );
        let off = Privacy {
            zdr: false,
            data_collection: "allow".into(),
        };
        assert!(off.provider_json().is_empty());
        // Unknown values never weaken privacy.
        let odd = Privacy {
            zdr: false,
            data_collection: "maybe".into(),
        };
        assert!(odd.data_collection_deny());
    }

    #[test]
    fn model_ids() {
        for ok in [
            "openai/whisper-large-v3-turbo",
            "meta-llama/llama-3.3-70b-instruct:nitro",
            "x/y:free:nitro",
            "a~b/c.d_e",
        ] {
            assert!(valid_model_id(ok), "{ok}");
        }
        for bad in [
            "",
            "../../etc",
            "a//b",
            "/x",
            "Openai/x",
            "a b",
            "x:",
            "x:NITRO",
            "x/y:1",
            &"a".repeat(129),
        ] {
            assert!(!valid_model_id(bad), "{bad}");
        }
    }

    #[test]
    fn json_names_match_the_router_schema() {
        let s: SttSettings = serde_json::from_value(serde_json::json!({
            "model": "m", "stt_path": "chat_audio", "language": "de",
            "send_dictionary": false, "format_override": "wav", "timeout_base_ms": 1
        }))
        .unwrap();
        assert_eq!(s.stt_path, SttPathChoice::ChatAudio);
        assert_eq!(s.privacy, Privacy::default());
        let c = CleanupSettings::default();
        assert_eq!(c.scope(), CleanupScope::default());
        assert_eq!((c.timeout_ms, c.connect_timeout_ms), (2500, 1500));
        assert_eq!(SttPathChoice::parse("bogus"), SttPathChoice::Auto);
    }

    #[test]
    fn router_config_converts() {
        let mut o = murmur_router::OpenRouterConfig::default();
        assert_eq!(SttSettings::from(&o), SttSettings::default());
        assert_eq!(CleanupSettings::from(&o), CleanupSettings::default());
        assert_eq!(DEFAULT_STT_MODEL, murmur_router::cloud::DEFAULT_STT_MODEL);
        assert_eq!(
            DEFAULT_CLEANUP_MODEL,
            murmur_router::cloud::DEFAULT_CLEANUP_MODEL
        );
        o.stt.stt_path = "chat_audio".into();
        o.privacy.zdr = false;
        o.cleanup.send_app_name = true;
        assert_eq!(SttSettings::from(&o).stt_path, SttPathChoice::ChatAudio);
        assert!(!SttSettings::from(&o).privacy.zdr);
        assert!(CleanupSettings::from(&o).scope().app_id);
    }
}
