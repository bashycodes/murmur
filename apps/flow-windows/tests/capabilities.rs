//! Review S18: every Tauri command is declared in build.rs (so it gets an
//! `allow-*` permission), the settings window is granted exactly the app's
//! commands, the overlay window only `overlay_action`, and the Scratchpad
//! only its note commands plus `clipboard_copy`.

/// The Scratchpad's note commands: no other window needs them.
const SCRATCHPAD_ONLY: &[&str] = &["note_load", "note_edit", "note_flush"];

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
    assert_eq!(
        permissions("capabilities/overlay.json"),
        vec!["overlay_action"]
    );
    let v: serde_json::Value = serde_json::from_str(&read("capabilities/overlay.json")).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["overlay"]));
}

#[test]
fn settings_gets_the_app_commands_but_not_the_overlay_click() {
    let mut granted = permissions("capabilities/default.json");
    granted.sort();
    let mut want: Vec<String> = handler_commands()
        .into_iter()
        .filter(|c| c != "overlay_action" && !SCRATCHPAD_ONLY.contains(&c.as_str()))
        .collect();
    want.sort();
    assert_eq!(granted, want);
    let v: serde_json::Value = serde_json::from_str(&read("capabilities/default.json")).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["settings"]));
}

#[test]
fn scratchpad_may_only_edit_its_note_and_copy() {
    let mut granted = permissions("capabilities/scratchpad.json");
    granted.sort();
    let mut want: Vec<String> = SCRATCHPAD_ONLY.iter().map(|c| c.to_string()).collect();
    want.push("clipboard_copy".into());
    want.sort();
    assert_eq!(granted, want);
    let v: serde_json::Value = serde_json::from_str(&read("capabilities/scratchpad.json")).unwrap();
    assert_eq!(v["windows"], serde_json::json!(["scratchpad"]));
    let handlers = handler_commands();
    for c in SCRATCHPAD_ONLY {
        assert!(handlers.iter().any(|h| h == c), "{c} is not a command");
    }
}
