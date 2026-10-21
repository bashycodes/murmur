//! Provider presets (docs/cloud-stt.md §8.2) and the static price table
//! used when a provider does not report the cost (prices 2026-10, §2).

use flow_router::{CloudProviderConfig, Usd};

/// Preset id → a provider config without id / key / consent. The Developer
/// page copies one, assigns an id and lets the user edit it.
pub fn presets() -> Vec<CloudProviderConfig> {
    let p = |preset: &str, adapter: &str, label: &str, base: &str, model: &str, zdr: bool| {
        CloudProviderConfig {
            preset: preset.into(),
            adapter: adapter.into(),
            label: label.into(),
            base_url: base.into(),
            model: model.into(),
            zero_retention: zdr,
            ..Default::default()
        }
    };
    vec![
        p(
            "openrouter",
            "openrouter",
            "OpenRouter",
            "https://openrouter.ai/api/v1",
            "openai/gpt-transcribe",
            true,
        ),
        // Zero retention needs an enterprise plan: off by default.
        p(
            "elevenlabs",
            "elevenlabs",
            "ElevenLabs Scribe",
            "https://api.elevenlabs.io",
            "scribe_v2",
            false,
        ),
        // ZDR is a Groq account setting, not a request flag.
        p(
            "groq",
            "openai_compat",
            "Groq",
            "https://api.groq.com/openai/v1",
            "whisper-large-v3-turbo",
            false,
        ),
        CloudProviderConfig {
            dialect: "gpt_transcribe".into(),
            ..p(
                "openai",
                "openai_compat",
                "OpenAI",
                "https://api.openai.com/v1",
                "gpt-transcribe",
                false,
            )
        },
        p(
            "deepgram",
            "deepgram",
            "Deepgram",
            "https://api.deepgram.com",
            "nova-3",
            true,
        ),
        p(
            "assemblyai",
            "assemblyai",
            "AssemblyAI",
            "https://api.assemblyai.com",
            "universal",
            true,
        ),
        p("custom", "openai_compat", "Custom", "", "", false),
    ]
}

/// USD per hour of audio for a provider, when it does not report cost:
/// the config's `price_per_hour_usd`, else the model table, else the
/// preset default, else `None`.
pub fn price_per_hour(p: &CloudProviderConfig) -> Option<f64> {
    if let Some(Usd(v)) = p.price_per_hour_usd {
        return Some(v);
    }
    let m = p.model.as_str();
    let by_model = match m {
        "openai/gpt-transcribe" | "gpt-transcribe" => Some(0.27),
        "openai/whisper-1" | "whisper-1" => Some(0.36),
        "openai/whisper-large-v3-turbo" => Some(0.012),
        "elevenlabs/scribe-v2" | "scribe_v2" => Some(0.22),
        "whisper-large-v3-turbo" => Some(0.04),
        "whisper-large-v3" => Some(0.111),
        "nova-3" => Some(0.26),
        "universal" => Some(0.21),
        _ => None,
    };
    by_model.or(match p.preset.as_str() {
        "elevenlabs" => Some(0.22),
        "openrouter" | "openai" => Some(0.27),
        "groq" => Some(0.04),
        "deepgram" => Some(0.26),
        "assemblyai" => Some(0.21),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_validate_once_given_an_id() {
        for mut p in presets().into_iter().filter(|p| p.preset != "custom") {
            p.id = format!("{}-1", p.preset);
            assert_eq!(p.validate(), Ok(()), "{}", p.preset);
            assert!(price_per_hour(&p).is_some(), "{}", p.preset);
        }
    }

    #[test]
    fn price_override_wins() {
        let mut p = presets().remove(2);
        assert_eq!(price_per_hour(&p), Some(0.04));
        p.price_per_hour_usd = Some(Usd(1.5));
        assert_eq!(price_per_hour(&p), Some(1.5));
    }
}
