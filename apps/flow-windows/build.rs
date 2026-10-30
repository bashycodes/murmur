fn main() {
    // D26: the installer version is this package's (Tauri reads Cargo.toml);
    // it must equal the repo's VERSION (tools/release.sh stamps both).
    println!("cargo:rerun-if-changed=../../VERSION");
    if let Ok(v) = std::fs::read_to_string("../../VERSION") {
        let pkg = std::env::var("CARGO_PKG_VERSION").unwrap();
        if v.trim() != pkg {
            panic!(
                "VERSION says {} but apps/flow-windows/Cargo.toml says {pkg}: run tools/release.sh",
                v.trim()
            );
        }
    }
    #[cfg(feature = "app")]
    {
        // Every app command gets an `allow-<command>` permission, so each
        // window can be granted only what it uses (capabilities/*.json;
        // review S18: the overlay may invoke `overlay_action` / `overlay_theme`
        // only, not the history or cloud-key commands).
        const COMMANDS: &[&str] = &[
            "get_prefs",
            "set_prefs",
            "list_microphones",
            "get_model_status",
            "overlay_action",
            "overlay_theme",
            "history_list",
            "history_delete",
            "history_clear",
            "history_copy",
            "history_repaste",
            "history_retry",
            "insights_get",
            "insights_reset",
            "clipboard_copy",
            "note_load",
            "note_edit",
            "note_flush",
            "note_list",
            "note_create",
            "note_delete",
            "note_set_pinned",
            "note_export",
            "note_open",
            "suggestions_list",
            "suggestion_accept",
            "suggestion_dismiss",
            "app_style_choices",
            "recent_apps_clear",
            "dictionary_import",
            "dictionary_export",
            "dictionary_export_text",
            "pick_path",
            "open_external",
            "model_catalog",
            "llm_backend",
            "model_download_start",
            "model_download_cancel",
            "model_delete",
            "transforms_reset",
            "transform_try",
            "hotkey_capture_start",
            "hotkey_capture_stop",
            "dev_get",
            "dev_validate",
            "dev_set",
            "dev_test_connection",
            "dev_test_session",
            "dev_route_status",
            "dev_reset_circuit",
            "dev_reset_defaults",
            "dev_unlock",
            "dev_cloud_set_key",
            "dev_cloud_clear_key",
            "dev_cloud_usage",
            "dev_cloud_reset_usage",
            "dev_or_get",
            "dev_or_catalog",
            "dev_or_catalog_status",
            "dev_or_refresh_catalog",
            "dev_or_key_info",
            "dev_or_set_model",
            "dev_or_set_stt",
            "dev_or_set_cleanup",
            "dev_cleanup_set_engines",
            "dev_or_set_privacy",
            "dev_or_set_budget",
            "dev_or_ack_notice",
            "dev_or_test_stt",
            "dev_or_test_cleanup",
            "dev_or_cleanup_status",
            "debug_log_status",
            "debug_log_set",
            "debug_log_clear",
            "debug_log_open_folder",
        ];
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
        )
        .expect("tauri-build");
    }
}
