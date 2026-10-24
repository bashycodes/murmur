//! The OpenRouter settings page (D28, `docs/cloud-stt.md`, spec
//! `docs/proposals/openrouter-only-cloud.md` §2.2-§2.6): what the page shows
//! ([`OrView`]) and the validated edits it can make. Pure: no Win32, no
//! network, no key. `app.rs` wraps each edit in a Tauri command that saves
//! prefs.json and hands the new config to the router.
//!
//! Rules the page cannot get around:
//! * a use is switched on only with its **own** consent for openrouter.ai
//!   (STT consent is not cleanup consent, and the reverse);
//! * consent is recorded here, with the host fixed to openrouter.ai and the
//!   time of the call: the page only says "the dialog was accepted";
//! * the key never appears in any view (only [`crate::credentials::KeyState`]).

use crate::credentials::KeyState;
use crate::dev_settings::{BuildCaps, DevSettings, Route};
use flow_router::cloud::{valid_model_id, CLEANUP_TIMEOUT_RANGE_MS, OPENROUTER_HOST};
use flow_router::{CleanupRoute, CleanupStatus, CloudUsage, Usd};
use serde::{Deserialize, Serialize};

/// Speech-to-text card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SttView {
    pub allow: bool,
    pub consent: bool,
    pub consent_at_ms: u64,
    pub model: String,
    pub stt_path: String,
    pub language: String,
    pub send_dictionary: bool,
}

/// Text cleanup card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CleanupView {
    pub allow: bool,
    pub consent: bool,
    pub consent_at_ms: u64,
    pub order: Vec<CleanupRoute>,
    pub model: String,
    pub sort: String,
    pub timeout_ms: u64,
    pub send_surrounding_text: bool,
    pub send_screen_words: bool,
    pub send_app_name: bool,
    /// `Router::cleanup_status()` (blocked reason, circuit, last call).
    pub status: CleanupStatus,
}

/// `dev_or_get` and every `dev_or_set_*` answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrView {
    /// This build can reach OpenRouter (feature `cloud`).
    pub supported: bool,
    pub key: KeyState,
    pub privacy: flow_router::cloud::Privacy,
    pub stt: SttView,
    pub cleanup: CleanupView,
    pub budget: Option<f64>,
    pub usage: crate::cloud::UsageView,
    /// `CatalogStore::status_json` (`source`, `fetched_at_ms`, counts).
    pub catalog: serde_json::Value,
    /// One-time migration notice, until dismissed.
    pub notice: Option<String>,
    pub warnings: Vec<String>,
    /// The saved route order (turning STT on puts `cloud` first; the
    /// Developer tab shows it).
    pub route_order: Vec<Route>,
    /// Picker rows of the selected models (`None` = not in the catalogue,
    /// e.g. a typed id).
    pub stt_model_info: Option<serde_json::Value>,
    pub cleanup_model_info: Option<serde_json::Value>,
}

/// What the page knows besides the settings (router / store state).
pub struct ViewInputs {
    pub caps: BuildCaps,
    pub key: KeyState,
    pub usage: CloudUsage,
    pub cleanup_status: CleanupStatus,
    pub catalog: serde_json::Value,
    pub stt_model_info: Option<serde_json::Value>,
    pub cleanup_model_info: Option<serde_json::Value>,
}

pub fn view(dev: &DevSettings, i: ViewInputs) -> OrView {
    let or = &dev.cloud.openrouter;
    let budget = dev.cloud.monthly_budget_usd.map(|b| b.0);
    OrView {
        supported: i.caps.cloud,
        privacy: or.privacy.clone(),
        stt: SttView {
            allow: dev.allow_cloud,
            consent: or.stt.consent_ok(),
            consent_at_ms: or.stt.consent_at_ms,
            model: or.stt.model.clone(),
            stt_path: or.stt.stt_path.clone(),
            language: or.stt.language.clone(),
            send_dictionary: or.stt.send_dictionary,
        },
        cleanup: CleanupView {
            allow: dev.allow_cloud_cleanup,
            consent: or.cleanup.consent_ok(),
            consent_at_ms: or.cleanup.consent_at_ms,
            order: dev.cleanup_order.clone(),
            model: or.cleanup.model.clone(),
            sort: or.cleanup.sort.clone(),
            timeout_ms: or.cleanup.timeout_ms,
            send_surrounding_text: or.cleanup.send_surrounding_text,
            send_screen_words: or.cleanup.send_screen_words,
            send_app_name: or.cleanup.send_app_name,
            status: i.cleanup_status,
        },
        budget,
        usage: crate::cloud::usage_view(&i.usage, budget),
        catalog: i.catalog,
        notice: dev.cloud_notice.clone(),
        route_order: dev.order(),
        warnings: dev.cloud_warnings(i.caps, i.key.set),
        key: i.key,
        stt_model_info: i.stt_model_info,
        cleanup_model_info: i.cleanup_model_info,
    }
}

/// `dev_or_set_stt`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SttForm {
    pub allow: bool,
    /// The consent dialog was accepted just now.
    pub consent: bool,
    pub language: String,
    pub send_dictionary: bool,
    /// `auto` | `transcriptions` | `chat_audio`; empty = keep.
    pub stt_path: String,
}

/// `dev_or_set_cleanup`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CleanupForm {
    pub allow: bool,
    /// The cleanup consent dialog was accepted just now.
    pub consent: bool,
    /// `None` = keep; `[]` with `allow` = the default `[cloud, local]`.
    pub order: Option<Vec<CleanupRoute>>,
    /// `latency` | `throughput` | `price` | `""`.
    pub sort: String,
    /// i64 so 0 or 99999 reach validation instead of a parse error.
    pub timeout_ms: i64,
    pub send_surrounding_text: bool,
    pub send_screen_words: bool,
    pub send_app_name: bool,
}

/// `""` or an ISO-639-1/-3 code (`en`, `de`, `yue`).
fn normalize_language(raw: &str) -> Result<String, String> {
    let l = raw.trim().to_ascii_lowercase();
    if l.is_empty() || ((2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase())) {
        Ok(l)
    } else {
        Err("Language must be a 2-letter code like en or de (empty = automatic)".into())
    }
}

fn check(dev: &DevSettings) -> Result<(), String> {
    dev.cloud.validate()
}

/// Speech-to-text card: toggle (with consent), language, dictionary,
/// path. Turning it on puts `cloud` first in the route order (the user can
/// reorder under Developer → Engines).
pub fn apply_stt(dev: &mut DevSettings, f: &SttForm, now_ms: u64) -> Result<(), String> {
    let mut next = dev.clone();
    let stt = &mut next.cloud.openrouter.stt;
    stt.language = normalize_language(&f.language)?;
    stt.send_dictionary = f.send_dictionary;
    if !f.stt_path.is_empty() {
        stt.stt_path = f.stt_path.clone();
    }
    if f.consent {
        stt.consent_host = OPENROUTER_HOST.into();
        stt.consent_at_ms = now_ms.max(1);
    }
    if f.allow && !stt.consent_ok() {
        return Err("Accept sending audio to OpenRouter before turning this on".into());
    }
    if f.allow && !dev.allow_cloud {
        let mut order: Vec<Route> = vec![Route::Cloud];
        order.extend(next.order().into_iter().filter(|r| *r != Route::Cloud));
        next.route_order = order;
    }
    next.allow_cloud = f.allow;
    check(&next)?;
    *dev = next;
    Ok(())
}

/// Text cleanup card: toggle (with its own consent), order, sort, timeout
/// and the three "send …" switches.
pub fn apply_cleanup(dev: &mut DevSettings, f: &CleanupForm, now_ms: u64) -> Result<(), String> {
    let mut next = dev.clone();
    let c = &mut next.cloud.openrouter.cleanup;
    let (lo, hi) = CLEANUP_TIMEOUT_RANGE_MS;
    if !(lo as i64..=hi as i64).contains(&f.timeout_ms) {
        return Err(format!("Cleanup time limit must be {lo}-{hi} ms"));
    }
    c.timeout_ms = f.timeout_ms as u64;
    c.sort = f.sort.trim().to_string();
    c.send_surrounding_text = f.send_surrounding_text;
    c.send_screen_words = f.send_screen_words;
    c.send_app_name = f.send_app_name;
    if f.consent {
        c.consent_host = OPENROUTER_HOST.into();
        c.consent_at_ms = now_ms.max(1);
    }
    if f.allow && !c.consent_ok() {
        return Err("Accept sending text to OpenRouter before turning this on".into());
    }
    if let Some(order) = &f.order {
        let mut o: Vec<CleanupRoute> = Vec::new();
        for r in order {
            if !o.contains(r) {
                o.push(*r);
            }
        }
        next.cleanup_order = o;
    }
    if f.allow && !next.cleanup_order.contains(&CleanupRoute::Cloud) {
        next.cleanup_order = vec![CleanupRoute::Cloud, CleanupRoute::Local];
    }
    next.allow_cloud_cleanup = f.allow;
    check(&next)?;
    *dev = next;
    Ok(())
}

/// Privacy (shared by both uses): zero-retention endpoints only, and
/// providers that do not collect data.
pub fn apply_privacy(
    dev: &mut DevSettings,
    zdr: bool,
    data_collection: &str,
) -> Result<(), String> {
    let mut next = dev.clone();
    next.cloud.openrouter.privacy.zdr = zdr;
    next.cloud.openrouter.privacy.data_collection = data_collection.to_string();
    check(&next)?;
    *dev = next;
    Ok(())
}

/// Monthly budget for both uses (`None` = no limit).
pub fn apply_budget(dev: &mut DevSettings, budget: Option<f64>) -> Result<(), String> {
    if let Some(b) = budget {
        if !(b.is_finite() && (0.0..=100_000.0).contains(&b)) {
            return Err("Budget must be an amount from 0 to 100000 USD".into());
        }
    }
    dev.cloud.monthly_budget_usd = budget.map(Usd);
    Ok(())
}

/// The model of one use (`stt` / `cleanup`), from the picker or typed.
/// Any OpenRouter-shaped id is accepted (models newer than the catalogue,
/// `:nitro` variants).
pub fn apply_model(dev: &mut DevSettings, purpose: &str, model: &str) -> Result<(), String> {
    let m = model.trim();
    if !valid_model_id(m) {
        return Err(format!(
            "\"{m}\" is not an OpenRouter model id (like openai/whisper-large-v3-turbo)"
        ));
    }
    let mut next = dev.clone();
    match purpose {
        "stt" => next.cloud.openrouter.stt.model = m.into(),
        "cleanup" => next.cloud.openrouter.cleanup.model = m.into(),
        p => return Err(format!("unknown use {p:?}")),
    }
    check(&next)?;
    *dev = next;
    Ok(())
}

/// The notice was shown and dismissed.
pub fn ack_notice(dev: &mut DevSettings) {
    dev.cloud_notice = None;
}

/// The sentence "Test" sends for cleanup (never the user's text).
pub const CLEANUP_SAMPLE: &str =
    "um so i think we should uh meet on tuesday no wait wednesday at three";

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: BuildCaps = BuildCaps {
        local: true,
        remote: true,
        cloud: true,
    };

    fn dev() -> DevSettings {
        DevSettings::for_build(ALL)
    }

    fn stt(allow: bool, consent: bool) -> SttForm {
        SttForm {
            allow,
            consent,
            send_dictionary: true,
            ..Default::default()
        }
    }

    fn cleanup(allow: bool, consent: bool) -> CleanupForm {
        CleanupForm {
            allow,
            consent,
            timeout_ms: 2500,
            sort: "latency".into(),
            ..Default::default()
        }
    }

    #[test]
    fn stt_needs_its_consent_and_records_it_for_openrouter() {
        let mut d = dev();
        assert!(apply_stt(&mut d, &stt(true, false), 10).is_err());
        assert!(!d.allow_cloud, "refused = unchanged");
        apply_stt(&mut d, &stt(true, true), 10).unwrap();
        let s = &d.cloud.openrouter.stt;
        assert!(d.allow_cloud && s.consent_ok());
        assert_eq!(
            (s.consent_host.as_str(), s.consent_at_ms),
            ("openrouter.ai", 10)
        );
        assert_eq!(
            d.route_order[0],
            Route::Cloud,
            "turning it on puts cloud first"
        );
        // Off and on again: the recorded consent still counts.
        apply_stt(&mut d, &stt(false, false), 20).unwrap();
        assert!(!d.allow_cloud && d.cloud.openrouter.stt.consent_ok());
        apply_stt(&mut d, &stt(true, false), 30).unwrap();
        assert_eq!(d.cloud.openrouter.stt.consent_at_ms, 10);
    }

    #[test]
    fn the_two_consents_are_independent() {
        let mut d = dev();
        apply_stt(&mut d, &stt(true, true), 10).unwrap();
        assert!(!d.cloud.openrouter.cleanup.consent_ok());
        assert!(
            apply_cleanup(&mut d, &cleanup(true, false), 11).is_err(),
            "STT consent does not cover text"
        );
        assert!(!d.allow_cloud_cleanup);
        let mut e = dev();
        apply_cleanup(&mut e, &cleanup(true, true), 12).unwrap();
        assert!(e.allow_cloud_cleanup && !e.allow_cloud);
        assert!(
            !e.cloud.openrouter.stt.consent_ok(),
            "cleanup consent does not cover audio"
        );
        assert!(apply_stt(&mut e, &stt(true, false), 13).is_err());
    }

    #[test]
    fn cleanup_order_defaults_and_validation() {
        let mut d = dev();
        apply_cleanup(&mut d, &cleanup(true, true), 1).unwrap();
        assert_eq!(
            d.cleanup_order,
            vec![CleanupRoute::Cloud, CleanupRoute::Local]
        );
        let mut f = cleanup(true, false);
        f.order = Some(vec![
            CleanupRoute::Local,
            CleanupRoute::Cloud,
            CleanupRoute::Local,
        ]);
        apply_cleanup(&mut d, &f, 2).unwrap();
        assert_eq!(
            d.cleanup_order,
            vec![CleanupRoute::Local, CleanupRoute::Cloud]
        );
        assert!(d.cleanup_effective(ALL));
        for bad in [0, 799, 8001] {
            let mut f = cleanup(true, false);
            f.timeout_ms = bad;
            assert!(apply_cleanup(&mut d, &f, 3).is_err(), "{bad}");
        }
        let mut f = cleanup(true, false);
        f.sort = "fastest".into();
        assert!(apply_cleanup(&mut d, &f, 3).is_err());
        let mut f = cleanup(true, false);
        f.send_surrounding_text = true;
        apply_cleanup(&mut d, &f, 4).unwrap();
        assert!(d.cloud.openrouter.cleanup.scope().surrounding_text);
        assert!(!d.cloud.openrouter.cleanup.scope().screen_words);
    }

    #[test]
    fn models_privacy_budget_language() {
        let mut d = dev();
        apply_model(&mut d, "stt", " elevenlabs/scribe-v2 ").unwrap();
        assert_eq!(d.cloud.openrouter.stt.model, "elevenlabs/scribe-v2");
        apply_model(&mut d, "cleanup", "meta-llama/llama-3.1-8b-instruct:nitro").unwrap();
        for bad in ["", "Has Spaces", "x:", "https://evil.example/model"] {
            assert!(apply_model(&mut d, "stt", bad).is_err(), "{bad}");
        }
        assert!(apply_model(&mut d, "tts", "a/b").is_err());
        apply_privacy(&mut d, false, "allow").unwrap();
        assert!(!d.cloud.openrouter.privacy.zdr);
        assert!(!d.cloud.openrouter.privacy.data_collection_deny());
        assert!(apply_privacy(&mut d, true, "maybe").is_err());
        apply_budget(&mut d, Some(5.0)).unwrap();
        assert_eq!(d.cloud.monthly_budget_usd, Some(Usd(5.0)));
        assert!(apply_budget(&mut d, Some(-1.0)).is_err());
        assert!(apply_budget(&mut d, Some(f64::NAN)).is_err());
        apply_budget(&mut d, None).unwrap();
        let mut f = stt(false, false);
        f.language = " DE ".into();
        apply_stt(&mut d, &f, 1).unwrap();
        assert_eq!(d.cloud.openrouter.stt.language, "de");
        f.language = "german".into();
        assert!(apply_stt(&mut d, &f, 1).is_err());
        f.language = String::new();
        f.stt_path = "carrier_pigeon".into();
        assert!(apply_stt(&mut d, &f, 1).is_err());
    }

    #[test]
    fn the_view_never_holds_a_key_and_shows_the_notice() {
        let mut d = dev();
        d.cloud_notice = Some("Cloud speech-to-text now uses OpenRouter only.".into());
        let v = view(
            &d,
            ViewInputs {
                caps: ALL,
                key: KeyState {
                    set: true,
                    masked: "••••cdef".into(),
                },
                usage: CloudUsage::default(),
                cleanup_status: CleanupStatus::default(),
                catalog: serde_json::json!({"source": "Baked"}),
                stt_model_info: None,
                cleanup_model_info: None,
            },
        );
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(
            j["key"],
            serde_json::json!({"set": true, "masked": "••••cdef"})
        );
        assert_eq!(j["stt"]["model"], "openai/whisper-large-v3-turbo");
        assert_eq!(j["cleanup"]["model"], "meta-llama/llama-3.3-70b-instruct");
        assert_eq!(
            j["privacy"],
            serde_json::json!({"zdr": true, "data_collection": "deny"})
        );
        assert!(v.notice.is_some());
        ack_notice(&mut d);
        assert_eq!(d.cloud_notice, None);
    }
}
