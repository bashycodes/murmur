//! Transforms settings (docs/proposals/transforms.md): the master opt-in,
//! the auto-apply choice, the "view changes" shortcut and the saved
//! transforms with their shortcuts. Stored in `prefs.json` as
//! `transforms`; the instructions themselves run through
//! `murmur_core::transform`.

use crate::hotkeys::{Chord, Key};
use murmur_core::transform::{
    self as core, Transform, MAX_DESCRIPTION_CHARS, MAX_INSTRUCTION_CHARS, MAX_NAME_CHARS,
    MAX_TRANSFORMS,
};
use serde::{Deserialize, Serialize};

/// One saved transform and its shortcut (`None` = none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub transform: Transform,
    #[serde(default, deserialize_with = "crate::hotkeys::chord::optional_chord")]
    pub shortcut: Option<Chord>,
}

/// `prefs.transforms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransformPrefs {
    /// Master "Opt in" (off by default): when off no transform shortcut
    /// is registered and auto-apply never runs.
    pub enabled: bool,
    /// Transform id run on every dictation before it is pasted.
    pub auto_apply: Option<String>,
    /// Opens Home with the last transform's before / after.
    #[serde(deserialize_with = "crate::hotkeys::chord::optional_chord")]
    pub view_changes: Option<Chord>,
    pub items: Vec<Item>,
}

/// Win+Alt+`c` (the design's defaults: Win+Alt+1, 2 … and Win+Alt+O).
fn win_alt(c: char) -> Chord {
    Chord::new([Key::Win, Key::Alt, Key::Vk(c as u16)])
}

/// The built-ins with their default shortcuts Win+Alt+1, 2.
pub fn default_items() -> Vec<Item> {
    core::builtins()
        .into_iter()
        .enumerate()
        .map(|(i, transform)| Item {
            transform,
            shortcut: char::from_digit(i as u32 + 1, 10).map(win_alt),
        })
        .collect()
}

impl Default for TransformPrefs {
    fn default() -> Self {
        TransformPrefs {
            enabled: false,
            auto_apply: None,
            view_changes: Some(win_alt('O')),
            items: default_items(),
        }
    }
}

impl TransformPrefs {
    /// Auto-apply runs: Transforms is on and the chosen transform exists.
    pub fn auto_apply_item(&self) -> Option<&Item> {
        if !self.enabled {
            return None;
        }
        let id = self.auto_apply.as_deref()?;
        self.items.iter().find(|i| i.transform.id == id)
    }

    pub fn find(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.transform.id == id)
    }

    /// The chords to register: one per item (in order) and the view
    /// chord, all empty while Transforms is off.
    pub fn chords(&self) -> (Vec<Option<Chord>>, Option<Chord>) {
        if !self.enabled {
            return (Vec::new(), None);
        }
        (
            self.items.iter().map(|i| i.shortcut.clone()).collect(),
            self.view_changes.clone(),
        )
    }

    /// "Reset to defaults": the built-ins come back as shipped (their
    /// shortcuts too, unless a custom transform took one since); custom
    /// transforms stay after them.
    pub fn reset_builtins(&mut self) {
        let custom: Vec<Item> = self
            .items
            .drain(..)
            .filter(|i| !core::is_builtin(&i.transform.id))
            .collect();
        let taken: Vec<&Chord> = custom.iter().filter_map(|i| i.shortcut.as_ref()).collect();
        let mut items = default_items();
        for i in &mut items {
            if i.shortcut.as_ref().is_some_and(|c| taken.contains(&c)) {
                i.shortcut = None;
            }
        }
        items.extend(custom);
        items.truncate(MAX_TRANSFORMS);
        self.items = items;
        if self
            .auto_apply
            .as_deref()
            .is_some_and(|id| self.find(id).is_none())
        {
            self.auto_apply = None;
        }
    }

    /// Tidy a loaded or submitted list: trimmed text, an id for every item
    /// (unique), an auto-apply choice that exists.
    pub fn normalize(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        for (n, it) in self.items.iter_mut().enumerate() {
            let t = &mut it.transform;
            t.name = t.name.trim().to_string();
            t.description = t.description.trim().to_string();
            t.instruction = t.instruction.trim().to_string();
            let mut id = slug(t.id.trim());
            if id.is_empty() {
                id = slug(&t.name);
            }
            if id.is_empty() {
                id = format!("transform-{}", n + 1);
            }
            let base = id.clone();
            let mut k = 2;
            while seen.contains(&id) {
                id = format!("{base}-{k}");
                k += 1;
            }
            seen.push(id.clone());
            t.id = id;
        }
        if self
            .auto_apply
            .as_deref()
            .is_some_and(|id| self.find(id).is_none())
        {
            self.auto_apply = None;
        }
    }

    /// Reject what the editor must not save: too many items, empty or
    /// over-long fields, malformed shortcuts. Chord clashes with the other
    /// shortcuts are checked by `Prefs::validate`.
    pub fn validate(&self) -> Result<(), String> {
        if self.items.len() > MAX_TRANSFORMS {
            return Err(format!("at most {MAX_TRANSFORMS} transforms"));
        }
        for it in &self.items {
            let t = &it.transform;
            let name = if t.name.trim().is_empty() {
                return Err("every transform needs a name".into());
            } else {
                t.name.trim()
            };
            if t.name.chars().count() > MAX_NAME_CHARS {
                return Err(format!(
                    "transform \"{name}\": name is longer than {MAX_NAME_CHARS} characters"
                ));
            }
            if t.description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(format!(
                    "transform \"{name}\": description is longer than {MAX_DESCRIPTION_CHARS} characters"
                ));
            }
            if t.instruction.trim().is_empty() {
                return Err(format!("transform \"{name}\" needs an instruction"));
            }
            if t.instruction.chars().count() > MAX_INSTRUCTION_CHARS {
                return Err(format!(
                    "transform \"{name}\": instruction is longer than {MAX_INSTRUCTION_CHARS} characters"
                ));
            }
            if let Some(c) = &it.shortcut {
                Chord::parse_validated(&c.to_string(), false)
                    .map_err(|e| format!("transform \"{name}\" shortcut: {e}"))?;
            }
        }
        if let Some(c) = &self.view_changes {
            Chord::parse_validated(&c.to_string(), false)
                .map_err(|e| format!("view-changes shortcut: {e}"))?;
        }
        Ok(())
    }

    /// Every transform chord (items, then view changes), for the clash
    /// check across all shortcuts.
    pub fn all_chords(&self) -> Vec<&Chord> {
        self.items
            .iter()
            .filter_map(|i| i.shortcut.as_ref())
            .chain(self.view_changes.as_ref())
            .collect()
    }
}

/// `My Transform!` → `my-transform`.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(id: &str, name: &str, shortcut: Option<&str>) -> Item {
        Item {
            transform: Transform {
                id: id.into(),
                name: name.into(),
                description: String::new(),
                instruction: "do it".into(),
            },
            shortcut: shortcut.map(|s| s.parse().unwrap()),
        }
    }

    #[test]
    fn defaults_are_off_with_the_builtins_on_win_alt_digits() {
        let t = TransformPrefs::default();
        assert!(!t.enabled);
        assert_eq!(t.auto_apply, None);
        assert_eq!(t.view_changes.as_ref().unwrap().to_string(), "Win+Alt+O");
        let names: Vec<_> = t.items.iter().map(|i| i.transform.name.as_str()).collect();
        assert_eq!(names, ["Polish", "Prompt Builder"]);
        let keys: Vec<_> = t
            .items
            .iter()
            .map(|i| i.shortcut.as_ref().unwrap().to_string())
            .collect();
        assert_eq!(keys, ["Win+Alt+1", "Win+Alt+2"]);
        assert_eq!(t.validate(), Ok(()));
        assert_eq!(t.chords(), (Vec::new(), None), "off: nothing registered");
    }

    #[test]
    fn chords_follow_the_items_when_enabled() {
        let mut t = TransformPrefs {
            enabled: true,
            ..TransformPrefs::default()
        };
        t.items.push(custom("x", "X", None));
        let (items, view) = t.chords();
        assert_eq!(items.len(), 3);
        assert!(items[2].is_none());
        assert!(view.is_some());
    }

    #[test]
    fn auto_apply_needs_the_opt_in_and_an_existing_transform() {
        let mut t = TransformPrefs {
            auto_apply: Some(core::POLISH.into()),
            ..TransformPrefs::default()
        };
        assert!(t.auto_apply_item().is_none(), "off");
        t.enabled = true;
        assert_eq!(t.auto_apply_item().unwrap().transform.name, "Polish");
        t.auto_apply = Some("gone".into());
        assert!(t.auto_apply_item().is_none());
        t.normalize();
        assert_eq!(t.auto_apply, None);
    }

    #[test]
    fn reset_restores_builtins_and_keeps_custom_ones() {
        let mut t = TransformPrefs::default();
        t.items[0].transform.instruction = "changed".into();
        t.items.remove(1);
        t.items.push(custom("mine", "Mine", Some("Win+Alt+2")));
        t.reset_builtins();
        let ids: Vec<_> = t.items.iter().map(|i| i.transform.id.as_str()).collect();
        assert_eq!(ids, [core::POLISH, core::PROMPT_ENGINEER, "mine"]);
        assert_eq!(t.items[0], default_items()[0]);
        assert_eq!(
            t.items[1].shortcut, None,
            "Win+Alt+2 is the custom one's now"
        );
        assert_eq!(t.validate(), Ok(()));
    }

    #[test]
    fn normalize_gives_every_item_a_unique_id() {
        let mut t = TransformPrefs {
            items: vec![
                custom("", "  Fix Typos! ", None),
                custom("", "Fix typos", None),
                custom("", "", None),
            ],
            ..TransformPrefs::default()
        };
        t.normalize();
        let ids: Vec<_> = t.items.iter().map(|i| i.transform.id.as_str()).collect();
        assert_eq!(ids, ["fix-typos", "fix-typos-2", "transform-3"]);
        assert_eq!(t.items[0].transform.name, "Fix Typos!");
    }

    #[test]
    fn validate_rejects_bad_items() {
        let bad = |f: &dyn Fn(&mut TransformPrefs)| {
            let mut t = TransformPrefs::default();
            f(&mut t);
            t.validate().unwrap_err()
        };
        assert!(bad(&|t| t.items[0].transform.name = " ".into()).contains("name"));
        assert!(bad(&|t| t.items[0].transform.instruction = "".into()).contains("instruction"));
        assert!(
            bad(&|t| t.items[0].transform.instruction = "x".repeat(MAX_INSTRUCTION_CHARS + 1))
                .contains("longer")
        );
        assert!(bad(&|t| t.items[0].shortcut = Some("A".parse().unwrap())).contains("shortcut"));
        assert!(bad(&|t| {
            for n in 0..MAX_TRANSFORMS {
                t.items.push(custom(&format!("c{n}"), "C", None));
            }
        })
        .contains("at most"));
    }

    #[test]
    fn serde_shape_and_missing_fields() {
        let t: TransformPrefs = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(t.enabled);
        assert_eq!(
            t.items,
            default_items(),
            "a file without items gets the built-ins"
        );
        let t: TransformPrefs = serde_json::from_str(r#"{"items":[],"view_changes":""}"#).unwrap();
        assert!(t.items.is_empty(), "deleting every transform sticks");
        assert_eq!(t.view_changes, None);
        let j = serde_json::to_value(TransformPrefs::default()).unwrap();
        assert_eq!(j["items"][0]["transform"]["id"], "polish");
        assert_eq!(j["items"][0]["shortcut"], "Win+Alt+1");
        assert_eq!(j["view_changes"], "Win+Alt+O");
        assert_eq!(j["auto_apply"], serde_json::Value::Null);
    }
}
