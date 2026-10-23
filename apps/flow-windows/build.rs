fn main() {
    #[cfg(feature = "app")]
    {
        // Every app command gets an `allow-<command>` permission, so each
        // window can be granted only what it uses (capabilities/*.json;
        // review S18: the overlay may invoke `overlay_action` only, not the
        // history or cloud-key commands).
        const COMMANDS: &[&str] = &[
            "get_prefs",
            "set_prefs",
            "list_microphones",
            "get_model_status",
            "overlay_action",
            "history_list",
            "history_delete",
            "history_clear",
            "history_copy",
            "history_repaste",
            "clipboard_copy",
            "suggestions_list",
            "suggestion_accept",
            "suggestion_dismiss",
            "dictionary_import",
            "dictionary_export",
            "dictionary_export_text",
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
            "dev_cloud_test",
            "dev_cloud_usage",
            "dev_cloud_reset_usage",
            "dev_cloud_presets",
            "dev_cloud_new_id",
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
