//! Tauri 2 shell (Windows only): tray, overlay pill, settings window, and
//! the Win32 [`Platform`] implementation the controller drives.

use crate::audio::capture::{self, Capture};
use crate::context::{uia, RawSnapshot};
use crate::controller::{AudioSink, Controller, Platform};
use crate::core_adapter::{self, FlowCoreAdapter, LlmSpec, LlmStatus, ModelSpec, ModelStatus};
use crate::dev_settings::{BuildCaps, DevForm, DevSettings, FieldError};
use crate::router_adapter::{self, RouteView, RouterAdapter, SessionTest};
use crate::dictation::{Input, OverlayState, Sound};
use crate::ducking::win::Ducker;
use crate::history::{self, History};
use crate::hotkeys::{self, win::HookHandle};
use crate::paste::{self, PasteOptions};
use crate::prefs::{LoadOutcome, Prefs};
use crate::suggestions::{self as sugg, SuggestionStore};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow, WindowEvent};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, ShowWindow,
    SystemParametersInfoW, GWL_EXSTYLE, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

const OVERLAY: &str = "overlay";
const SETTINGS: &str = "settings";

struct AppState {
    prefs: Arc<RwLock<Prefs>>,
    path: PathBuf,
    appdata: Option<String>,
    hook: Mutex<Option<HookHandle>>,
    /// The local model slot; empty in server-only builds.
    core: OnceLock<Arc<FlowCoreAdapter>>,
    /// Every dictation goes through this (D17, invariant I1).
    router: OnceLock<Arc<RouterAdapter>>,
    /// Controller's "dictation running" flag (Test session must not race it).
    dictating: OnceLock<Arc<std::sync::atomic::AtomicBool>>,
    /// Controller input (overlay ✕/✓ clicks).
    input: OnceLock<mpsc::Sender<Input>>,
    history: Option<Arc<Mutex<History>>>,
    suggestions: Arc<Mutex<SuggestionStore>>,
    /// Tray line "Dictionary suggestions (n)…".
    suggest_item: OnceLock<MenuItem<tauri::Wry>>,
}

fn hwnd_of(w: &WebviewWindow) -> Option<HWND> {
    w.hwnd().ok().map(|h| HWND(h.0 as _))
}

// ---------------------------------------------------------------- platform

struct WinPlatform {
    app: AppHandle,
    prefs: Arc<RwLock<Prefs>>,
    capture: Mutex<Option<Capture>>,
    ducker: Ducker,
    /// Last state sent to the pill (the hover watcher reads it).
    overlay_state: Arc<Mutex<OverlayState>>,
}

impl WinPlatform {
    fn set_overlay_visible(&self, visible: bool) {
        if let Some(w) = self.app.get_webview_window(OVERLAY) {
            if let Some(h) = hwnd_of(&w) {
                unsafe {
                    let _ = ShowWindow(h, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
                }
            }
        }
    }
}

impl Platform for WinPlatform {
    fn play(&self, s: Sound) {
        crate::sounds::win::play(s);
    }

    fn duck(&self) {
        self.ducker.duck(0.0);
    }

    fn unduck(&self) {
        self.ducker.restore();
    }

    fn snapshot_context(&self) -> Option<RawSnapshot> {
        uia::snapshot(Duration::from_millis(300), true)
    }

    fn start_capture(&self, device: Option<String>, sink: AudioSink) -> Result<(), String> {
        let c = Capture::start(device, sink)?;
        *self.capture.lock().unwrap() = Some(c);
        Ok(())
    }

    fn stop_capture(&self) {
        if let Some(c) = self.capture.lock().unwrap().take() {
            c.stop();
        }
    }

    fn paste(&self, text: &str, prefs: &Prefs) -> Result<(), String> {
        paste::win::paste(text, &PasteOptions::from_prefs(prefs))
    }

    fn overlay(&self, st: &OverlayState) {
        *self.overlay_state.lock().unwrap() = st.clone();
        let always = self.prefs.read().unwrap().show_flow_bar_always;
        self.set_overlay_visible(always || *st != OverlayState::Resting);
        let _ = self.app.emit_to(OVERLAY, "dictation:status", st);
    }

    fn level(&self, level: f32) {
        let _ = self.app.emit_to(OVERLAY, "audio:level", level);
    }

    fn max_duration_warning(&self, remaining_ms: u64) {
        let _ = self
            .app
            .emit_to(OVERLAY, "dictation:max-warning", remaining_ms);
    }

    fn preview(&self, text: &str) {
        let _ = self.app.emit_to(OVERLAY, "dictation:preview", text);
    }

    fn send_key(&self, key: crate::core_api::PasteKey) -> Result<(), String> {
        use crate::core_api::PasteKey;
        use crate::input::{vk, Stroke};
        let code = match key {
            PasteKey::Enter => vk::RETURN,
            PasteKey::Tab => vk::TAB,
            PasteKey::Break => return Ok(()),
        };
        let s = [Stroke::down(code), Stroke::up(code)];
        if crate::input::win::send(&s) == s.len() {
            Ok(())
        } else {
            Err(format!("could not send {key:?}"))
        }
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        paste::win::copy_text(text)
    }

    fn field_reader(&self) -> Option<Box<dyn crate::learn::FieldReader>> {
        uia::field_reader().map(|r| Box::new(r) as Box<dyn crate::learn::FieldReader>)
    }

    fn history_changed(&self) {
        let _ = self.app.emit_to(SETTINGS, "history:changed", ());
    }

    fn suggestions_changed(&self, pending: usize) {
        show_suggestion_count(&self.app, pending);
    }
}

/// Non-intrusive notice: a tray line (and a Settings badge via the event);
/// no toast, no focus change. Accepting happens in Settings → Dictionary.
fn show_suggestion_count(app: &AppHandle, pending: usize) {
    let state = app.state::<AppState>();
    if let Some(item) = state.suggest_item.get() {
        let text = match pending {
            0 => "No dictionary suggestions".to_string(),
            1 => "1 dictionary suggestion…".to_string(),
            n => format!("{n} dictionary suggestions…"),
        };
        let _ = item.set_text(text);
        let _ = item.set_enabled(pending > 0);
    }
    let _ = app.emit_to(SETTINGS, "suggestions:changed", pending);
}

// ---------------------------------------------------------------- model notice

const TRAY_TIP: &str = "Flow Clone — hold Ctrl+Win to dictate";

/// Tray notice for the model state: menu line + tooltip; Settings opens on
/// a missing/broken model so the user sees why dictation can't start.
fn apply_model_status(app: &AppHandle, item: &MenuItem<tauri::Wry>, st: &ModelStatus) {
    let file = |p: &str| {
        std::path::Path::new(p)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let (line, tip) = match st {
        ModelStatus::Loading { path } => (
            format!("Speech model: loading {}…", file(path)),
            TRAY_TIP.to_string(),
        ),
        ModelStatus::Ready { path, .. } => (
            format!("Speech model: {}", file(path)),
            TRAY_TIP.to_string(),
        ),
        ModelStatus::Missing { .. } => (
            "⚠ Speech model missing — open Settings".to_string(),
            "Flow Clone — speech model missing (see Settings)".to_string(),
        ),
        ModelStatus::Failed { .. } => (
            "⚠ Speech model failed to load — open Settings".to_string(),
            "Flow Clone — speech model failed to load (see Settings)".to_string(),
        ),
    };
    let _ = item.set_text(line);
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(tip));
    }
    let _ = app.emit_to(SETTINGS, "model:status", st);
    if let Some(problem) = st.problem() {
        if !matches!(st, ModelStatus::Loading { .. }) {
            eprintln!("[model] {problem}");
            show_settings(app);
        }
    }
}

/// Tray line for the optional LLM cleanup (D5: off by default). A failure
/// is a notice only: dictation keeps pasting deterministic output.
fn apply_llm_status(app: &AppHandle, item: &MenuItem<tauri::Wry>, st: &LlmStatus) {
    let file = |p: &str| {
        std::path::Path::new(p)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let line = match st {
        LlmStatus::Off => "AI cleanup: off".to_string(),
        LlmStatus::Loading { path } => format!("AI cleanup: loading {}…", file(path)),
        LlmStatus::Ready { path, .. } => format!("AI cleanup: {}", file(path)),
        LlmStatus::Missing { .. } => "⚠ AI cleanup model missing — open Settings".to_string(),
        LlmStatus::PluginMissing { .. } => {
            format!(
                "⚠ AI cleanup engine ({}) missing",
                core_adapter::llm_plugin_file()
            )
        }
        LlmStatus::Failed { .. } => "⚠ AI cleanup failed to load — open Settings".to_string(),
    };
    let _ = item.set_text(line);
    let _ = app.emit_to(SETTINGS, "llm:status", st);
    if let Some(problem) = st.problem() {
        eprintln!("[llm] {problem}");
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_prefs(state: tauri::State<'_, AppState>) -> Prefs {
    state.prefs.read().unwrap().clone()
}

#[tauri::command]
fn set_prefs(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    mut prefs: Prefs,
) -> Result<(), String> {
    // The Developer tab saves its own section (dev_set); a General/AI save
    // from a page loaded earlier must not roll it back.
    prefs.developer = state.prefs.read().unwrap().developer.clone();
    prefs.dictionary = crate::dictionary::normalize(prefs.dictionary);
    prefs.validate()?;
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    if let Some(h) = state.hook.lock().unwrap().as_ref() {
        h.set_bindings(prefs.shortcuts.clone());
    }
    let _ = autostart::set(prefs.launch_at_login);
    let (reload, reload_llm) = {
        let old = state.prefs.read().unwrap();
        (
            core_adapter::needs_reload(&old, &prefs) || !model_ready(&state),
            core_adapter::needs_llm_reload(&old, &prefs) || llm_wanted_but_down(&state, &prefs),
        )
    };
    if let Some(core) = state.core.get() {
        let ad = state.appdata.as_deref();
        if want_local(&prefs) {
            if reload {
                core.load(ModelSpec::from_prefs(&prefs, ad));
            }
            if reload_llm {
                core.load_llm(LlmSpec::from_prefs(&prefs, ad));
            }
        }
    }
    if let Some(h) = &state.history {
        let _ = h.lock().unwrap().prune(prefs.retention, history::now_ms());
    }
    let pending = {
        let mut s = state.suggestions.lock().unwrap();
        s.data.prune_known(&prefs.dictionary);
        let _ = s.save();
        s.data.pending.len()
    };
    *state.prefs.write().unwrap() = prefs;
    show_suggestion_count(&app, pending);
    Ok(())
}

// ------------------------------------------------- history tab (05 §3, 03 §6)

fn history_db(state: &AppState) -> Result<&Arc<Mutex<History>>, String> {
    state
        .history
        .as_ref()
        .ok_or_else(|| "history is unavailable (see the log)".to_string())
}

#[tauri::command]
fn history_list(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<history::Item>, String> {
    let retention = state.prefs.read().unwrap().retention;
    let rows = history_db(&state)?
        .lock()
        .unwrap()
        .list(&query, limit.unwrap_or(200), retention, history::now_ms())
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(history::Item::from).collect())
}

#[tauri::command]
fn history_delete(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    history_db(&state)?
        .lock()
        .unwrap()
        .delete(id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn history_clear(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    history_db(&state)?
        .lock()
        .unwrap()
        .clear()
        .map_err(|e| e.to_string())
}

/// Row text: the raw ASR text ("undo AI edit") or what was inserted.
fn history_text(state: &AppState, id: i64, raw: bool) -> Result<String, String> {
    let e = history_db(state)?
        .lock()
        .unwrap()
        .get(id)
        .map_err(|e| e.to_string())?
        .ok_or("that transcript is gone")?;
    let t = if raw {
        e.raw.clone()
    } else {
        e.best_text().to_string()
    };
    Ok(paste::plain_text(&t))
}

#[tauri::command]
fn history_copy(state: tauri::State<'_, AppState>, id: i64, raw: bool) -> Result<(), String> {
    paste::win::copy_text(&history_text(&state, id, raw)?)
}

/// Hide Settings so focus returns to the app the user was in, then paste.
#[tauri::command]
fn history_repaste(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
    raw: bool,
) -> Result<(), String> {
    let text = history_text(&state, id, raw)?;
    let prefs = state.prefs.read().unwrap().clone();
    let settings = app.get_webview_window(SETTINGS);
    // As an integer: HWND is not Send.
    let own = settings.as_ref().and_then(hwnd_of).map(|h| h.0 as usize);
    if let Some(w) = &settings {
        let _ = w.hide();
    }
    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(50));
            let fg = unsafe { GetForegroundWindow() };
            if !fg.is_invalid() && Some(fg.0 as usize) != own {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        if let Err(e) = paste::win::paste(&text, &PasteOptions::from_prefs(&prefs)) {
            eprintln!("[history] re-paste: {e}");
        }
    });
    Ok(())
}

#[tauri::command]
fn clipboard_copy(text: String) -> Result<(), String> {
    paste::win::copy_text(&text)
}

// ------------------------------------------------- dictionary tab

#[tauri::command]
fn suggestions_list(state: tauri::State<'_, AppState>) -> Vec<sugg::Suggestion> {
    state.suggestions.lock().unwrap().data.pending.clone()
}

/// Explicit accept: the only way a learned word reaches the dictionary.
#[tauri::command]
fn suggestion_accept(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    word: String,
    as_replacement: bool,
) -> Result<Prefs, String> {
    let entry = state
        .suggestions
        .lock()
        .unwrap()
        .data
        .accept(&word, as_replacement)
        .ok_or("suggestion not found")?;
    let mut prefs = state.prefs.read().unwrap().clone();
    let merged = crate::dictionary::merge(&prefs.dictionary, vec![entry]);
    prefs.dictionary = merged.entries;
    set_prefs(app, state.clone(), prefs)?;
    Ok(state.prefs.read().unwrap().clone())
}

#[tauri::command]
fn suggestion_dismiss(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    word: String,
) -> Result<(), String> {
    let pending = {
        let mut s = state.suggestions.lock().unwrap();
        s.data.dismiss(&word);
        s.save().map_err(|e| e.to_string())?;
        s.data.pending.len()
    };
    show_suggestion_count(&app, pending);
    Ok(())
}

/// Parse an exchange file the UI read and merge it into `entries` (not
/// saved until the user presses Save).
#[tauri::command]
fn dictionary_import(
    entries: Vec<crate::prefs::DictEntry>,
    json: String,
    replace: bool,
) -> Result<crate::dictionary::MergeReport, String> {
    crate::dictionary::import(&entries, &json, replace)
}

/// Write the exchange JSON to the Downloads folder; returns the path.
#[tauri::command]
fn dictionary_export(entries: Vec<crate::prefs::DictEntry>) -> Result<String, String> {
    let base = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    let dir = [base.join("Downloads"), base.join("Documents"), base]
        .into_iter()
        .find(|d| d.is_dir())
        .ok_or("no Downloads folder")?;
    let path = dir.join("flow-dictionary.json");
    std::fs::write(&path, crate::dictionary::export(&entries)).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn dictionary_export_text(entries: Vec<crate::prefs::DictEntry>) -> String {
    crate::dictionary::export(&entries)
}

fn model_ready(state: &AppState) -> bool {
    state.core.get().is_some_and(|c| c.status().is_ready())
}

/// Saving again retries a failed LLM load (e.g. after copying the file in).
fn llm_wanted_but_down(state: &AppState, prefs: &Prefs) -> bool {
    prefs.llm_enabled
        && state
            .core
            .get()
            .is_some_and(|c| c.llm_status().problem().is_some())
}

#[derive(serde::Serialize)]
struct ModelInfo {
    status: ModelStatus,
    default_path: String,
    default_parakeet_dir: String,
    models_dir: String,
    llm: LlmStatus,
    default_llm_path: String,
    llm_plugin: &'static str,
}

#[tauri::command]
fn get_model_status(state: tauri::State<'_, AppState>) -> ModelInfo {
    let ad = state.appdata.as_deref();
    ModelInfo {
        status: state
            .core
            .get()
            .map(|c| c.status())
            .unwrap_or(ModelStatus::Loading {
                path: String::new(),
            }),
        default_path: core_adapter::default_model_path(ad).display().to_string(),
        default_parakeet_dir: core_adapter::default_parakeet_dir(ad).display().to_string(),
        models_dir: core_adapter::models_dir(ad).display().to_string(),
        llm: state
            .core
            .get()
            .map(|c| c.llm_status())
            .unwrap_or(LlmStatus::Off),
        default_llm_path: core_adapter::default_llm_path(ad).display().to_string(),
        llm_plugin: core_adapter::llm_plugin_file(),
    }
}

/// ✕ / ✓ clicked on the hands-free pill.
#[tauri::command]
fn overlay_action(state: tauri::State<'_, AppState>, action: String) -> Result<(), String> {
    let input = match action.as_str() {
        "cancel" => Input::Hotkey(hotkeys::HotkeyAction::Cancel),
        "finish" => Input::Finish,
        other => return Err(format!("unknown overlay action {other}")),
    };
    if let Some(tx) = state.input.get() {
        let _ = tx.send(input);
    }
    Ok(())
}

#[tauri::command]
fn list_microphones() -> Vec<String> {
    capture::list_input_devices()
}


// ------------------------------------------------- developer tab (D17 §5)

/// Load the on-device models? Only when the build has them, the client
/// route is allowed, and it may be needed soon: always when it is primary,
/// and as a warm fallback behind the server unless `keep_local_warm` is off.
fn want_local(prefs: &Prefs) -> bool {
    let caps = BuildCaps::current();
    let d = &prefs.developer;
    let (srv, cli) = d.effective(caps);
    cli && (d.keep_local_warm || !srv || d.prefer() == crate::dev_settings::Route::Client)
}

/// Load local models now if the settings want them and nothing is loaded
/// (e.g. Allow client was just turned on). Never reloads a loaded model.
fn ensure_local(state: &AppState, prefs: &Prefs) {
    let Some(core) = state.core.get() else { return };
    if !want_local(prefs) {
        return;
    }
    let ad = state.appdata.as_deref();
    // A slot that never started a load reports `Loading` with no path.
    if matches!(core.status(), ModelStatus::Loading { ref path } if path.is_empty()) {
        core.load(ModelSpec::from_prefs(prefs, ad));
        core.load_llm(LlmSpec::from_prefs(prefs, ad));
    }
}

#[derive(serde::Serialize)]
struct DevCheck {
    errors: Vec<FieldError>,
    warnings: Vec<String>,
    tls_forced: bool,
}

fn dev_check(form: &DevForm) -> DevCheck {
    let caps = BuildCaps::current();
    match form.validate(caps) {
        Ok((s, token)) => {
            let mut s2 = s.clone();
            s2.token_dpapi = if token.is_empty() { String::new() } else { "set".into() };
            // The router's own validation must agree (it is the gate).
            let errors = match router_adapter::server_config(&s, &token) {
                Ok(_) => Vec::new(),
                Err(m) => vec![FieldError {
                    field: "host",
                    message: m,
                }],
            };
            DevCheck {
                errors,
                warnings: s2.warnings(caps),
                tls_forced: !s.server.host.is_empty()
                    && crate::dev_settings::host_is_public(&s.server.host),
            }
        }
        Err(errors) => DevCheck {
            errors,
            warnings: Vec::new(),
            tls_forced: !form.host.trim().is_empty()
                && crate::dev_settings::normalize_host(&form.host)
                    .is_ok_and(|h| crate::dev_settings::host_is_public(&h)),
        },
    }
}

#[derive(serde::Serialize)]
struct DevView {
    form: DevForm,
    caps: BuildCaps,
    capabilities: flow_router::Capabilities,
    visible: bool,
    version: &'static str,
    check: DevCheck,
    route: Option<RouteView>,
}

fn dev_view(state: &AppState) -> DevView {
    let caps = BuildCaps::current();
    let dev = state.prefs.read().unwrap().developer.clone();
    let form = DevForm::from_settings(&dev, &dev.token());
    let router = state.router.get();
    DevView {
        check: dev_check(&form),
        form,
        caps,
        capabilities: router
            .map(|r| r.router().capabilities())
            .unwrap_or_else(|| flow_router::Capabilities {
                local: caps.local,
                remote: caps.remote,
                asr_engines: Vec::new(),
                llm: false,
                proto: 0,
                core_version: String::new(),
            }),
        visible: dev.tab_visible(caps),
        version: env!("CARGO_PKG_VERSION"),
        route: router.map(|r| r.route_view()),
    }
}

/// Save developer settings: validate → DPAPI the token → prefs.json →
/// `router.update_config` (next session) → probe in the background.
fn save_dev(app: &AppHandle, state: &AppState, dev: DevSettings) -> Result<(), String> {
    let mut prefs = state.prefs.read().unwrap().clone();
    prefs.developer = dev;
    if let Some(r) = state.router.get() {
        r.apply(&prefs.developer)?;
    }
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    ensure_local(state, &prefs);
    *state.prefs.write().unwrap() = prefs;
    probe_in_background(app);
    Ok(())
}

/// Refresh "Server info" off the UI thread when a server is configured.
fn probe_in_background(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(r) = state.router.get().cloned() else { return };
    let dev = state.prefs.read().unwrap().developer.clone();
    let (srv, _) = dev.effective(BuildCaps::current());
    if !srv || dev.server.host.is_empty() {
        let _ = app.emit_to(SETTINGS, "dev:route", r.route_view());
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("flow-probe".into())
        .spawn(move || {
            let _ = r.router().probe_server();
            let _ = app.emit_to(SETTINGS, "dev:route", r.route_view());
        });
}

#[tauri::command]
fn dev_get(state: tauri::State<'_, AppState>) -> DevView {
    dev_view(&state)
}

#[tauri::command]
fn dev_validate(form: DevForm) -> DevCheck {
    dev_check(&form)
}

#[tauri::command]
fn dev_set(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    form: DevForm,
) -> Result<DevView, Vec<FieldError>> {
    let (mut dev, token) = form.validate(BuildCaps::current())?;
    let as_error = |m: String| {
        vec![FieldError {
            field: "host",
            message: m,
        }]
    };
    router_adapter::server_config(&dev, &token).map_err(as_error)?;
    dev.token_dpapi = crate::secrets::protect(&token).map_err(|m| {
        vec![FieldError {
            field: "token",
            message: m,
        }]
    })?;
    save_dev(&app, &state, dev).map_err(as_error)?;
    Ok(dev_view(&state))
}

/// Probe the form's (unsaved) server; blocking, so async = off the UI thread.
#[tauri::command]
async fn dev_test_connection(
    app: AppHandle,
    form: DevForm,
) -> Result<flow_router::HealthReport, String> {
    let (dev, token) = form
        .validate(BuildCaps::current())
        .map_err(|e| e.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; "))?;
    let server = router_adapter::server_config(&dev, &token)?;
    let state = app.state::<AppState>();
    let router = state.router.get().cloned().ok_or("router not ready")?;
    tauri::async_runtime::spawn_blocking(move || router.router().probe_config(&server))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn dev_test_session(app: AppHandle) -> Result<SessionTest, String> {
    let state = app.state::<AppState>();
    if state
        .dictating
        .get()
        .is_some_and(|d| d.load(std::sync::atomic::Ordering::Relaxed))
    {
        return Err("a dictation is running".into());
    }
    let router = state.router.get().cloned().ok_or("router not ready")?;
    let cfg = state.prefs.read().unwrap().session_config();
    let r = tauri::async_runtime::spawn_blocking(move || router.test_session(cfg))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(router) = state.router.get() {
        let _ = app.emit_to(SETTINGS, "dev:route", router.route_view());
    }
    Ok(r)
}

#[tauri::command]
fn dev_route_status(state: tauri::State<'_, AppState>) -> Option<RouteView> {
    state.router.get().map(|r| r.route_view())
}

#[tauri::command]
fn dev_reset_circuit(state: tauri::State<'_, AppState>) -> Option<RouteView> {
    let r = state.router.get()?;
    r.router().reset_circuit();
    Some(r.route_view())
}

#[tauri::command]
fn dev_reset_defaults(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<DevView, String> {
    let unlocked = state.prefs.read().unwrap().developer.dev_unlocked;
    let dev = DevSettings {
        dev_unlocked: unlocked,
        ..DevSettings::default()
    };
    save_dev(&app, &state, dev)?;
    Ok(dev_view(&state))
}

/// 7 taps on the version row (client-only builds) / the Lock button.
#[tauri::command]
fn dev_unlock(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    unlocked: bool,
) -> Result<DevView, String> {
    let mut dev = state.prefs.read().unwrap().developer.clone();
    dev.dev_unlocked = unlocked;
    save_dev(&app, &state, dev)?;
    Ok(dev_view(&state))
}

// ---------------------------------------------------------------- windows

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Never take focus from the app being dictated into; never show in
/// Alt+Tab; click-through (02 §6 "Overlay").
fn make_overlay_passive(w: &WebviewWindow) {
    let _ = w.set_ignore_cursor_events(true);
    if let Some(h) = hwnd_of(w) {
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            let add = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | add);
        }
    }
}

/// Click-through except while the cursor is over the locked pill, so the
/// hands-free ✕/✓ are clickable but the transparent canvas never eats clicks.
fn spawn_hover_watch(app: AppHandle, state: Arc<Mutex<OverlayState>>) {
    let _ = std::thread::Builder::new()
        .name("flow-overlay-hover".into())
        .spawn(move || {
            let mut ignoring = true;
            loop {
                std::thread::sleep(Duration::from_millis(30));
                let st = state.lock().unwrap().clone();
                if st != OverlayState::ActiveLocked && ignoring {
                    continue;
                }
                let Some(w) = app.get_webview_window(OVERLAY) else {
                    continue;
                };
                let want = match hwnd_of(&w) {
                    Some(h) if st == OverlayState::ActiveLocked => {
                        let mut r = RECT::default();
                        let mut p = POINT::default();
                        let ok = unsafe {
                            GetWindowRect(h, &mut r).is_ok() && GetCursorPos(&mut p).is_ok()
                        };
                        !ok || crate::overlay::click_through(
                            &st,
                            crate::overlay::Rect {
                                left: r.left as f64,
                                top: r.top as f64,
                                right: r.right as f64,
                                bottom: r.bottom as f64,
                            },
                            w.scale_factor().unwrap_or(1.0),
                            (p.x as f64, p.y as f64),
                        )
                    }
                    _ => true,
                };
                if want != ignoring && w.set_ignore_cursor_events(want).is_ok() {
                    ignoring = want;
                }
            }
        });
}

/// 512x614 canvas, bottom-centre of the primary work area (above the taskbar).
fn position_overlay(w: &WebviewWindow) {
    let mut wa = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut wa as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if !ok {
        return;
    }
    if let Ok(size) = w.outer_size() {
        let x = wa.left + ((wa.right - wa.left) - size.width as i32) / 2;
        // Wispr: canvas flush with the work-area bottom minus a 2 px gap.
        let gap = (2.0 * w.scale_factor().unwrap_or(1.0)).round() as i32;
        let y = wa.bottom - size.height as i32 - gap;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

// ---------------------------------------------------------------- autostart

mod autostart {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };

    /// HKCU\…\Run\FlowClone = "<exe>" (per-user, no elevation).
    pub fn set(enabled: bool) -> windows::core::Result<()> {
        unsafe {
            let mut key = HKEY::default();
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
            .ok()?;
            let r = if enabled {
                let exe = std::env::current_exe().unwrap_or_default();
                let val = format!("\"{}\"", exe.display());
                let wide: Vec<u16> = val.encode_utf16().chain(std::iter::once(0)).collect();
                let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
                RegSetValueExW(key, w!("FlowClone"), 0, REG_SZ, Some(bytes)).ok()
            } else {
                let _ = RegDeleteValueW(key, PCWSTR(w!("FlowClone").as_ptr()));
                Ok(())
            };
            let _ = RegCloseKey(key);
            r
        }
    }
}

// ---------------------------------------------------------------- entry

pub fn run() {
    let appdata = std::env::var("APPDATA").ok();
    let path = Prefs::default_path(appdata.as_deref());
    let (prefs, outcome) = Prefs::load(&path);
    if let LoadOutcome::Corrupt(bak) = &outcome {
        eprintln!("[prefs] corrupt prefs moved to {}", bak.display());
    }
    let prefs = Arc::new(RwLock::new(prefs));
    let history =
        match crate::history::History::open(&crate::history::default_path(appdata.as_deref())) {
            Ok(h) => Some(Arc::new(Mutex::new(h))),
            Err(e) => {
                eprintln!("[history] disabled: {e}");
                None
            }
        };
    let suggestions = Arc::new(Mutex::new(SuggestionStore::open(&sugg::default_path(
        appdata.as_deref(),
    ))));

    tauri::Builder::default()
        .manage(AppState {
            prefs: prefs.clone(),
            path,
            appdata: appdata.clone(),
            hook: Mutex::new(None),
            core: OnceLock::new(),
            router: OnceLock::new(),
            dictating: OnceLock::new(),
            input: OnceLock::new(),
            history: history.clone(),
            suggestions: suggestions.clone(),
            suggest_item: OnceLock::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_prefs,
            set_prefs,
            list_microphones,
            get_model_status,
            overlay_action,
            history_list,
            history_delete,
            history_clear,
            history_copy,
            history_repaste,
            clipboard_copy,
            suggestions_list,
            suggestion_accept,
            suggestion_dismiss,
            dictionary_import,
            dictionary_export,
            dictionary_export_text,
            dev_get,
            dev_validate,
            dev_set,
            dev_test_connection,
            dev_test_session,
            dev_route_status,
            dev_reset_circuit,
            dev_reset_defaults,
            dev_unlock
        ])
        .on_window_event(|window, event| {
            if window.label() == SETTINGS {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();

            // Tray.
            let model_item =
                MenuItem::with_id(app, "model", "Speech model: loading…", true, None::<&str>)?;
            let llm_item = MenuItem::with_id(app, "llm", "AI cleanup: off", true, None::<&str>)?;
            let suggest_item = MenuItem::with_id(
                app,
                "suggestions",
                "No dictionary suggestions",
                false,
                None::<&str>,
            )?;
            let _ = app
                .state::<AppState>()
                .suggest_item
                .set(suggest_item.clone());
            let sep0 = PredefinedMenuItem::separator(app)?;
            let open = MenuItem::with_id(app, "open", "Settings…", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Flow Clone", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &model_item,
                    &llm_item,
                    &suggest_item,
                    &sep0,
                    &open,
                    &sep,
                    &quit,
                ],
            )?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip(TRAY_TIP)
                .menu(&menu)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "open" | "model" | "llm" => show_settings(app),
                    "suggestions" => {
                        show_settings(app);
                        let _ = app.emit_to(SETTINGS, "settings:tab", "dictionary");
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        show_settings(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // Overlay.
            if let Some(w) = app.get_webview_window(OVERLAY) {
                make_overlay_passive(&w);
                position_overlay(&w);
            }

            // Controller + hook.
            let (tx, rx) = mpsc::channel::<Input>();
            let _ = app.state::<AppState>().input.set(tx.clone());
            let overlay_state = Arc::new(Mutex::new(OverlayState::Resting));
            spawn_hover_watch(handle.clone(), overlay_state.clone());
            let platform = Arc::new(WinPlatform {
                app: handle.clone(),
                prefs: prefs.clone(),
                capture: Mutex::new(None),
                ducker: Ducker::spawn(),
                overlay_state,
            });
            // Local engine (client builds): load in the background; the
            // tray notice tracks it. Server-only builds have no model slot.
            let caps = BuildCaps::current();
            let local = caps.local.then(FlowCoreAdapter::new);
            if let Some(core) = &local {
                let h = handle.clone();
                let item = model_item.clone();
                core.on_status(move |st| apply_model_status(&h, &item, st));
                let h = handle.clone();
                let item = llm_item.clone();
                core.on_llm_status(move |st| apply_llm_status(&h, &item, st));
                let p = prefs.read().unwrap().clone();
                if want_local(&p) {
                    core.load(ModelSpec::from_prefs(&p, appdata.as_deref()));
                    core.load_llm(LlmSpec::from_prefs(&p, appdata.as_deref()));
                } else {
                    let _ = model_item.set_text("Speech model: not loaded (server first)");
                }
                let _ = app.state::<AppState>().core.set(core.clone());
            } else {
                let _ = model_item.set_text("Speech: your flow-server (no local model)");
                let _ = llm_item.set_text("AI cleanup: on the server");
            }
            // Every dictation goes through the router (D17).
            let dev = prefs.read().unwrap().developer.clone();
            let cfg = router_adapter::router_config(&dev, caps).unwrap_or_else(|e| {
                eprintln!("[developer] invalid settings, using defaults: {e}");
                router_adapter::router_config(&DevSettings::default(), caps)
                    .expect("default router config")
            });
            let router = RouterAdapter::new(cfg, local, router_adapter::remote_factory());
            {
                let h = handle.clone();
                let p = prefs.clone();
                let r = Arc::downgrade(&router);
                router.on_route(move |last| {
                    if p.read().unwrap().developer.show_engine_badge {
                        let _ = h.emit_to(OVERLAY, "dictation:route", last.badge());
                    }
                    if let Some(r) = r.upgrade() {
                        let _ = h.emit_to(SETTINGS, "dev:route", r.route_view());
                    }
                });
            }
            let _ = app.state::<AppState>().router.set(router.clone());
            probe_in_background(&handle);
            let start = Instant::now();
            let mut controller = Controller::new(
                platform.clone(),
                router,
                prefs.clone(),
                tx.clone(),
                Box::new(move || start.elapsed().as_millis() as u64),
            );
            if let Some(h) = history.clone() {
                controller = controller.with_history(h);
            }
            controller = controller.with_suggestions(suggestions.clone());
            let pending = suggestions.lock().unwrap().data.pending.len();
            show_suggestion_count(&handle, pending);
            let dictating = controller.dictating_flag();
            let _ = app.state::<AppState>().dictating.set(dictating.clone());
            std::thread::Builder::new()
                .name("flow-controller".into())
                .spawn(move || controller.run(rx))?;

            let (hk_tx, hk_rx) = mpsc::channel();
            let bindings = prefs.read().unwrap().shortcuts.clone();
            match hotkeys::win::install(bindings, dictating, hk_tx) {
                Ok(h) => {
                    *app.state::<AppState>().hook.lock().unwrap() = Some(h);
                }
                Err(e) => eprintln!("[hotkeys] {e}"),
            }
            std::thread::Builder::new()
                .name("flow-hotkey-fwd".into())
                .spawn(move || {
                    for a in hk_rx {
                        if tx.send(Input::Hotkey(a)).is_err() {
                            break;
                        }
                    }
                })?;

            platform.overlay(&OverlayState::Resting);
            if matches!(outcome, LoadOutcome::Missing) {
                show_settings(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Tauri app")
        .run(|_app, event| {
            // Closing the settings window must not quit the tray app.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
