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
//!
//! D52: text cleanup can go to the user's own OpenAI-compatible server
//! (LiteLLM, vLLM, Ollama, …) instead ([`apply_custom_cleanup`]). Its
//! consent is bound to the host of the typed base URL: pointing the URL at
//! another host voids it. Cloud speech-to-text always stays on OpenRouter.

use crate::credentials::KeyState;
use crate::dev_settings::{full_cleanup_order, BuildCaps, CleanupEngine, DevSettings, Route};
use flow_router::cloud::{
    normalize_custom_base_url, valid_custom_model_id, valid_model_id, CLEANUP_TIMEOUT_RANGE_MS,
    CUSTOM_CLEANUP_TIMEOUT_RANGE_MS, OPENROUTER_HOST,
};
use flow_router::{CleanupProvider, CleanupRoute, CleanupStatus, CloudUsage, Usd};
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

/// Text cleanup card, and the D36 cleanup order (AI cleanup page).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CleanupView {
    /// OpenRouter's entry in the cleanup order is on.
    pub allow: bool,
    pub consent: bool,
    pub consent_at_ms: u64,
    /// D36: all three engines in the order tried, with their on/off flags.
    pub engines: Vec<CleanupEngine>,
    pub model: String,
    pub sort: String,
    pub timeout_ms: u64,
    pub send_surrounding_text: bool,
    pub send_screen_words: bool,
    pub send_app_name: bool,
    /// `Router::cleanup_status()` (blocked reason, circuit, last call).
    pub status: CleanupStatus,
    /// D52: who the `cloud` cleanup route calls (`openrouter` | `custom`).
    /// `allow`, the order and the "send …" switches are shared; `consent`,
    /// `model` and `timeout_ms` above are OpenRouter's.
    pub provider: CleanupProvider,
    pub custom: CustomView,
}

/// D52: the user's own OpenAI-compatible cleanup server.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomView {
    /// As saved (normalised: `/v1` added when typed without a path).
    pub base_url: String,
    /// Lower-cased host of `base_url` (`""` when empty).
    pub host: String,
    pub model: String,
    pub timeout_ms: u64,
    /// Consent for the current host (a host change voids it).
    pub consent: bool,
    pub consent_host: String,
    pub consent_at_ms: u64,
    /// `http://`: the key and the text cross the network unencrypted.
    pub plain_http: bool,
    /// The optional key (`flow-clone/cloud/custom`), masked.
    pub key: KeyState,
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
    /// D52: the custom cleanup server's key, masked.
    pub custom_key: KeyState,
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
            allow: dev.cleanup_on(CleanupRoute::Cloud),
            consent: or.cleanup.consent_ok(),
            consent_at_ms: or.cleanup.consent_at_ms,
            engines: dev.cleanup(),
            model: or.cleanup.model.clone(),
            sort: or.cleanup.sort.clone(),
            timeout_ms: or.cleanup.timeout_ms,
            send_surrounding_text: or.cleanup.send_surrounding_text,
            send_screen_words: or.cleanup.send_screen_words,
            send_app_name: or.cleanup.send_app_name,
            status: i.cleanup_status,
            provider: dev.cloud.cleanup_provider,
            custom: custom_view(&dev.cloud.custom_cleanup, i.custom_key),
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

fn custom_view(cc: &flow_router::CustomCleanupConfig, key: KeyState) -> CustomView {
    CustomView {
        base_url: cc.base_url.clone(),
        host: cc.host(),
        model: cc.model.clone(),
        timeout_ms: cc.timeout_ms,
        consent: cc.consent_ok(),
        consent_host: cc.consent_host.clone(),
        consent_at_ms: cc.consent_at_ms,
        plain_http: is_plain_http(&cc.base_url),
        key,
    }
}

fn is_plain_http(url: &str) -> bool {
    url.trim_start().to_ascii_lowercase().starts_with("http://")
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
    /// `None` = keep; otherwise the new cleanup order (completed as
    /// [`full_cleanup_order`]; every engine keeps its on/off flag).
    pub order: Option<Vec<CleanupRoute>>,
    /// `latency` | `throughput` | `price` | `""`.
    pub sort: String,
    /// i64 so 0 or 99999 reach validation instead of a parse error.
    pub timeout_ms: i64,
    pub send_surrounding_text: bool,
    pub send_screen_words: bool,
    pub send_app_name: bool,
}

/// `dev_or_set_custom_cleanup` (D52): the provider selector and the custom
/// server's fields, saved together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CustomCleanupForm {
    /// `"openrouter"` | `"custom"` (serde of [`CleanupProvider`]).
    pub provider: CleanupProvider,
    /// As typed; `""` = not set yet (Custom is then saved but not used).
    pub base_url: String,
    pub model: String,
    /// i64 so 0 or 99999 reach validation instead of a parse error.
    pub timeout_ms: i64,
    /// The consent dialog naming the typed host was accepted just now.
    pub consent: bool,
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

/// Text cleanup card: toggle (with its own consent: OpenRouter's flag in
/// the cleanup order), order, sort, timeout and the three "send …"
/// switches. Turning OpenRouter on keeps its place in the order.
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
        record_cleanup_consent(&mut next.cloud, now_ms);
    }
    // A custom server without its address or model is switched on unused:
    // its consent is asked once it has an address (`apply_custom_cleanup`).
    if f.allow && next.cloud.cleanup_ready() && !next.cloud.cleanup_consent_ok() {
        return Err(format!(
            "Accept sending text to {} before turning this on",
            cleanup_host_name(&next.cloud)
        ));
    }
    if let Some(order) = &f.order {
        reorder_cleanup(&mut next, order);
    }
    next.set_cleanup_on(CleanupRoute::Cloud, f.allow);
    check(&next)?;
    *dev = next;
    Ok(())
}

/// Consent for the selected cleanup provider's host: openrouter.ai, or
/// the custom server's current host (D52).
fn record_cleanup_consent(cloud: &mut flow_router::cloud::CloudConfig, now_ms: u64) {
    match cloud.cleanup_provider {
        CleanupProvider::OpenRouter => {
            cloud.openrouter.cleanup.consent_host = OPENROUTER_HOST.into();
            cloud.openrouter.cleanup.consent_at_ms = now_ms.max(1);
        }
        CleanupProvider::Custom => {
            let cc = &mut cloud.custom_cleanup;
            let host = cc.host();
            if !host.is_empty() {
                cc.consent_host = host;
                cc.consent_at_ms = now_ms.max(1);
            }
        }
    }
}

/// How an error names where cleanup text goes.
fn cleanup_host_name(cloud: &flow_router::cloud::CloudConfig) -> String {
    match cloud.cleanup_provider {
        CleanupProvider::OpenRouter => "OpenRouter".into(),
        CleanupProvider::Custom => match cloud.cleanup_host() {
            h if h.is_empty() => "your server".into(),
            h => h,
        },
    }
}

/// D52 provider selector + custom server card: base URL (normalised and
/// stored as normalised), model id (free text), timeout, and consent for
/// the typed URL's host. Custom is saved as soon as it is picked, so the
/// selector always shows what dictation uses; until its address and model
/// are set the router skips it ([`flow_router::cloud::CustomCleanupConfig::ready`]).
/// While the cloud cleanup switch is on, a ready provider needs its own
/// consent first.
pub fn apply_custom_cleanup(
    dev: &mut DevSettings,
    f: &CustomCleanupForm,
    now_ms: u64,
) -> Result<(), String> {
    let mut next = dev.clone();
    next.cloud.cleanup_provider = f.provider;
    let custom = !f.provider.is_openrouter();
    let cc = &mut next.cloud.custom_cleanup;
    let url = f.base_url.trim();
    cc.base_url = if url.is_empty() {
        String::new()
    } else {
        normalize_custom_base_url(url).map_err(|e| format!("Base URL: {e}"))?
    };
    let model = f.model.trim();
    if !model.is_empty() && !valid_custom_model_id(model) {
        return Err("Model id: up to 256 characters, no spaces".into());
    }
    cc.model = model.to_string();
    let (lo, hi) = CUSTOM_CLEANUP_TIMEOUT_RANGE_MS;
    if !(lo as i64..=hi as i64).contains(&f.timeout_ms) {
        return Err(format!("Cleanup time limit must be {lo}-{hi} ms"));
    }
    cc.timeout_ms = f.timeout_ms as u64;
    if f.consent {
        if custom && cc.host().is_empty() {
            return Err("Enter your server's base URL".into());
        }
        record_cleanup_consent(&mut next.cloud, now_ms);
    }
    // A custom server without its address or model yet is saved but not
    // used (the router skips it); consent is asked once it could be used.
    if next.cleanup_on(CleanupRoute::Cloud)
        && next.cloud.cleanup_ready()
        && !next.cloud.cleanup_consent_ok()
    {
        return Err(format!(
            "Accept sending text to {} before using it",
            cleanup_host_name(&next.cloud)
        ));
    }
    check(&next)?;
    *dev = next;
    Ok(())
}

/// D52 "Load models" / "Test connection" with the saved key on a host not
/// consented to yet: the page showed the custom server's consent dialog for
/// `base_url`'s host and it was accepted. Records that consent so the saved
/// key may go there; the provider is not switched. While OpenRouter is the
/// provider, or no address is saved yet, the address is saved too; with a
/// custom server saved, only its own host can be consented to here, so a
/// test never voids the live server's consent.
pub fn apply_custom_key_consent(
    dev: &mut DevSettings,
    base_url: &str,
    now_ms: u64,
) -> Result<(), String> {
    let a = flow_router::cloud::custom_server_address(base_url)
        .map_err(|e| format!("Base URL: {e}"))?;
    let mut next = dev.clone();
    let openrouter = next.cloud.cleanup_provider.is_openrouter();
    let cc = &mut next.cloud.custom_cleanup;
    if openrouter || cc.host().is_empty() {
        cc.base_url = a.url;
    } else if !cc.host().eq_ignore_ascii_case(&a.host) {
        return Err(
            "Save this server address first: cleanup still goes to your saved server".into(),
        );
    }
    cc.consent_host = a.host;
    cc.consent_at_ms = now_ms.max(1);
    check(&next)?;
    *dev = next;
    Ok(())
}

/// D52 `dev_custom_cleanup_address`: [`flow_router::cloud::custom_server_address`]
/// plus `model_ok`, the JSON `flow_ffi::json_api::custom_cleanup_address`
/// gives Android.
pub fn custom_address(base_url: &str, model: &str) -> serde_json::Value {
    let model_ok = valid_custom_model_id(model.trim());
    match flow_router::cloud::custom_server_address(base_url) {
        Ok(a) => serde_json::json!({"url": a.url, "host": a.host, "model_ok": model_ok}),
        Err(e) => serde_json::json!({"error": "bad_base_url", "message": e, "model_ok": model_ok}),
    }
}

/// `order` (completed as [`full_cleanup_order`]) with each engine's flag kept.
fn reorder_cleanup(dev: &mut DevSettings, order: &[CleanupRoute]) {
    let now = dev.cleanup();
    dev.cleanup_engines = full_cleanup_order(order)
        .into_iter()
        .map(|route| CleanupEngine {
            route,
            enabled: now.iter().any(|e| e.route == route && e.enabled),
        })
        .collect();
}

/// D36 cleanup order (AI cleanup page): the order of `engines` and the
/// on/off flags of this PC and your server. OpenRouter keeps its flag: it
/// is switched with its consent ([`apply_cleanup`]).
pub fn apply_cleanup_engines(
    dev: &mut DevSettings,
    engines: &[CleanupEngine],
) -> Result<(), String> {
    let mut next = dev.clone();
    let order: Vec<CleanupRoute> = engines.iter().map(|e| e.route).collect();
    reorder_cleanup(&mut next, &order);
    for e in engines {
        if e.route != CleanupRoute::Cloud {
            next.set_cleanup_on(e.route, e.enabled);
        }
    }
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
#[cfg(feature = "cloud")]
pub const CLEANUP_SAMPLE: &str = flow_cloud::custom::TEST_SAMPLE;
#[cfg(not(feature = "cloud"))]
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
        assert!(!d.cleanup_on(CleanupRoute::Cloud));
        let mut e = dev();
        apply_cleanup(&mut e, &cleanup(true, true), 12).unwrap();
        assert!(e.cleanup_on(CleanupRoute::Cloud) && !e.allow_cloud);
        assert!(
            !e.cloud.openrouter.stt.consent_ok(),
            "cleanup consent does not cover audio"
        );
        assert!(apply_stt(&mut e, &stt(true, false), 13).is_err());
    }

    #[test]
    fn cleanup_order_defaults_and_validation() {
        use CleanupRoute::{Cloud, Local, Server};
        let routes = |d: &DevSettings| d.cleanup().iter().map(|e| e.route).collect::<Vec<_>>();
        let mut d = dev();
        apply_cleanup(&mut d, &cleanup(true, true), 1).unwrap();
        assert_eq!(
            routes(&d),
            vec![Local, Server, Cloud],
            "turning it on keeps its place"
        );
        assert!(d.cleanup().iter().all(|e| e.enabled));
        let mut f = cleanup(true, false);
        f.order = Some(vec![Cloud, Local, Cloud]);
        apply_cleanup(&mut d, &f, 2).unwrap();
        assert_eq!(
            routes(&d),
            vec![Cloud, Local, Server],
            "server after this PC"
        );
        assert!(d.cleanup_effective(ALL));
        apply_cleanup(&mut d, &cleanup(false, false), 2).unwrap();
        assert_eq!(
            routes(&d),
            vec![Cloud, Local, Server],
            "off keeps its place"
        );
        assert!(!d.cleanup_on(Cloud) && !d.cleanup_effective(ALL));
        apply_cleanup(&mut d, &cleanup(true, false), 2).unwrap();
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
    fn the_cleanup_order_page_moves_engines_but_not_the_cloud_switch() {
        use CleanupRoute::{Cloud, Local, Server};
        let e = |route, enabled| CleanupEngine { route, enabled };
        let mut d = dev();
        apply_cleanup_engines(&mut d, &[e(Server, false), e(Cloud, true), e(Local, true)]).unwrap();
        assert_eq!(
            d.cleanup(),
            vec![e(Server, false), e(Cloud, false), e(Local, true)],
            "OpenRouter needs its consent: its flag is not this page's"
        );
        apply_cleanup(&mut d, &cleanup(true, true), 5).unwrap();
        apply_cleanup_engines(&mut d, &[e(Local, true), e(Cloud, false)]).unwrap();
        assert_eq!(
            d.cleanup(),
            vec![e(Local, true), e(Server, false), e(Cloud, true)],
            "a short list is completed; flags are kept"
        );
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
                custom_key: KeyState {
                    set: false,
                    masked: String::new(),
                },
            },
        );
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["cleanup"]["provider"], "openrouter");
        assert_eq!(j["cleanup"]["custom"]["base_url"], "");
        assert_eq!(j["cleanup"]["custom"]["key"]["set"], false);
        assert_eq!(
            j["key"],
            serde_json::json!({"set": true, "masked": "••••cdef"})
        );
        assert_eq!(j["stt"]["model"], "openai/whisper-large-v3-turbo");
        assert_eq!(j["cleanup"]["model"], "meta-llama/llama-3.3-70b-instruct");
        assert_eq!(
            j["cleanup"]["engines"],
            serde_json::json!([
                {"route": "local", "enabled": true},
                {"route": "server", "enabled": true},
                {"route": "cloud", "enabled": false}
            ])
        );
        assert_eq!(j["cleanup"]["allow"], false);
        assert_eq!(
            j["privacy"],
            serde_json::json!({"zdr": true, "data_collection": "deny"})
        );
        assert!(v.notice.is_some());
        ack_notice(&mut d);
        assert_eq!(d.cloud_notice, None);
    }

    fn custom(provider: &str, url: &str, model: &str, consent: bool) -> CustomCleanupForm {
        CustomCleanupForm {
            provider: serde_json::from_value(serde_json::json!(provider)).unwrap(),
            base_url: url.into(),
            model: model.into(),
            timeout_ms: 4000,
            consent,
        }
    }

    #[test]
    fn key_consent_from_a_test_lets_the_saved_key_go_to_that_host() {
        // Owner report (1.1.2): URL typed, key saved, consent accepted, but
        // "Test connection" sent no key (401): nothing had recorded consent
        // for the typed host while Custom was not saved yet.
        let url = "http://100.64.0.99:4100/v1";
        let saved = || Some("sk-saved-LEAKCHECK".to_string());
        let mut d = dev();
        assert!(crate::credentials::saved_custom_key_withheld(
            None,
            true,
            &d.cloud.custom_cleanup,
            url
        ));
        apply_custom_key_consent(&mut d, url, 7).unwrap();
        assert_eq!(d.cloud.cleanup_provider, CleanupProvider::OpenRouter);
        assert_eq!(d.cloud.custom_cleanup.base_url, url);
        let cc = &d.cloud.custom_cleanup;
        assert!(!crate::credentials::saved_custom_key_withheld(
            None, true, cc, url
        ));
        assert_eq!(
            crate::credentials::custom_key_for(None, saved, cc, url).unwrap(),
            Some("sk-saved-LEAKCHECK".into())
        );
        // The later switch to Custom needs no second consent for that host.
        apply_custom_cleanup(&mut d, &custom("custom", url, "m", false), 9).unwrap();
        assert!(d.cloud.cleanup_consent_ok());
        // With Custom live, a test of another host cannot move its consent.
        assert!(apply_custom_key_consent(&mut d, "http://100.64.0.98:4100", 11).is_err());
        assert_eq!(d.cloud.custom_cleanup.consent_host, "100.64.0.99");
        assert!(apply_custom_key_consent(&mut d, "not a url", 11).is_err());
    }

    #[test]
    fn custom_address_is_the_router_normaliser() {
        let v = custom_address("http://[2001:DB8::0:1]:4000/", "m-1");
        assert_eq!(v["url"], "http://[2001:db8::1]:4000/v1");
        assert_eq!(v["host"], "2001:db8::1");
        assert_eq!(v["model_ok"], true);
        let v = custom_address("http://x/v1?k=1", "two words");
        assert_eq!(v["error"], "bad_base_url");
        assert_eq!(v["model_ok"], false);
    }

    #[test]
    fn custom_is_saved_when_picked_and_used_once_ready() {
        let mut d = dev();
        // The provider is typed: an unknown one never reaches the settings.
        assert!(serde_json::from_value::<CustomCleanupForm>(
            serde_json::json!({"provider": "lmstudio", "timeout_ms": 4000})
        )
        .is_err());
        // Owner report (1.1.2): Custom picked, address typed, no model yet.
        // It is saved as picked, so the selector (and a reload after the
        // key's Save) shows Custom, and the router skips it until ready.
        apply_custom_cleanup(&mut d, &custom("custom", "", "", false), 5).unwrap();
        assert_eq!(d.cloud.cleanup_provider, CleanupProvider::Custom);
        assert!(!d.cloud.cleanup_ready());
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "", false),
            5,
        )
        .unwrap();
        assert_eq!(d.cloud.cleanup_provider, CleanupProvider::Custom);
        assert_eq!(d.cloud.custom_cleanup.base_url, "http://10.0.0.5:4000/v1");
        assert!(!d.cloud.cleanup_ready());
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000/v1", "gpt-4o-mini", false),
            5,
        )
        .unwrap();
        assert!(d.cloud.cleanup_ready());
        apply_custom_cleanup(&mut d, &custom("openrouter", "", "", false), 5).unwrap();
        assert_eq!(d.cloud.cleanup_provider, CleanupProvider::OpenRouter);
        assert!(d.cloud.custom_cleanup.base_url.is_empty());
    }

    #[test]
    fn a_ready_custom_server_needs_consent_while_cleanup_is_on() {
        let mut d = dev();
        d.cloud.openrouter.cleanup.consent_host = OPENROUTER_HOST.into();
        d.cloud.openrouter.cleanup.consent_at_ms = 1;
        d.set_cleanup_on(CleanupRoute::Cloud, true);
        // Not ready yet: saved without asking.
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "", false),
            5,
        )
        .unwrap();
        // Ready on a host not agreed to: refused until consent comes.
        let ready = custom("custom", "http://10.0.0.5:4000", "m", false);
        assert!(apply_custom_cleanup(&mut d, &ready, 6).is_err());
        assert!(
            d.cloud.custom_cleanup.model.is_empty(),
            "refused = unchanged"
        );
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "m", true),
            7,
        )
        .unwrap();
        assert!(d.cloud.cleanup_ready() && d.cloud.cleanup_consent_ok());
    }

    #[test]
    fn custom_urls_are_normalised_and_checked() {
        let mut d = dev();
        for (typed, saved) in [
            ("  http://10.0.0.5:4000  ", "http://10.0.0.5:4000/v1"),
            ("https://llm.example.test/", "https://llm.example.test/v1"),
            (
                "https://llm.example.test/api/v1/",
                "https://llm.example.test/api/v1",
            ),
        ] {
            apply_custom_cleanup(&mut d, &custom("custom", typed, "m", false), 5).unwrap();
            assert_eq!(d.cloud.custom_cleanup.base_url, saved, "{typed}");
        }
        for bad in [
            "ftp://llm.example.test",
            "llm.example.test:4000",
            "https://user:pw@llm.example.test",
            "https://llm.example.test/v1?x=1",
        ] {
            let e = apply_custom_cleanup(&mut d, &custom("custom", bad, "m", false), 5);
            assert!(e.unwrap_err().starts_with("Base URL"), "{bad}");
        }
        assert_eq!(
            d.cloud.custom_cleanup.base_url,
            "https://llm.example.test/api/v1"
        );
        let mut f = custom("custom", "http://10.0.0.5:4000", "m", false);
        for t in [0, 799, 30_001] {
            f.timeout_ms = t;
            assert!(apply_custom_cleanup(&mut d, &f, 5).is_err(), "{t}");
        }
        f.timeout_ms = 30_000;
        apply_custom_cleanup(&mut d, &f, 5).unwrap();
        assert_eq!(d.cloud.custom_cleanup.timeout_ms, 30_000);
    }

    #[test]
    fn custom_model_ids_are_free_text() {
        let mut d = dev();
        for id in ["gpt-4o", "Qwen/Qwen2.5-7B-Instruct", "ollama/llama3:8b"] {
            apply_custom_cleanup(
                &mut d,
                &custom("custom", "http://10.0.0.5:4000", id, false),
                5,
            )
            .unwrap();
            assert_eq!(
                d.cloud.custom_cleanup.model, id,
                "case and punctuation kept"
            );
        }
        for bad in ["two words", "tab\there", &"x".repeat(257)] {
            assert!(apply_custom_cleanup(
                &mut d,
                &custom("custom", "http://10.0.0.5:4000", bad, false),
                5
            )
            .is_err());
        }
    }

    #[test]
    fn custom_consent_is_bound_to_the_typed_host() {
        let mut d = dev();
        // OpenRouter consent and switch on.
        apply_cleanup(&mut d, &cleanup(true, true), 10).unwrap();
        // Switching to Custom while on asks for the server's own consent.
        let e = apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "m", false),
            20,
        )
        .unwrap_err();
        assert!(e.contains("10.0.0.5"), "{e}");
        assert_eq!(d.cloud.cleanup_provider, CleanupProvider::OpenRouter);
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "m", true),
            20,
        )
        .unwrap();
        let cc = &d.cloud.custom_cleanup;
        assert_eq!(
            (cc.consent_host.as_str(), cc.consent_at_ms),
            ("10.0.0.5", 20)
        );
        assert!(
            d.cloud.openrouter.cleanup.consent_ok(),
            "OpenRouter's is kept"
        );
        // Same host, other port/path: still consented.
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:8000/api/v1", "m", false),
            30,
        )
        .unwrap();
        // Another host voids it: refused while on…
        let e = apply_custom_cleanup(
            &mut d,
            &custom("custom", "https://llm.example.test", "m", false),
            40,
        )
        .unwrap_err();
        assert!(e.contains("llm.example.test"), "{e}");
        // …and when saved with the switch off, the view shows no consent.
        let mut off = d.clone();
        off.set_cleanup_on(CleanupRoute::Cloud, false);
        apply_custom_cleanup(
            &mut off,
            &custom("custom", "https://llm.example.test", "m", false),
            40,
        )
        .unwrap();
        assert!(!off.cloud.custom_cleanup.consent_ok());
        assert!(apply_cleanup(&mut off, &cleanup(true, false), 50).is_err());
        apply_cleanup(&mut off, &cleanup(true, true), 50).unwrap();
        assert_eq!(off.cloud.custom_cleanup.consent_host, "llm.example.test");
        assert_eq!(
            off.cloud.openrouter.cleanup.consent_at_ms, 10,
            "custom consent never re-dates OpenRouter's"
        );
    }

    #[test]
    fn the_openrouter_path_is_unchanged() {
        let mut d = dev();
        apply_custom_cleanup(
            &mut d,
            &custom("openrouter", "http://10.0.0.5:4000", "m", false),
            5,
        )
        .unwrap();
        let e = apply_cleanup(&mut d, &cleanup(true, false), 10).unwrap_err();
        assert_eq!(
            e,
            "Accept sending text to OpenRouter before turning this on"
        );
        apply_cleanup(&mut d, &cleanup(true, true), 10).unwrap();
        let c = &d.cloud.openrouter.cleanup;
        assert_eq!(
            (c.consent_host.as_str(), c.consent_at_ms),
            (OPENROUTER_HOST, 10)
        );
        assert!(d.cloud.custom_cleanup.consent_host.is_empty());
        assert!(d.cleanup_on(CleanupRoute::Cloud));
        // Back from Custom to OpenRouter needs no new consent: it has one.
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "m", true),
            20,
        )
        .unwrap();
        apply_custom_cleanup(
            &mut d,
            &custom("openrouter", "http://10.0.0.5:4000", "m", false),
            30,
        )
        .unwrap();
        assert_eq!(d.cloud.cleanup_model(), "meta-llama/llama-3.3-70b-instruct");
    }

    #[test]
    fn the_custom_view_flags_plain_http_and_never_holds_the_key() {
        let mut d = dev();
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "http://10.0.0.5:4000", "m", true),
            5,
        )
        .unwrap();
        let v = custom_view(
            &d.cloud.custom_cleanup,
            KeyState {
                set: true,
                masked: "••••0001".into(),
            },
        );
        assert!(v.plain_http && v.consent);
        assert_eq!(v.host, "10.0.0.5");
        let j = serde_json::to_string(&v).unwrap();
        assert!(j.contains("••••0001") && !j.contains("sk-"));
        apply_custom_cleanup(
            &mut d,
            &custom("custom", "https://llm.example.test", "m", false),
            5,
        )
        .unwrap();
        let v = custom_view(&d.cloud.custom_cleanup, v.key);
        assert!(!v.plain_http && !v.consent);
    }
}
