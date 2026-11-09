//! The single provider entry (D28 §2.1): presets, the `custom` base URL and
//! the static price table are gone. `presets()` stays for the shells'
//! existing `nativeCloudPresets` / Developer-page call sites and returns
//! exactly one OpenRouter entry. Prices come from the catalogue
//! (`crate::budget`).

use flow_router::CloudProviderConfig;

use crate::api::BASE_URL;
use crate::settings::DEFAULT_STT_MODEL;

pub fn presets() -> Vec<CloudProviderConfig> {
    vec![CloudProviderConfig {
        preset: "openrouter".into(),
        adapter: "openrouter".into(),
        label: "OpenRouter".into(),
        base_url: BASE_URL.into(),
        model: DEFAULT_STT_MODEL.into(),
        zero_retention: true,
        ..Default::default()
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_openrouter_preset() {
        let p = presets();
        assert_eq!(p.len(), 1);
        let mut p = p.into_iter().next().unwrap();
        assert_eq!(
            (p.adapter.as_str(), p.base_url.as_str()),
            ("openrouter", BASE_URL)
        );
        p.id = "openrouter".into();
        assert_eq!(p.validate(), Ok(()));
    }
}
