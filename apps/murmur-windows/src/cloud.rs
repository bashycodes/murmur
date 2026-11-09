//! OpenRouter (D18 / D28) shell helpers that need no Win32 or network:
//! user-facing failure text, the Developer "Active engine" rows, the usage
//! file (`%APPDATA%\Murmur\cloud_usage.json`, never sent anywhere), the
//! model-catalogue cache path and the one-time D28 settings migration.
//!
//! OpenRouter is the only cloud provider (D28): no presets, no base URLs,
//! no other host. `murmur_cloud` (feature `cloud`) talks to it; this module
//! only formats and persists.

use crate::credentials::{self, CredentialStore};
use crate::dev_settings::DevSettings;
use murmur_router::cloud::{CUSTOM_CLEANUP_ID, PURPOSE_CLEANUP, PURPOSE_STT, REMOVED_PROVIDERS_ID};
use murmur_router::{CloudUsage, MigrationReport};
use std::path::{Path, PathBuf};

/// How the shell names the provider (pill, badge, rows).
pub const PROVIDER_NAME: &str = "OpenRouter";
/// The one host audio or text can go to.
pub const PROVIDER_HOST: &str = murmur_router::cloud::OPENROUTER_HOST;

/// `route.fallback` / `CleanupStatus.blocked` code → what the user is told
/// (changelog "What W needs" + D28). `None` for codes that are not about
/// OpenRouter.
pub fn failure_text(code: &str) -> Option<&'static str> {
    Some(match code {
        "auth_failed" => "key rejected",
        "quota_exceeded" => "out of credits",
        "connect_failed" => "can't reach openrouter.ai",
        "budget_exceeded" => "monthly budget reached",
        "cloud_not_configured" => "set a key and accept the consent",
        "cloud_cleanup_not_configured" => "turn on cleanup, accept its consent and save a key",
        "no_private_route" => "no zero-retention provider serves this model",
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
    // An empty f64 sum is -0.0: never show "$-0.0000".
    let usd = usd + 0.0;
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

/// Engine row for a cloud dictation: `cloud · OpenRouter · <model> · $cost`.
pub fn engine_row(model: &str, cost_usd: Option<f64>, estimated: bool) -> String {
    let mut parts = vec!["cloud".to_string(), PROVIDER_NAME.to_string()];
    if !model.is_empty() {
        parts.push(model.to_string());
    }
    if let Some(c) = cost_usd {
        parts.push(format_cost(c, estimated));
    }
    parts.join(" · ")
}

/// The LLM step's row: `<outcome> · OpenRouter · <model> · $cost` for a
/// cloud cleanup, `<outcome> · this PC` / `· your server` otherwise.
/// `None` when the LLM did not run (no outcome).
pub fn llm_row(
    outcome: Option<&str>,
    engine: Option<&str>,
    model: Option<&str>,
    cost_usd: Option<f64>,
    estimated: bool,
) -> Option<String> {
    let outcome = outcome?;
    let mut parts = vec![outcome.to_string()];
    match engine {
        Some("cloud") => {
            parts.push(PROVIDER_NAME.to_string());
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                parts.push(m.to_string());
            }
            if let Some(c) = cost_usd {
                parts.push(format_cost(c, estimated));
            }
        }
        Some("local") => parts.push("this PC".into()),
        Some("server") => parts.push("your server".into()),
        _ => {}
    }
    Some(parts.join(" · "))
}

/// D30: [`llm_row`] plus what produced the text when AI cleanup (cloud)
/// was tried and its answer was not used: "… · OpenRouter not used
/// (llm_timeout) → this PC" / "→ fixed rules". Unchanged otherwise.
pub fn with_cleanup_route(
    row: Option<String>,
    cleanup_route: Option<&str>,
    cloud_fallback: Option<&str>,
) -> Option<String> {
    let used = match cleanup_route {
        Some("local") => "this PC",
        Some("rules") => "fixed rules",
        _ => return row,
    };
    let note = format!(
        "{PROVIDER_NAME} not used ({}) → {used}",
        cloud_fallback.unwrap_or("?")
    );
    Some(match row {
        Some(r) => format!("{r} · {note}"),
        None => note,
    })
}

// ------------------------------------------------------------------ usage

pub fn usage_path(appdata: Option<&str>) -> PathBuf {
    crate::prefs::Prefs::default_path(appdata).with_file_name("cloud_usage.json")
}

/// `%APPDATA%\Murmur\cache\openrouter-catalog.json`: the trimmed model
/// catalogue (`murmur_cloud::CatalogStore`, 24 h TTL). Public data only.
pub fn catalog_cache_path(appdata: Option<&str>) -> PathBuf {
    crate::prefs::Prefs::default_path(appdata)
        .with_file_name("cache")
        .join("openrouter-catalog.json")
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

/// One usage row: speech-to-text, cleanup, providers removed by the D28
/// migration (their cost still counts toward the budget), or the D52
/// custom cleanup server (its requests always; never in the budget).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageRow {
    /// `stt` / `cleanup` / `removed` / `custom`.
    pub purpose: String,
    pub label: String,
    pub requests: u64,
    pub minutes: f64,
    pub cost: String,
}

/// OpenRouter page "Usage this month": `Speech $x · Cleanup $y`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageView {
    pub month: String,
    pub speech: String,
    pub cleanup: String,
    pub rows: Vec<UsageRow>,
    /// OpenRouter's sum this month (both uses + removed providers), what
    /// the budget counts (`CloudUsage::budget_cost`), formatted.
    pub total: String,
    pub budget: Option<f64>,
    pub budget_reached: bool,
}

/// Row of the providers the D28 migration removed (still in the budget).
pub const PURPOSE_REMOVED: &str = "removed";
/// D52 row of the custom cleanup server: its cleanups always, its cost
/// header when sent ("no cost reported" otherwise); never in the budget.
pub const PURPOSE_CUSTOM: &str = "custom";
/// The custom row's cost when the server sends no cost header.
pub const NO_COST_REPORTED: &str = "no cost reported";

fn purpose_of(p: &murmur_router::ProviderUsage) -> &'static str {
    if p.provider_id == CUSTOM_CLEANUP_ID {
        // Its own row, outside OpenRouter's speech/cleanup sums and budget.
        PURPOSE_CUSTOM
    } else if p.provider_id == REMOVED_PROVIDERS_ID {
        PURPOSE_REMOVED
    } else if p.purpose == PURPOSE_CLEANUP {
        PURPOSE_CLEANUP
    } else {
        PURPOSE_STT
    }
}

pub fn usage_view(u: &CloudUsage, budget: Option<f64>) -> UsageView {
    let est = |purpose: &str| {
        u.providers
            .iter()
            .any(|p| purpose_of(p) == purpose && p.cost_estimated)
    };
    let sum = |purpose: &str| -> f64 {
        u.providers
            .iter()
            .filter(|p| purpose_of(p) == purpose)
            .map(|p| p.cost_usd.0)
            .sum()
    };
    let mut rows: Vec<UsageRow> = Vec::new();
    for purpose in [
        PURPOSE_STT,
        PURPOSE_CLEANUP,
        PURPOSE_REMOVED,
        PURPOSE_CUSTOM,
    ] {
        let of: Vec<&murmur_router::ProviderUsage> = u
            .providers
            .iter()
            .filter(|p| purpose_of(p) == purpose)
            .collect();
        if of.is_empty() {
            continue;
        }
        let cost: f64 = of.iter().map(|p| p.cost_usd.0).sum();
        let ms: u64 = of.iter().map(|p| p.billed_audio_ms).sum();
        rows.push(UsageRow {
            purpose: purpose.into(),
            label: match purpose {
                PURPOSE_STT => "Speech-to-text",
                PURPOSE_CLEANUP => "Text cleanup",
                PURPOSE_CUSTOM => "Your proxy",
                _ => "Removed providers (before OpenRouter only)",
            }
            .into(),
            requests: of.iter().map(|p| p.requests).sum(),
            minutes: (ms as f64 / 60_000.0 * 100.0).round() / 100.0,
            cost: match purpose {
                // The server's own cost header: exact, not an estimate.
                PURPOSE_CUSTOM if cost > 0.0 => format_cost(cost, false),
                PURPOSE_CUSTOM => NO_COST_REPORTED.into(),
                _ => format_cost(cost, est(purpose)),
            },
        });
    }
    let total = u.budget_cost();
    UsageView {
        month: u.month.clone(),
        // Not `CloudUsage::cost_for`: that counts the `removed` counter
        // (no purpose) as speech; here it has its own row.
        speech: format_cost(sum(PURPOSE_STT), est(PURPOSE_STT)),
        cleanup: format_cost(sum(PURPOSE_CLEANUP), est(PURPOSE_CLEANUP)),
        rows,
        total: format_cost(
            total,
            u.providers
                .iter()
                .any(|p| p.cost_estimated && p.provider_id != CUSTOM_CLEANUP_ID),
        ),
        budget,
        budget_reached: budget.is_some_and(|b| total >= b),
    }
}

// -------------------------------------------------------------- migration

/// What the startup migration did (for the debug log; never a key).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Migrated {
    pub report: MigrationReport,
    pub keys: credentials::KeyMigration,
    pub usage_rewritten: bool,
}

/// One-time D28 migration at startup (openrouter-only-cloud.md §2.1, changelog W5 step 1), on
/// the stored settings before the router is built:
/// 1. `DevSettings::migrate_d28` (schema 1 → 2; the notice is kept in
///    `cloud_notice` until the user dismisses it);
/// 2. Credential Manager: the kept OpenRouter key moves to
///    `murmur/cloud/openrouter`, every other provider's key is deleted;
/// 3. `cloud_usage.json`: the kept entry's counters become `openrouter`,
///    removed providers collapse into one `removed` counter (the month's
///    total and so the budget are unchanged).
///
/// `None` when the settings were already schema 2 (nothing touched). The
/// caller persists `dev` (prefs.json).
pub fn migrate_d28(
    dev: &mut DevSettings,
    store: &dyn CredentialStore,
    usage_path: &Path,
) -> Option<Migrated> {
    if !dev.cloud.is_legacy() {
        return None;
    }
    let legacy = dev.cloud.providers.clone();
    let report = dev.migrate_d28();
    let host = |id: &str| legacy.iter().find(|p| p.id == id).map(|p| p.host());
    let keys = credentials::apply_migration(store, &report, &host);
    let mut usage_rewritten = false;
    if let Some(mut u) = load_usage(usage_path) {
        let before = u.clone();
        u.migrate_d28(&report);
        if u != before && save_usage(usage_path, &u).is_ok() {
            usage_rewritten = true;
        }
    }
    Some(Migrated {
        report,
        keys,
        usage_rewritten,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmur_router::{ProviderUsage, Usd};

    #[test]
    fn failure_texts_match_the_spec() {
        assert_eq!(failure_text("auth_failed"), Some("key rejected"));
        assert_eq!(failure_text("quota_exceeded"), Some("out of credits"));
        assert_eq!(
            failure_text("connect_failed"),
            Some("can't reach openrouter.ai")
        );
        assert_eq!(
            failure_text("budget_exceeded"),
            Some("monthly budget reached")
        );
        assert_eq!(
            failure_text("cloud_not_configured"),
            Some("set a key and accept the consent")
        );
        assert_eq!(
            failure_text("no_private_route"),
            Some("no zero-retention provider serves this model")
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
            engine_row("openai/whisper-large-v3-turbo", Some(0.0012), false),
            "cloud · OpenRouter · openai/whisper-large-v3-turbo · $0.0012"
        );
        assert_eq!(
            engine_row("openai/whisper-large-v3-turbo", Some(0.00011), true),
            "cloud · OpenRouter · openai/whisper-large-v3-turbo · ≈ $0.0001"
        );
        assert_eq!(engine_row("", None, false), "cloud · OpenRouter");
    }

    #[test]
    fn llm_row_names_the_cleanup_engine() {
        assert_eq!(
            llm_row(
                Some("AI cleaned"),
                Some("cloud"),
                Some("meta-llama/llama-3.3-70b-instruct"),
                Some(0.0007),
                false
            )
            .as_deref(),
            Some("AI cleaned · OpenRouter · meta-llama/llama-3.3-70b-instruct · $0.0007")
        );
        assert_eq!(
            llm_row(Some("AI cleaned"), Some("local"), None, None, false).as_deref(),
            Some("AI cleaned · this PC")
        );
        assert_eq!(
            llm_row(
                Some("AI not needed (already clean)"),
                None,
                None,
                None,
                false
            )
            .as_deref(),
            Some("AI not needed (already clean)"),
            "llm_skipped_clean is a normal result"
        );
        assert_eq!(llm_row(None, Some("cloud"), None, None, false), None);
    }

    #[test]
    fn cleanup_route_names_the_fallback_when_ai_cleanup_was_not_used() {
        let row = || Some("AI cleaned · this PC".to_string());
        assert_eq!(
            with_cleanup_route(row(), Some("local"), Some("llm_timeout")).as_deref(),
            Some("AI cleaned · this PC · OpenRouter not used (llm_timeout) → this PC")
        );
        assert_eq!(
            with_cleanup_route(None, Some("rules"), Some("llm_empty")).as_deref(),
            Some("OpenRouter not used (llm_empty) → fixed rules")
        );
        assert_eq!(with_cleanup_route(row(), Some("cloud"), None), row());
        assert_eq!(with_cleanup_route(row(), None, None), row());
    }

    #[test]
    fn usage_persists_and_restores() {
        let dir = std::env::temp_dir().join(format!("murmur-cloud-usage-{}", std::process::id()));
        let path = dir.join("cloud_usage.json");
        assert_eq!(load_usage(&path), None);
        let fixture: CloudUsage = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../crates/murmur-router/fixtures/cloud_usage.json"),
            )
            .unwrap(),
        )
        .unwrap();
        save_usage(&path, &fixture).unwrap();
        assert_eq!(load_usage(&path), Some(fixture));
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_usage(&path), None, "corrupt file = start at zero");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(usage_path(Some("C:/AppData")).ends_with("cloud_usage.json"));
        let c = catalog_cache_path(Some("C:/AppData"));
        assert!(c.ends_with("Murmur/cache/openrouter-catalog.json"), "{c:?}");
    }

    #[test]
    fn usage_view_splits_speech_and_cleanup() {
        let u = CloudUsage {
            month: "2026-10".into(),
            providers: vec![
                ProviderUsage {
                    provider_id: "openrouter".into(),
                    purpose: "stt".into(),
                    requests: 3,
                    billed_audio_ms: 90_000,
                    cost_usd: Usd(1.5),
                    cost_estimated: false,
                },
                ProviderUsage {
                    provider_id: "openrouter".into(),
                    purpose: "cleanup".into(),
                    requests: 2,
                    billed_audio_ms: 0,
                    cost_usd: Usd(0.0014),
                    cost_estimated: true,
                },
                ProviderUsage {
                    provider_id: REMOVED_PROVIDERS_ID.into(),
                    requests: 1,
                    billed_audio_ms: 60_000,
                    cost_usd: Usd(0.01),
                    ..Default::default()
                },
            ],
        };
        let v = usage_view(&u, Some(1.0));
        assert_eq!(v.speech, "$1.50");
        assert_eq!(v.cleanup, "≈ $0.0014");
        assert_eq!(v.rows.len(), 3);
        assert_eq!(v.rows[0].purpose, "stt");
        assert_eq!(v.rows[0].minutes, 1.5);
        assert_eq!(v.rows[2].purpose, PURPOSE_REMOVED);
        assert_eq!(v.total, "≈ $1.51");
        assert!(v.budget_reached);
        assert!(!usage_view(&u, None).budget_reached);
        let empty = usage_view(&CloudUsage::default(), None);
        assert_eq!(
            (empty.speech.as_str(), empty.cleanup.as_str()),
            ("$0.0000", "$0.0000")
        );
        assert!(empty.rows.is_empty());
    }

    #[test]
    fn custom_server_cost_is_shown_apart_from_the_budget() {
        let u = CloudUsage {
            month: "2026-11".into(),
            providers: vec![
                ProviderUsage {
                    provider_id: "openrouter".into(),
                    purpose: "stt".into(),
                    requests: 1,
                    cost_usd: Usd(0.2),
                    ..Default::default()
                },
                ProviderUsage {
                    provider_id: CUSTOM_CLEANUP_ID.into(),
                    purpose: "cleanup".into(),
                    requests: 9,
                    cost_usd: Usd(5.0),
                    cost_estimated: true,
                    ..Default::default()
                },
            ],
        };
        let v = usage_view(&u, Some(1.0));
        assert_eq!(v.total, "$0.2000");
        assert!(!v.budget_reached, "custom spend never counts");
        assert_eq!(v.cleanup, "$0.0000");
        assert!(v.rows.iter().all(|r| r.purpose != "cleanup"));
        let row = v.rows.last().unwrap();
        assert_eq!(
            (
                row.purpose.as_str(),
                row.label.as_str(),
                row.requests,
                row.cost.as_str()
            ),
            (PURPOSE_CUSTOM, "Your proxy", 9, "$5.00")
        );
    }

    #[test]
    fn custom_server_requests_count_without_a_cost() {
        // A server that sends no cost header still shows its cleanups,
        // next to OpenRouter's own cleanup row.
        let u = CloudUsage {
            month: "2026-11".into(),
            providers: vec![
                ProviderUsage {
                    provider_id: "openrouter".into(),
                    purpose: "cleanup".into(),
                    requests: 2,
                    cost_usd: Usd(0.01),
                    ..Default::default()
                },
                ProviderUsage {
                    provider_id: CUSTOM_CLEANUP_ID.into(),
                    purpose: "cleanup".into(),
                    requests: 3,
                    ..Default::default()
                },
            ],
        };
        let v = usage_view(&u, None);
        let purposes: Vec<&str> = v.rows.iter().map(|r| r.purpose.as_str()).collect();
        assert_eq!(purposes, [PURPOSE_CLEANUP, PURPOSE_CUSTOM]);
        assert_eq!((v.rows[0].requests, v.rows[1].requests), (2, 3));
        assert_eq!(v.cleanup, "$0.0100");
        assert_eq!(v.total, "$0.0100");
        assert_eq!(v.rows[1].cost, NO_COST_REPORTED);
    }
}
