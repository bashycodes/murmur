//! The NSIS uninstaller's "Delete the application data" box must also
//! remove the app's own data folders (`%APPDATA%\Murmur` and the pre-D83
//! `%APPDATA%\FlowClone`), not only Tauri's `%APPDATA%\<identifier>`, and
//! only when ticked and not during an update. The hook is NSIS, so this
//! checks the wiring and the folder names, not the uninstaller itself.

use std::path::Path;

fn read(p: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(p)).unwrap()
}

/// The string value of `pub const <name>: &str = "...";` in a source file.
fn str_const(file: &str, name: &str) -> String {
    let src = read(file);
    let decl = format!("pub const {name}: &str = \"");
    let start = src
        .find(&decl)
        .unwrap_or_else(|| panic!("{name} in {file}"))
        + decl.len();
    src[start..start + src[start..].find('"').unwrap()].to_string()
}

fn hook_path() -> String {
    let conf: serde_json::Value = serde_json::from_str(&read("tauri.conf.json")).unwrap();
    conf["bundle"]["windows"]["nsis"]["installerHooks"]
        .as_str()
        .expect("bundle.windows.nsis.installerHooks in tauri.conf.json")
        .to_string()
}

/// The body of `!macro NSIS_HOOK_POSTUNINSTALL`.
fn post_uninstall() -> String {
    let hook = read(&hook_path());
    let start = hook
        .find("!macro NSIS_HOOK_POSTUNINSTALL")
        .expect("NSIS_HOOK_POSTUNINSTALL macro");
    let body = &hook[start..];
    body[..body.find("!macroend").unwrap()].to_string()
}

#[test]
fn uninstall_hook_is_wired_into_the_bundle() {
    assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(hook_path())
        .is_file());
}

#[test]
fn delete_app_data_removes_the_app_folders() {
    let body = post_uninstall();
    for dir in [
        str_const("src/prefs.rs", "APP_DIR"),
        str_const("src/old_data.rs", "OLD_APP_DIR"),
    ] {
        let rm = format!("RmDir /r \"$APPDATA\\{dir}\"");
        assert!(body.contains(&rm), "hook lacks {rm}");
    }
}

#[test]
fn deletes_only_when_ticked_and_not_updating() {
    let body = post_uninstall();
    // Variable names from tauri-bundler's installer.nsi (2.10.4).
    let guard = body
        .find("${If} $DeleteAppDataCheckboxState = 1")
        .expect("checkbox guard");
    let update = body
        .find("${AndIf} $UpdateMode <> 1")
        .expect("update-mode guard");
    let first_rm = body.find("RmDir").unwrap();
    assert!(
        guard < update && update < first_rm,
        "RmDir outside the guard"
    );
    assert!(body[first_rm..].contains("${EndIf}"));
    assert_eq!(body.matches("RmDir").count(), 2, "only the two app folders");
}
