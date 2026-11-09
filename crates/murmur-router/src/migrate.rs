//! One-time D28 migration: schema-1 cloud config (D18 providers/presets) →
//! schema 2 (OpenRouter only). Spec `docs/proposals/openrouter-only-cloud.md`
//! §2.1. Pure data: the shell applies the key moves / deletions the report
//! lists to its secure store (Credential Manager / Keystore) and shows the
//! notice once.

use serde::{Deserialize, Serialize};

use crate::cloud::{
    CloudConfig, CloudUsage, OpenRouterConfig, Route, CLOUD_SCHEMA, DEFAULT_STT_MODEL,
    NO_ZDR_STT_MODELS_2026_10_24, OPENROUTER_HOST, OPENROUTER_ID, REMOVED_PROVIDERS_ID,
};
use crate::RouterConfig;

/// A provider entry the migration deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemovedProvider {
    pub id: String,
    /// `label`, else the id.
    pub name: String,
    pub preset: String,
    pub adapter: String,
}

/// What `RouterConfig::migrate_d28` did. JSON for the shells
/// (`nativeMigrateCloud`); every list empty and `migrated = false` when
/// the config was already schema 2.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MigrationReport {
    /// The config was schema 1 and is now schema 2.
    pub migrated: bool,
    /// Move this stored key to `key_to` (the kept OpenRouter entry's old
    /// id → `"openrouter"`): Windows `murmur/cloud/<from>` →
    /// `murmur/cloud/openrouter`, Android `murmur_cloud_<from>` →
    /// `murmur_cloud_openrouter`. `None` when there is nothing to move.
    pub key_from: Option<String>,
    pub key_to: Option<String>,
    /// Stored keys to delete (every non-OpenRouter entry, and extra
    /// OpenRouter entries after the first).
    pub delete_keys: Vec<String>,
    pub removed: Vec<RemovedProvider>,
    /// The cloud STT route was turned off (`allow_cloud = false`, `cloud`
    /// removed from `order`): the active provider was not OpenRouter.
    pub cloud_disabled: bool,
    /// The kept model has no ZDR endpoint while ZDR is on, so it was
    /// replaced by the default: `(from, to)`.
    pub model_from: Option<String>,
    pub model_to: Option<String>,
    /// One-time notice to show ("Cloud speech-to-text now uses OpenRouter
    /// only. …"); `None` when nothing user-visible changed.
    pub notice: Option<String>,
}

impl RouterConfig {
    /// Schema 1 → 2, idempotent (spec §2.1 rules 1-5):
    /// 1. The first `adapter == "openrouter"` entry is kept: its model,
    ///    language, send_dictionary, format, timeout, zero-retention flag
    ///    and audio consent (only if it was given for openrouter.ai). Its
    ///    key moves to id `openrouter`.
    /// 2. Every other entry and its key is deleted.
    /// 3. An active non-OpenRouter provider (or none) is not replaced
    ///    silently: `allow_cloud = false`, `cloud` leaves `order`, notice.
    /// 4. A kept model without a ZDR endpoint (ZDR on) becomes the default.
    /// 5. `schema = 2`; legacy fields cleared.
    pub fn migrate_d28(&mut self) -> MigrationReport {
        let mut rep = MigrationReport::default();
        if !self.cloud.is_legacy() {
            return rep;
        }
        rep.migrated = true;
        let old = std::mem::take(&mut self.cloud);
        let mut or = OpenRouterConfig::default();
        let kept = old.providers.iter().find(|p| p.adapter == "openrouter");
        if let Some(p) = kept {
            or.privacy.zdr = p.zero_retention;
            or.stt.model = if p.model.trim().is_empty() {
                DEFAULT_STT_MODEL.into()
            } else {
                p.model.clone()
            };
            or.stt.language = p.language.clone();
            or.stt.send_dictionary = p.send_dictionary;
            or.stt.format_override = p.format_override.clone();
            if (1000..=60_000).contains(&p.timeout_base_ms) {
                or.stt.timeout_base_ms = p.timeout_base_ms;
            }
            // Consent stays valid only if it was for openrouter.ai and the
            // entry really pointed there.
            if p.consent_ok() && p.host() == OPENROUTER_HOST {
                or.stt.consent_host = OPENROUTER_HOST.into();
                or.stt.consent_at_ms = p.consent_at_ms;
            }
            if p.id != OPENROUTER_ID {
                rep.key_from = Some(p.id.clone());
                rep.key_to = Some(OPENROUTER_ID.into());
            }
            if or.privacy.zdr && NO_ZDR_STT_MODELS_2026_10_24.contains(&or.stt.model.as_str()) {
                rep.model_from = Some(or.stt.model.clone());
                rep.model_to = Some(DEFAULT_STT_MODEL.into());
                or.stt.model = DEFAULT_STT_MODEL.into();
            }
        }
        for p in &old.providers {
            if kept.is_some_and(|k| std::ptr::eq(k, p)) {
                continue;
            }
            if p.id != OPENROUTER_ID {
                rep.delete_keys.push(p.id.clone());
            }
            rep.removed.push(RemovedProvider {
                id: p.id.clone(),
                name: p.display_name().to_string(),
                preset: p.preset.clone(),
                adapter: p.adapter.clone(),
            });
        }
        let active_is_kept = kept.is_some_and(|k| !old.active.is_empty() && k.id == old.active);
        if !active_is_kept && (self.allow_cloud || self.order.contains(&Route::Cloud)) {
            self.allow_cloud = false;
            self.order.retain(|r| *r != Route::Cloud);
            rep.cloud_disabled = true;
        }
        self.cloud = CloudConfig {
            schema: CLOUD_SCHEMA,
            openrouter: or,
            monthly_budget_usd: old.monthly_budget_usd,
            // Schema 1 had no custom cleanup server (D52).
            ..CloudConfig::default()
        };
        rep.notice = notice(&rep, &old);
        rep
    }
}

fn notice(rep: &MigrationReport, old: &CloudConfig) -> Option<String> {
    let mut parts = Vec::new();
    let active_removed = rep.removed.iter().find(|r| r.id == old.active);
    if !rep.removed.is_empty() || rep.cloud_disabled {
        let names: Vec<&str> = rep.removed.iter().map(|r| r.name.as_str()).collect();
        let mut s = String::from("Cloud speech-to-text now uses OpenRouter only.");
        if !names.is_empty() {
            let verb = if names.len() == 1 { "was" } else { "were" };
            let keys = if names.len() == 1 {
                "its key"
            } else {
                "their keys"
            };
            s.push_str(&format!(
                " {} {verb} removed and {keys} deleted.",
                names.join(", ")
            ));
        }
        if rep.cloud_disabled || active_removed.is_some() {
            s.push_str(" Set up OpenRouter to keep using cloud STT.");
        }
        parts.push(s);
    }
    if let (Some(f), Some(t)) = (&rep.model_from, &rep.model_to) {
        parts.push(format!(
            "The model {f} has no zero-retention provider on OpenRouter, so speech-to-text now uses {t}."
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

impl CloudUsage {
    /// Apply a migration to the month's counters: the kept entry's
    /// counters move to `openrouter`; deleted providers' counters collapse
    /// into one `removed` entry so the month's total (budget) is unchanged.
    pub fn migrate_d28(&mut self, rep: &MigrationReport) {
        if !rep.migrated {
            return;
        }
        let removed: Vec<&str> = rep.removed.iter().map(|r| r.id.as_str()).collect();
        let mut gone = crate::cloud::ProviderUsage {
            provider_id: REMOVED_PROVIDERS_ID.into(),
            ..Default::default()
        };
        let mut any_gone = false;
        self.providers.retain_mut(|p| {
            if rep.key_from.as_deref() == Some(p.provider_id.as_str()) {
                p.provider_id = OPENROUTER_ID.into();
                true
            } else if removed.contains(&p.provider_id.as_str()) {
                gone.requests += p.requests;
                gone.billed_audio_ms += p.billed_audio_ms;
                gone.cost_usd = crate::Usd(gone.cost_usd.0 + p.cost_usd.0);
                gone.cost_estimated |= p.cost_estimated;
                any_gone = true;
                false
            } else {
                true
            }
        });
        if any_gone {
            match self
                .providers
                .iter_mut()
                .find(|p| p.provider_id == REMOVED_PROVIDERS_ID)
            {
                Some(p) => {
                    p.requests += gone.requests;
                    p.billed_audio_ms += gone.billed_audio_ms;
                    p.cost_usd = crate::Usd(p.cost_usd.0 + gone.cost_usd.0);
                    p.cost_estimated |= gone.cost_estimated;
                }
                None => self.providers.push(gone),
            }
        }
    }
}
