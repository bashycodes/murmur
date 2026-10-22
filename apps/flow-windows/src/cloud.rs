//! Cloud STT (D18) shell helpers that need no Win32 or network: user-facing
//! failure text, the Developer "Active engine" row, the usage file
//! (`%APPDATA%\FlowClone\cloud_usage.json`, never sent anywhere) and the
//! provider presets for "Add provider".

use flow_router::{CloudProviderConfig, CloudUsage};
use std::path::{Path, PathBuf};

/// `route.fallback` code → what the user is told (§8.3 / changelog "What W
/// needs"). `None` for codes that are not about the cloud provider.
pub fn failure_text(code: &str) -> Option<&'static str> {
    Some(match code {
        "auth_failed" => "key rejected",
        "quota_exceeded" => "out of credits",
        "connect_failed" => "no internet / base URL wrong",
        "budget_exceeded" => "monthly budget reached",
        "cloud_not_configured" => "set a key and accept the consent",
        "provider_error" => "the provider returned an error",
        "timeout" => "the provider took too long",
        "busy" => "rate limited by the provider",
        "circuit_open" => "paused after recent failures",
        _ => return None,
    })
}

/// "Cloud STT failed (<reason>); used <fallback>" — `used` is the route
/// that served the dictation instead (`None` = nothing did).
pub fn failure_message(code: &str, used: Option<&str>) -> String {
    let why = failure_text(code).unwrap_or(code);
    match used {
        Some("local") => format!("Cloud STT failed ({why}); used this PC"),
        Some("remote") => format!("Cloud STT failed ({why}); used your server"),
        Some(other) => format!("Cloud STT failed ({why}); used {other}"),
        None => format!("Cloud STT failed ({why})"),
    }
}

/// `$0.0012`, `≈ $0.0012` when estimated. Small amounts keep 4 decimals.
pub fn format_cost(usd: f64, estimated: bool) -> String {
    let n = if usd >= 1.0 {
        format!("${usd:.2}")
    } else {
        format!("${usd:.4}")
    };
    if estimated {
        format!("≈ {n}")
    } else {
        n
    }
}

/// Engine row for a cloud dictation: `cloud · <provider> · <model> · $cost`.
pub fn engine_row(provider: &str, model: &str, cost_usd: Option<f64>, estimated: bool) -> String {
    let mut parts = vec!["cloud".to_string(), provider.to_string()];
    if !model.is_empty() {
        parts.push(model.to_string());
    }
    if let Some(c) = cost_usd {
        parts.push(format_cost(c, estimated));
    }
    parts.join(" · ")
}

// ------------------------------------------------------------------ usage

pub fn usage_path(appdata: Option<&str>) -> PathBuf {
    crate::prefs::Prefs::default_path(appdata).with_file_name("cloud_usage.json")
}

/// Persisted counters; `None` when missing or unreadable (start at zero).
pub fn load_usage(path: &Path) -> Option<CloudUsage> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Atomic write (temp file + rename), like `prefs.json`.
pub fn save_usage(path: &Path, usage: &CloudUsage) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(usage).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Developer page usage row per provider.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageRow {
    pub provider_id: String,
    pub label: String,
    pub requests: u64,
    pub minutes: f64,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageView {
    pub month: String,
    pub rows: Vec<UsageRow>,
    /// Sum this month, formatted.
    pub total: String,
    pub budget: Option<f64>,
    pub budget_reached: bool,
}

pub fn usage_view(
    u: &CloudUsage,
    providers: &[CloudProviderConfig],
    budget: Option<f64>,
) -> UsageView {
    let estimated = u.providers.iter().any(|p| p.cost_estimated);
    let total = u.total_cost();
    UsageView {
        month: u.month.clone(),
        rows: u
            .providers
            .iter()
            .map(|p| UsageRow {
                provider_id: p.provider_id.clone(),
                label: providers
                    .iter()
                    .find(|c| c.id == p.provider_id)
                    .map(|c| c.display_name().to_string())
                    .unwrap_or_else(|| p.provider_id.clone()),
                requests: p.requests,
                minutes: (p.billed_audio_ms as f64 / 60_000.0 * 100.0).round() / 100.0,
                cost: format_cost(p.cost_usd.0, p.cost_estimated),
            })
            .collect(),
        total: format_cost(total, estimated),
        budget,
        budget_reached: budget.is_some_and(|b| total >= b),
    }
}

/// "Add provider" presets (§8.2). Empty in builds without the cloud client.
pub fn presets() -> Vec<CloudProviderConfig> {
    #[cfg(feature = "cloud")]
    {
        flow_cloud::presets()
    }
    #[cfg(not(feature = "cloud"))]
    {
        Vec::new()
    }
}

/// A free provider id for `preset` (`groq-1`, `groq-2`, …).
pub fn next_id(preset: &str, taken: &[CloudProviderConfig]) -> String {
    let base: String = preset
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(28)
        .collect::<String>()
        .to_ascii_lowercase();
    let base = if base.is_empty() {
        "custom".into()
    } else {
        base
    };
    (1..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.iter().any(|p| &p.id == id))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_router::{ProviderUsage, Usd};

    #[test]
    fn failure_texts_match_the_spec() {
        assert_eq!(failure_text("auth_failed"), Some("key rejected"));
        assert_eq!(failure_text("quota_exceeded"), Some("out of credits"));
        assert_eq!(
            failure_text("connect_failed"),
            Some("no internet / base URL wrong")
        );
        assert_eq!(
            failure_text("budget_exceeded"),
            Some("monthly budget reached")
        );
        assert_eq!(
            failure_text("cloud_not_configured"),
            Some("set a key and accept the consent")
        );
        assert_eq!(failure_text("proto_mismatch"), None);
        assert_eq!(
            failure_message("auth_failed", Some("local")),
            "Cloud STT failed (key rejected); used this PC"
        );
        assert_eq!(
            failure_message("budget_exceeded", None),
            "Cloud STT failed (monthly budget reached)"
        );
    }

    #[test]
    fn engine_row_shape() {
        assert_eq!(
            engine_row("OpenRouter", "openai/gpt-transcribe", Some(0.0012), false),
            "cloud · OpenRouter · openai/gpt-transcribe · $0.0012"
        );
        assert_eq!(
            engine_row("Groq", "whisper-large-v3-turbo", Some(0.00011), true),
            "cloud · Groq · whisper-large-v3-turbo · ≈ $0.0001"
        );
        assert_eq!(engine_row("Groq", "", None, false), "cloud · Groq");
    }

    #[test]
    fn usage_persists_and_restores() {
        let dir = std::env::temp_dir().join(format!("flow-cloud-usage-{}", std::process::id()));
        let path = dir.join("cloud_usage.json");
        assert_eq!(load_usage(&path), None);
        let u = CloudUsage {
            month: "2026-10".into(),
            providers: vec![ProviderUsage {
                provider_id: "openrouter-1".into(),
                requests: 42,
                billed_audio_ms: 312_000,
                cost_usd: Usd(0.0234),
                cost_estimated: false,
            }],
        };
        save_usage(&path, &u).unwrap();
        assert_eq!(load_usage(&path), Some(u.clone()));
        // Same shape as the router fixture.
        let fixture: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../crates/flow-router/fixtures/cloud_usage.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&u).unwrap(), fixture);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_usage(&path), None, "corrupt file = start at zero");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(usage_path(Some("C:/AppData")).ends_with("cloud_usage.json"));
    }

    #[test]
    fn usage_view_and_budget() {
        let u = CloudUsage {
            month: "2026-10".into(),
            providers: vec![ProviderUsage {
                provider_id: "groq-1".into(),
                requests: 3,
                billed_audio_ms: 90_000,
                cost_usd: Usd(1.5),
                cost_estimated: true,
            }],
        };
        let p = CloudProviderConfig {
            id: "groq-1".into(),
            label: "Groq".into(),
            ..Default::default()
        };
        let v = usage_view(&u, &[p], Some(1.0));
        assert_eq!(v.rows[0].label, "Groq");
        assert_eq!(v.rows[0].minutes, 1.5);
        assert_eq!(v.total, "≈ $1.50");
        assert!(v.budget_reached);
        assert!(!usage_view(&u, &[], None).budget_reached);
    }

    #[test]
    fn ids_are_unique_and_valid() {
        let taken = vec![CloudProviderConfig {
            id: "groq-1".into(),
            ..Default::default()
        }];
        assert_eq!(next_id("groq", &taken), "groq-2");
        assert_eq!(next_id("OpenRouter", &[]), "openrouter-1");
        assert_eq!(next_id("", &[]), "custom-1");
    }
}
