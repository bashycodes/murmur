//! Review S18: every Tauri command is declared in build.rs (so it gets an
//! `allow-*` permission), the settings (app) window is granted exactly the
//! app's commands, the overlay window only `overlay_action` and
//! `overlay_theme` (the theme pref, not `get_prefs`), and the Scratchpad
//! only its note commands plus `clipboard_copy`.

/// The note editor's commands: only the Scratchpad / Notes window reads,
/// writes, deletes or exports a note's text.
const SCRATCHPAD_ONLY: &[&str] = &[
    "note_load",
    "note_edit",
    "note_flush",
    "note_delete",
    "note_export",
];

/// The note list commands both windows use: the Notes list in the
/// Scratchpad window and the app window's Scratchpad page.
const NOTES_SHARED: &[&str] = &["note_list", "note_create", "note_set_pinned"];

/// The app window's Scratchpad page opens a note in the notes window; the
/// notes window navigates itself instead.
const SETTINGS_ONLY_NOTES: &[&str] = &["note_open"];

/// The Flow bar's commands: its clicks / link box, and the theme pref
/// (never all of `get_prefs`). No other window needs them.
const OVERLAY_ONLY: &[&str] = &["overlay_action", "overlay_theme"];

fn read(p: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(p)).unwrap()
}

fn handler_commands() -> Vec<String> {
    let app = read("src/app.rs");
    let blk = &app[app.find("generate_handler![").unwrap()..];
    let blk = &blk["generate_handler![".len()..blk.find(']').unwrap()];
    let mut v: Vec<String> = blk
        .split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    v.sort();
    v
}

fn permissions(file: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(&read(file)).unwrap();
    v["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str())
        .filter_map(|p| p.strip_prefix("allow-"))
        .map(|p| p.replace('-', "_"))
        .collect()
}

#[test]
fn build_rs_declares_every_command() {
    let build = read("build.rs");
    for c in handler_commands() {
        assert!(
            build.contains(&format!("\"{c}\"")),
            "{c} missing in build.rs"
        );
    }
}

#[test]
fn overlay_may_only_click_cancel_or_finish() {
    assert_eq!(permissions("capabilities/overlay.json"), OVERLAY_ONLY);
    let v: serde_json::Value = serde_json::from_str(&read("capabilities/overlay.json")).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["overlay"]));
}

#[test]
fn settings_gets_the_app_commands_but_not_the_overlay_click_or_note_text() {
    let mut granted = permissions("capabilities/default.json");
    granted.sort();
    let mut want: Vec<String> = handler_commands()
        .into_iter()
        .filter(|c| !OVERLAY_ONLY.contains(&c.as_str()) && !SCRATCHPAD_ONLY.contains(&c.as_str()))
        .collect();
    want.sort();
    assert_eq!(granted, want);
    for c in NOTES_SHARED.iter().chain(SETTINGS_ONLY_NOTES) {
        assert!(granted.iter().any(|g| g == c), "{c} not granted");
    }
    let v: serde_json::Value = serde_json::from_str(&read("capabilities/default.json")).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["settings"]));
}

#[test]
fn scratchpad_may_only_edit_its_notes_and_copy() {
    let mut granted = permissions("capabilities/scratchpad.json");
    granted.sort();
    let mut want: Vec<String> = SCRATCHPAD_ONLY
        .iter()
        .chain(NOTES_SHARED)
        .map(|c| c.to_string())
        .collect();
    want.push("clipboard_copy".into());
    want.sort();
    assert_eq!(granted, want);
    let v: serde_json::Value = serde_json::from_str(&read("capabilities/scratchpad.json")).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["scratchpad"]));
    let handlers = handler_commands();
    for c in SCRATCHPAD_ONLY
        .iter()
        .chain(NOTES_SHARED)
        .chain(SETTINGS_ONLY_NOTES)
    {
        assert!(handlers.iter().any(|h| h == c), "{c} is not a command");
    }
}

#[test]
fn every_note_command_has_one_owner_list() {
    let handlers = handler_commands();
    for h in handlers.iter().filter(|h| h.starts_with("note_")) {
        let n = [SCRATCHPAD_ONLY, NOTES_SHARED, SETTINGS_ONLY_NOTES]
            .iter()
            .filter(|l| l.contains(&h.as_str()))
            .count();
        assert_eq!(n, 1, "{h} must be in exactly one note list");
    }
}
