//! Tauri 2 shell (Windows only): tray, overlay pill, settings window, and
//! the Win32 [`Platform`] implementation the controller drives.

use crate::audio::capture::{self, Capture};
use crate::context::{uia, RawSnapshot};
use crate::controller::{AudioSink, Controller, Platform};
use crate::core_adapter::{
    self, FlowCoreAdapter, LanguageSpec, LlmSpec, LlmStatus, ModelSpec, ModelStatus,
};
use crate::credentials::{self, CredentialStore, KeyState};
use crate::debug_log::{self, DebugLog};
use crate::dev_settings::{BuildCaps, DevForm, DevSettings, FieldError};
use crate::dictation::{Input, OverlayState, Sound};
use crate::ducking::win::Ducker;
use crate::history::{self, History};
use crate::hotkeys::{self, win::HookHandle};
use crate::insights::{self, InsightsStore};
use crate::notes::{self, AutoSave, NoteStore};
use crate::paste::{self, PasteOptions};
use crate::prefs::{LoadOutcome, Prefs};
use crate::recent_apps::{self, RecentAppStore};
use crate::router_adapter::{self, RouteView, RouterAdapter, SessionTest};
use crate::suggestions::{self as sugg, SuggestionStore};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, ShowWindow,
    SystemParametersInfoW, GWL_EXSTYLE, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

const OVERLAY: &str = "overlay";
const SETTINGS: &str = "settings";
/// Created on demand (tray / shortcut), destroyed on close.
const SCRATCHPAD: &str = "scratchpad";

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
    /// Box (CSS px) of the bubble's "Fix →" link while the overlay shows
    /// one; the hover watcher makes it clickable (`overlay::click_through`).
    overlay_link: Arc<Mutex<Option<crate::overlay::Rect>>>,
    history: Option<Arc<Mutex<History>>>,
    /// Insights page counts (`insights.json`, local only).
    insights: Arc<Mutex<InsightsStore>>,
    suggestions: Arc<Mutex<SuggestionStore>>,
    /// Exe names dictated into, for the "Style per app" picker.
    recent_apps: Arc<Mutex<RecentAppStore>>,
    /// Tray line "Dictionary suggestions (n)…".
    suggest_item: OnceLock<MenuItem<tauri::Wry>>,
    /// The OpenRouter API key (D18 C3 / D28): Credential Manager.
    creds: Arc<dyn CredentialStore>,
    /// `cloud_usage.json` (this month's counters; never sent anywhere).
    usage_path: PathBuf,
    /// Opt-in debug log (Developer → Debug log; off by default).
    debug: Arc<DebugLog>,
    debug_path: PathBuf,
    /// Notes (Scratchpad = Quick note): debounced writes to `notes\<id>.txt`.
    notes: Arc<Mutex<AutoSave>>,
    /// Clock of `notes` (ms since start).
    started: Instant,
    /// The speech-model download the user started (at most one, D31).
    downloads: Arc<crate::model_download::Downloads>,
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
        self.start_capture_reporting(device, sink, Box::new(|_| {}))
    }

    fn start_capture_reporting(
        &self,
        device: Option<String>,
        sink: AudioSink,
        on_error: crate::controller::CaptureError,
    ) -> Result<(), String> {
        let c = Capture::start(device, sink, Arc::from(on_error))?;
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

    fn live_route(&self, label: Option<&crate::core_api::LiveRouteLabel>) {
        let _ = self.app.emit_to(OVERLAY, "dictation:live-route", label);
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

    fn keyboard_language(&self) -> Option<String> {
        crate::keyboard_language::win::foreground_language().map(String::from)
    }

    fn focus_target(&self) -> Option<paste::FocusTarget> {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let (hwnd, exe, _) = uia::foreground_app();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        Some(paste::FocusTarget {
            hwnd: hwnd.0 as usize,
            pid,
            exe: crate::context::exe_name(&exe),
        })
    }

    fn focused_is_password(&self) -> Option<bool> {
        uia::focused_is_password(Duration::from_millis(300))
    }

    fn wait_shortcut_released(&self) {
        paste::win::wait_modifiers_released(Duration::from_millis(1000));
    }

    fn capture_selection(&self) -> Result<Option<String>, String> {
        // UIA first (no clipboard traffic); Ctrl+C only when the control has
        // no TextPattern or UIA hangs. Bounded well under SELECTION_WAIT.
        crate::context::resolve_selection(
            uia::selection(Duration::from_millis(800)),
            paste::win::copy_selection,
        )
    }

    fn field_reader(&self) -> Option<Box<dyn crate::learn::FieldReader>> {
        uia::field_reader().map(|r| Box::new(r) as Box<dyn crate::learn::FieldReader>)
    }

    fn history_changed(&self) {
        let _ = self.app.emit_to(SETTINGS, "history:changed", ());
    }

    fn retry_finished(&self, row: i64, result: &Result<(), String>) {
        let _ = self.app.emit_to(
            SETTINGS,
            "history:retry",
            serde_json::json!({ "id": row, "error": result.as_ref().err() }),
        );
    }

    fn insights_changed(&self) {
        let _ = self.app.emit_to(SETTINGS, "insights:changed", ());
    }

    fn suggestions_changed(&self, pending: usize) {
        show_suggestion_count(&self.app, pending);
    }

    fn open_scratchpad(&self) {
        show_scratchpad(&self.app);
    }

    fn view_changes(&self, row: Option<i64>) {
        show_settings(&self.app);
        let _ = self.app.emit_to(SETTINGS, "transforms:view", row);
    }

    fn open_new_note(&self) -> bool {
        let st = self.app.state::<AppState>();
        let created = st.notes.lock().unwrap().create();
        let id = match created {
            Ok(id) => id,
            Err(e) => {
                eprintln!("[notes] new note: {e}");
                debug_log::event("notes", &format!("new note: {e}"));
                return false;
            }
        };
        show_notes_window(&self.app, format!("note={id}"));
        // The dictation pastes only into the window it started in (S13), so
        // start it once the note window is in front.
        let deadline = Instant::now() + Duration::from_millis(2000);
        while Instant::now() < deadline {
            let front = self
                .app
                .get_webview_window(SCRATCHPAD)
                .is_some_and(|w| w.is_focused().unwrap_or(false));
            if front {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        eprintln!("[notes] the new note's window did not come to the front");
        false
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

const TRAY_TIP: &str = "Murmur — hold Ctrl+Win to dictate";

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
            "Murmur — speech model missing (see Settings)".to_string(),
        ),
        ModelStatus::Failed { .. } => (
            "⚠ Speech model failed to load — open Settings".to_string(),
            "Murmur — speech model failed to load (see Settings)".to_string(),
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
            debug_log::event("engine_load", &problem);
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
        LlmStatus::Ready { path, backend, .. } => {
            let on = match backend {
                Some(b) if b.is_gpu() => " (GPU)",
                _ => "",
            };
            format!("AI cleanup: {}{on}", file(path))
        }
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
    // The cloud backend shares the LLM cleaner: rebuild it on a change.
    if matches!(st, LlmStatus::Ready { .. } | LlmStatus::Off) {
        if let Some(r) = app.state::<AppState>().router.get() {
            r.refresh_local();
        }
    }
    if let Some(problem) = st.problem() {
        eprintln!("[llm] {problem}");
        debug_log::event("llm_load", &problem);
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
    prefs: Prefs,
) -> Result<(), String> {
    save_prefs(&app, &state, prefs)
}

/// Validate, save and apply new prefs (reloads the model when its choice
/// changed). `set_prefs` and a finished model download use it.
fn save_prefs(app: &AppHandle, state: &AppState, mut prefs: Prefs) -> Result<(), String> {
    // The Developer tab saves its own section (dev_set); a General/AI save
    // from a page loaded earlier must not roll it back.
    prefs.developer = state.prefs.read().unwrap().developer.clone();
    // Whatever the page sent, what is saved now is the current schema.
    prefs.prefs_rev = crate::prefs::PREFS_REV;
    prefs.dictionary = crate::dictionary::normalize(prefs.dictionary);
    prefs.transforms.normalize();
    prefs.validate()?;
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    if let Some(h) = state.hook.lock().unwrap().as_ref() {
        h.set_bindings(prefs.hotkey_bindings());
    }
    let _ = autostart::set(prefs.launch_at_login);
    let (reload, reload_llm) = {
        let old = state.prefs.read().unwrap();
        (
            core_adapter::needs_reload(&old, &prefs) || !model_ready(state),
            core_adapter::needs_llm_reload(&old, &prefs) || llm_wanted_but_down(state, &prefs),
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
            // D33: the Hebrew model loads (or retries, e.g. after its
            // download) next to the default one, never reloading it.
            if core_adapter::needs_language_reload(core, &prefs, ad) {
                core.load_language(LanguageSpec::from_prefs(&prefs, ad));
            }
        } else if core.is_ready() {
            // D24: a model loaded on demand follows the new model choice.
            if core_adapter::needs_reload(&state.prefs.read().unwrap(), &prefs) {
                core.load(ModelSpec::from_prefs(&prefs, ad));
            }
            if core_adapter::needs_language_reload(core, &prefs, ad) {
                core.load_language(LanguageSpec::from_prefs(&prefs, ad));
            }
        }
    }
    sync_on_demand(state, &prefs);
    if let Some(h) = &state.history {
        let g = h.lock().unwrap();
        let _ = g.prune(prefs.retention, history::now_ms());
        let _ = g.prune_audio(prefs.retry_audio_keep);
    }
    let pending = {
        let mut s = state.suggestions.lock().unwrap();
        s.data.prune_known(&prefs.dictionary);
        let _ = s.save();
        s.data.pending.len()
    };
    let _ = app.emit_to(OVERLAY, "overlay:theme", prefs.theme);
    *state.prefs.write().unwrap() = prefs;
    show_suggestion_count(app, pending);
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
    let h = history_db(&state)?.lock().unwrap();
    let rows = h
        .list(&query, limit.unwrap_or(200), retention, history::now_ms())
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|e| h.item(e).map_err(|e| e.to_string()))
        .collect()
}

/// Re-transcribe row `id`'s kept recording with today's settings (never
/// pastes). Returns at once; `history:retry` {id, error} reports the end.
#[tauri::command]
fn history_retry(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let has = history_db(&state)?
        .lock()
        .unwrap()
        .has_audio(id)
        .map_err(|e| e.to_string())?;
    if !has {
        return Err(crate::controller::RETRY_GONE.into());
    }
    let tx = state.input.get().ok_or("dictation is not running yet")?;
    tx.send(Input::Retry { row: id })
        .map_err(|_| "dictation is not running".to_string())
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

// ------------------------------------------------- insights tab

#[tauri::command]
fn insights_get(state: tauri::State<'_, AppState>) -> insights::View {
    let today = insights::local_date(history::now_ms());
    state.insights.lock().unwrap().view(today)
}

/// Clear every count (History is untouched and is not re-added).
#[tauri::command]
fn insights_reset(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut s = state.insights.lock().unwrap();
    s.reset();
    s.save().map_err(|e| e.to_string())
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
            debug_log::event("paste", &format!("history re-paste: {e}"));
        }
    });
    Ok(())
}

#[tauri::command]
fn clipboard_copy(text: String) -> Result<(), String> {
    paste::win::copy_text(&text)
}

// ------------------------------------------------- scratchpad (notes)

fn notes_now(state: &AppState) -> u64 {
    state.started.elapsed().as_millis() as u64
}

/// The note's text (an edit not yet written included); `""` if never saved.
#[tauri::command]
fn note_load(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    state.notes.lock().unwrap().load(&id)
}

/// Every change of the editor; written after a pause (`notes::SAVE_DEBOUNCE_MS`).
#[tauri::command]
fn note_edit(state: tauri::State<'_, AppState>, id: String, text: String) -> Result<(), String> {
    let now = notes_now(&state);
    state.notes.lock().unwrap().edit(&id, &text, now)
}

/// Write the note now (the page is going away).
#[tauri::command]
fn note_flush(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state.notes.lock().unwrap().flush(&id).unwrap_or(Ok(()))
}

/// The Notes list (content search; pinned first, then newest).
#[tauri::command]
fn note_list(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<notes::NoteMeta>, String> {
    state.notes.lock().unwrap().list(&query)
}

/// A new empty note; returns its id (the page then opens it).
#[tauri::command]
fn note_create(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state.notes.lock().unwrap().create()
}

/// The page asked first (second click on Delete).
#[tauri::command]
fn note_delete(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state.notes.lock().unwrap().delete(&id)
}

#[tauri::command]
fn note_set_pinned(
    state: tauri::State<'_, AppState>,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    state.notes.lock().unwrap().set_pinned(&id, pinned)
}

/// Write the note to Downloads as `<title>.txt` / `.md`; returns the path.
#[tauri::command]
fn note_export(
    state: tauri::State<'_, AppState>,
    id: String,
    ext: String,
) -> Result<String, String> {
    let dir = downloads_dir()?;
    let path = state.notes.lock().unwrap().export(&id, &ext, &dir)?;
    Ok(path.display().to_string())
}

/// The app window's Scratchpad page: show `id` in the notes window (an
/// unknown id opens an empty editor, as `note=<id>` always has).
#[tauri::command]
fn note_open(app: AppHandle, id: String) -> Result<(), String> {
    if !notes::valid_id(&id) {
        return Err(format!("\"{id}\" is not a note id"));
    }
    show_notes_window(&app, format!("note={id}"));
    Ok(())
}

fn log_note_writes(results: Vec<(String, Result<(), String>)>) {
    for (id, r) in results {
        if let Err(e) = r {
            eprintln!("[notes] {id}: {e}");
            debug_log::event("notes", &format!("{id}: {e}"));
        }
    }
}

/// Writes debounced note edits (a quarter-second tick is plenty for 750 ms).
fn spawn_note_saver(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("flow-notes".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            let st = app.state::<AppState>();
            let now = notes_now(&st);
            let done = st.notes.lock().unwrap().tick(now);
            log_note_writes(done);
        });
    if let Err(e) = spawned {
        eprintln!("[notes] {e}");
    }
}

// ------------------------------------------------- style per app

/// Recent apps + the built-in exe → group mapping (Writing style table).
#[tauri::command]
fn app_style_choices(state: tauri::State<'_, AppState>) -> recent_apps::StyleChoices {
    recent_apps::style_choices(&state.recent_apps.lock().unwrap().data)
}

#[tauri::command]
fn recent_apps_clear(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut r = state.recent_apps.lock().unwrap();
    r.data.clear();
    r.save().map_err(|e| e.to_string())
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

/// `%USERPROFILE%\Downloads` (else Documents, else the profile folder).
fn downloads_dir() -> Result<PathBuf, String> {
    let base = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    [base.join("Downloads"), base.join("Documents"), base]
        .into_iter()
        .find(|d| d.is_dir())
        .ok_or_else(|| "no Downloads folder".to_string())
}

/// Write the exchange JSON to the Downloads folder; returns the path.
#[tauri::command]
fn dictionary_export(entries: Vec<crate::prefs::DictEntry>) -> Result<String, String> {
    let dir = downloads_dir()?;
    let path = dir.join("flow-dictionary.json");
    std::fs::write(&path, crate::dictionary::export(&entries)).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn dictionary_export_text(entries: Vec<crate::prefs::DictEntry>) -> String {
    crate::dictionary::export(&entries)
}

/// Native folder / file picker owned by the settings window ("Browse" /
/// "Change" next to model paths). `None` = cancelled. Runs on its own
/// thread so the UI keeps painting while it is open.
#[tauri::command]
async fn pick_path(
    app: AppHandle,
    kind: crate::shell::PickKind,
    title: Option<String>,
    filters: Option<Vec<crate::shell::PickFilter>>,
) -> Result<Option<String>, String> {
    let req = crate::shell::PickRequest::new(kind, title, filters)?;
    let owner = app
        .get_webview_window(SETTINGS)
        .and_then(|w| hwnd_of(&w))
        .map_or(0, |h| h.0 as isize);
    tauri::async_runtime::spawn_blocking(move || crate::shell::pick(owner, req))
        .await
        .map_err(|e| e.to_string())?
}

/// Open a help / download link in the browser: only the allowlisted hosts
/// in `shell::check_external_url` (the app never fetches it itself).
#[tauri::command]
async fn open_external(url: String) -> Result<(), String> {
    crate::shell::check_external_url(&url)?;
    tauri::async_runtime::spawn_blocking(move || crate::shell::open_external(&url))
        .await
        .map_err(|e| e.to_string())?
}

/// The speech and AI cleanup models this build can download (D31), with
/// where each is saved and whether it is already there.
#[tauri::command]
fn model_catalog(state: tauri::State<'_, AppState>) -> Vec<crate::model_download::CatalogView> {
    use crate::model_download::{catalog_view, ModelEngine, CAN_DOWNLOAD};
    let dir = core_adapter::models_dir(state.appdata.as_deref());
    catalog_view(&dir, CAN_DOWNLOAD, |e| match e {
        ModelEngine::Whisper => cfg!(feature = "whisper"),
        ModelEngine::Parakeet | ModelEngine::SherpaOnline => cfg!(feature = "parakeet"),
        // llama.cpp is a plugin (flow_llm.dll) loaded by the local engine.
        ModelEngine::Llm => cfg!(feature = "local-engine"),
    })
}

/// Where local AI cleanup would run (GPU or CPU), for Settings → AI
/// cleanup's model recommendation. Loads the plugin on first use (cached),
/// so the page asks only when it shows the cleanup models.
#[tauri::command]
async fn llm_backend(state: tauri::State<'_, AppState>) -> Result<core_adapter::LlmBackendView, String> {
    if !cfg!(feature = "local-engine") {
        return Ok(core_adapter::LlmBackendView::from_probe(Err(
            flow_core::llama::LlamaLoadError::Backend("this build has no local engine".into()),
        )));
    }
    let lib = state
        .prefs
        .read()
        .unwrap()
        .llm_lib_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    tauri::async_runtime::spawn_blocking(move || core_adapter::llm_backend_probe(lib.as_deref()))
        .await
        .map_err(|e| e.to_string())
}

/// Download catalogue model `id` (D9/D31: only ever sent after the user
/// pressed Download). Progress arrives as `model:download` events; on
/// `done` the prefs point the local engine at it and it loads.
#[tauri::command]
fn model_download_start(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    #[cfg(feature = "download")]
    {
        use crate::model_download::{self as md, Progress, State, Stop};
        let entry = md::entry(&id).ok_or_else(|| format!("unknown model {id}"))?;
        let fetch = md::HttpFetch::new()?;
        let cancel = state.downloads.begin(&id)?;
        let dir = core_adapter::models_dir(state.appdata.as_deref());
        let downloads = state.downloads.clone();
        std::thread::spawn(move || {
            let total = entry.size();
            let send = |p: Progress| {
                let _ = app.emit_to(SETTINGS, "model:download", p);
            };
            let r = md::run(entry, &dir, &fetch, &cancel, &mut |p| send(p));
            let end = |state: State, error: Option<String>, path: Option<String>| Progress {
                id: entry.id.to_string(),
                received: if state == State::Done { total } else { 0 },
                total,
                state,
                error,
                path,
            };
            let last = match r {
                Ok(path) => {
                    let st = app.state::<AppState>();
                    let mut prefs = st.prefs.read().unwrap().clone();
                    entry.apply_to(&mut prefs, &dir);
                    match save_prefs(&app, &st, prefs) {
                        Ok(()) => end(State::Done, None, Some(path.display().to_string())),
                        Err(e) => end(
                            State::Error,
                            Some(format!("downloaded, but the setting was not saved: {e}")),
                            None,
                        ),
                    }
                }
                Err(Stop::Cancelled) => end(State::Cancelled, None, None),
                Err(Stop::Failed(e)) => end(State::Error, Some(e), None),
            };
            eprintln!("model download {}: {:?}", entry.id, last.state);
            downloads.finish(entry.id);
            send(last);
        });
        Ok(())
    }
    #[cfg(not(feature = "download"))]
    {
        let _ = (app, state, id);
        Err("this build has no downloader: get the model from its page".into())
    }
}

/// Stop the running download of `id`; its `.part` file is deleted and a
/// `cancelled` event follows. False when it was not running.
#[tauri::command]
fn model_download_cancel(state: tauri::State<'_, AppState>, id: String) -> bool {
    state.downloads.cancel(&id)
}

// ------------------------------------------------- shortcut recording

/// How long "press the keys" waits for a chord before giving up.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
/// The latest recording; a timed-out older one must not stop it.
static CAPTURE_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Payload of `hotkey:capture`: "done" (with `chord`), "cancelled" (Esc)
/// or "timeout".
#[derive(Clone, serde::Serialize)]
struct CaptureResult {
    state: &'static str,
    chord: Option<String>,
}

/// Settings' Change button: record the next chord the user holds and
/// releases, through the keyboard hook (the webview can't tell Right Alt
/// from AltGr and never sees Win), and emit it as `hotkey:capture`.
/// Hotkeys don't fire meanwhile; the page saves the chord as if typed.
#[tauri::command]
fn hotkey_capture_start(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    use crate::hotkeys::capture::RecordStep;
    use std::sync::atomic::Ordering;
    let (tx, rx) = mpsc::channel();
    match state.hook.lock().unwrap().as_ref() {
        Some(h) => h.start_capture(tx),
        None => return Err("The keyboard hook is not running: type the shortcut instead.".into()),
    }
    let gen = CAPTURE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::Builder::new()
        .name("flow-hotkey-capture".into())
        .spawn(move || {
            let (state, chord) = match rx.recv_timeout(CAPTURE_TIMEOUT) {
                Ok(RecordStep::Done(c)) => ("done", Some(c.to_string())),
                Ok(_) => ("cancelled", None),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if CAPTURE_GEN.load(Ordering::SeqCst) == gen {
                        if let Some(h) = app.state::<AppState>().hook.lock().unwrap().as_ref() {
                            h.stop_capture();
                        }
                    }
                    ("timeout", None)
                }
                // Stopped by the page, or replaced by a newer recording.
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            let _ = app.emit("hotkey:capture", CaptureResult { state, chord });
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The page left recording (its window lost focus, or Cancel): hotkeys
/// work again.
#[tauri::command]
fn hotkey_capture_stop(state: tauri::State<'_, AppState>) {
    if let Some(h) = state.hook.lock().unwrap().as_ref() {
        h.stop_capture();
    }
}

/// Delete a downloaded AI cleanup model (Settings → AI cleanup). When the
/// prefs use it, local cleanup is turned off first so the engine lets go of
/// the file; the formatting rules keep working. Speech models are not
/// deleted from the app.
#[tauri::command]
fn model_delete(app: AppHandle, state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let entry = crate::model_download::entry(&id).ok_or_else(|| format!("unknown model {id}"))?;
    if !entry.is_llm() {
        return Err("only AI cleanup models can be deleted here".into());
    }
    if state.downloads.running().as_deref() == Some(entry.id) {
        return Err("this model is downloading; cancel it first".into());
    }
    let ad = state.appdata.as_deref();
    let dir = core_adapter::models_dir(ad);
    let mut prefs = state.prefs.read().unwrap().clone();
    if entry.forget_in(&mut prefs, &dir, ad) {
        save_prefs(&app, &state, prefs)?;
    }
    entry.delete(&dir)
}

// ------------------------------------------------- transforms page

/// "Reset to defaults": the built-in transforms come back, custom ones
/// stay. Returns the saved prefs for the page to adopt.
#[tauri::command]
fn transforms_reset(state: tauri::State<'_, AppState>) -> Result<Prefs, String> {
    let mut prefs = state.prefs.read().unwrap().clone();
    prefs.transforms.reset_builtins();
    prefs.validate()?;
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    if let Some(h) = state.hook.lock().unwrap().as_ref() {
        h.set_bindings(prefs.hotkey_bindings());
    }
    *state.prefs.write().unwrap() = prefs.clone();
    Ok(prefs)
}

/// "Try it out": run transform `id` on `text` typed in the page, with the
/// same route and privacy gate as a selection (the text is the user's,
/// like a selection). Nothing is pasted or recorded.
#[tauri::command]
async fn transform_try(app: AppHandle, id: String, text: String) -> Result<String, String> {
    let state = app.state::<AppState>();
    let instruction = state
        .prefs
        .read()
        .unwrap()
        .transforms
        .find(&id)
        .map(|i| i.transform.instruction.clone())
        .ok_or("that transform no longer exists")?;
    let router = state.router.get().cloned().ok_or("AI is not ready yet")?;
    tauri::async_runtime::spawn_blocking(move || {
        use crate::core_api::CoreApi;
        router.transform(
            &instruction,
            &text,
            flow_core::transform::Source::Selection,
            flow_core::transform::SELECTION_TIMEOUT,
        )
    })
    .await
    .map_err(|e| e.to_string())?
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
    /// The Hebrew model (D33 route); `null` = none chosen.
    hebrew: Option<ModelStatus>,
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
        hebrew: state.core.get().and_then(|c| c.language_status()),
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

/// Settings pages the overlay may open (its "Fix →" / "Choose a model →"
/// links); anything else is refused.
const OVERLAY_SETTINGS_PAGES: &[&str] = &[
    "settings-speech",
    "settings-cleanup",
    "settings-openrouter",
    "settings-server",
];

fn overlay_settings_page(page: Option<&str>) -> Result<&'static str, String> {
    let page = page.unwrap_or_default();
    OVERLAY_SETTINGS_PAGES
        .iter()
        .find(|p| **p == page)
        .copied()
        .ok_or_else(|| format!("unknown settings page {page:?}"))
}

/// ✕ / ✓ clicked on the hands-free pill, or a pill link to a settings
/// page: `open_settings` + `page` shows the settings window and emits
/// `settings:tab` with the page id. `link_rect` + `rect` (CSS px, `null` =
/// no link shown) tells the hover watcher where that link is, so it is
/// clickable while the rest of the canvas stays click-through.
#[tauri::command]
fn overlay_action(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    action: String,
    page: Option<String>,
    rect: Option<crate::overlay::Rect>,
) -> Result<(), String> {
    let input = match action.as_str() {
        "cancel" => Input::Hotkey(hotkeys::HotkeyAction::Cancel),
        "finish" => Input::Finish,
        "link_rect" => {
            let rect = rect.map(crate::overlay::accept_link_rect).transpose()?;
            *state.overlay_link.lock().unwrap() = rect;
            return Ok(());
        }
        "open_settings" => {
            let page = overlay_settings_page(page.as_deref())?;
            show_settings(&app);
            let _ = app.emit_to(SETTINGS, "settings:tab", page);
            return Ok(());
        }
        other => return Err(format!("unknown overlay action {other}")),
    };
    if let Some(tx) = state.input.get() {
        let _ = tx.send(input);
    }
    Ok(())
}

/// The Flow bar's colour scheme (`theme` pref). The overlay is not granted
/// `get_prefs` (review S18: it sees no dictionary, host or history), only
/// this; later changes arrive as `overlay:theme`.
#[tauri::command]
fn overlay_theme(state: tauri::State<'_, AppState>) -> crate::prefs::Theme {
    state.prefs.read().unwrap().theme
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

/// D24 never-fail fallback: the model the router loads on demand when the
/// client route is allowed but not kept warm (`keep_local_warm` off), so an
/// unreachable server never fails a dictation. `None` when the slot is
/// loaded at startup anyway, or the client route is off.
fn on_demand_spec(prefs: &Prefs, appdata: Option<&str>) -> Option<ModelSpec> {
    let (_, cli) = prefs.developer.effective(BuildCaps::current());
    (cli && !want_local(prefs)).then(|| ModelSpec::from_prefs(prefs, appdata))
}

fn sync_on_demand(state: &AppState, prefs: &Prefs) {
    if let Some(r) = state.router.get() {
        let ad = state.appdata.as_deref();
        r.set_on_demand_with(
            on_demand_spec(prefs, ad),
            LanguageSpec::from_prefs(prefs, ad),
        );
    }
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
        core.load_language(LanguageSpec::from_prefs(prefs, ad));
        core.load_llm(LlmSpec::from_prefs(prefs, ad));
    }
}

#[derive(serde::Serialize)]
struct DevCheck {
    errors: Vec<FieldError>,
    warnings: Vec<String>,
    tls_forced: bool,
}

fn dev_check(form: &DevForm, saved: &DevSettings, creds: &dyn CredentialStore) -> DevCheck {
    let caps = BuildCaps::current();
    match form.validate(caps) {
        Ok((s, token)) => {
            let mut s2 = s.clone();
            s2.keep_cloud(saved);
            s2.token_dpapi = if token.is_empty() {
                String::new()
            } else {
                "set".into()
            };
            // The router's own validation must agree (it is the gate).
            let errors = match router_adapter::server_config(&s, &token) {
                Ok(_) => Vec::new(),
                Err(m) => vec![FieldError {
                    field: "host",
                    message: m,
                }],
            };
            let mut warnings = s2.warnings(caps);
            warnings.extend(s2.cloud_warnings(
                caps,
                credentials::key_state(creds, credentials::OPENROUTER).set,
            ));
            DevCheck {
                errors,
                warnings,
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
    /// D26: product version (`0.2.0`), commit (`abc1234[-dirty]`), both (`0.2.0+abc1234`).
    version: &'static str,
    build: &'static str,
    full: &'static str,
    check: DevCheck,
    route: Option<RouteView>,
}

fn dev_view(state: &AppState) -> DevView {
    let caps = BuildCaps::current();
    let dev = state.prefs.read().unwrap().developer.clone();
    // S18: the decrypted token never goes to the webview.
    let form = DevForm::for_page(&dev);
    let router = state.router.get();
    DevView {
        check: dev_check(&form.with_token(&dev), &dev, &*state.creds),
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
        version: flow_build_info::VERSION,
        build: flow_build_info::BUILD,
        full: flow_build_info::FULL,
        route: router.map(|r| r.route_view()),
    }
}

/// Save developer settings: validate → DPAPI the token → prefs.json →
/// `router.update_config` (next session) → probe in the background.
/// The OpenRouter settings are not part of the form: they are kept as
/// stored (the OpenRouter page saves them with [`save_cloud`]).
fn save_dev(app: &AppHandle, state: &AppState, mut dev: DevSettings) -> Result<(), String> {
    let mut prefs = state.prefs.read().unwrap().clone();
    // Its own toggle (debug_log_set) owns it; a form save keeps it.
    dev.debug_log = prefs.developer.debug_log;
    dev.keep_cloud(&prefs.developer);
    let before = prefs.clone();
    prefs.developer = dev;
    if let Some(r) = state.router.get() {
        r.apply(&prefs.developer)?;
    }
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    // Parakeet decoding lives here and changes the recognizer set.
    if core_adapter::needs_reload(&before, &prefs) && want_local(&prefs) {
        if let Some(core) = state.core.get() {
            core.load(ModelSpec::from_prefs(&prefs, state.appdata.as_deref()));
        }
    }
    ensure_local(state, &prefs);
    sync_on_demand(state, &prefs);
    refresh_debug_secrets(state, &prefs.developer);
    *state.prefs.write().unwrap() = prefs;
    probe_in_background(app);
    Ok(())
}

/// The server token and the OpenRouter key, so the debug log can scrub
/// them out of any error text (memory only).
fn refresh_debug_secrets(state: &AppState, dev: &DevSettings) {
    let mut secrets = vec![dev.token()];
    if let Some(k) = credentials::openrouter_key(&*state.creds) {
        secrets.push(k);
    }
    secrets.retain(|s| !s.is_empty());
    state.debug.set_secrets(secrets);
}

// ------------------------------------------------------------ debug log

#[derive(serde::Serialize)]
struct DebugLogView {
    enabled: bool,
    /// Lines in `dictations.jsonl` (`None` = unreadable).
    count: Option<usize>,
    path: String,
    stats: debug_log::Stats,
}

fn debug_view(state: &AppState) -> DebugLogView {
    DebugLogView {
        enabled: state.debug.is_enabled(),
        count: state.debug.count(),
        path: state.debug_path.display().to_string(),
        stats: state.debug.stats(),
    }
}

#[tauri::command]
fn debug_log_status(state: tauri::State<'_, AppState>) -> DebugLogView {
    debug_view(&state)
}

/// The Developer toggle: saved at once (prefs.json `developer.debug_log`).
#[tauri::command]
fn debug_log_set(state: tauri::State<'_, AppState>, enabled: bool) -> Result<DebugLogView, String> {
    let mut prefs = state.prefs.read().unwrap().clone();
    prefs.developer.debug_log = enabled;
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    *state.prefs.write().unwrap() = prefs;
    state.debug.set_enabled(enabled);
    Ok(debug_view(&state))
}

#[tauri::command]
fn debug_log_clear(state: tauri::State<'_, AppState>) -> Result<DebugLogView, String> {
    state.debug.clear()?;
    Ok(debug_view(&state))
}

#[tauri::command]
fn debug_log_open_folder(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let dir = state
        .debug_path
        .parent()
        .ok_or("no log folder")?
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::process::Command::new("explorer.exe")
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Refresh "Server info" off the UI thread when a server is configured.
fn probe_in_background(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(r) = state.router.get().cloned() else {
        return;
    };
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
            let report = r.router().probe_server();
            if !report.ok {
                debug_log::event(
                    "server",
                    &format!(
                        "probe {}: {} {}",
                        report.server.as_deref().unwrap_or("?"),
                        report.error.as_deref().unwrap_or("failed"),
                        report.message.as_deref().unwrap_or("")
                    ),
                );
            }
            let _ = app.emit_to(SETTINGS, "dev:route", r.route_view());
        });
}

#[tauri::command]
fn dev_get(state: tauri::State<'_, AppState>) -> DevView {
    dev_view(&state)
}

#[tauri::command]
fn dev_validate(state: tauri::State<'_, AppState>, form: DevForm) -> DevCheck {
    let saved = state.prefs.read().unwrap().developer.clone();
    dev_check(&form.with_token(&saved), &saved, &*state.creds)
}

#[tauri::command]
fn dev_set(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    form: DevForm,
) -> Result<DevView, Vec<FieldError>> {
    let saved = state.prefs.read().unwrap().developer.clone();
    let (mut dev, token) = form.with_token(&saved).validate(BuildCaps::current())?;
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
    let saved = app
        .state::<AppState>()
        .prefs
        .read()
        .unwrap()
        .developer
        .clone();
    let (dev, token) = form
        .with_token(&saved)
        .validate(BuildCaps::current())
        .map_err(|e| {
            e.into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    let server = router_adapter::server_config(&dev, &token)?;
    let state = app.state::<AppState>();
    let router = state.router.get().cloned().ok_or("router not ready")?;
    tauri::async_runtime::spawn_blocking(move || router.router().probe_config(&server))
        .await
        .map_err(|e| e.to_string())
}

/// D37: Settings → Languages "Keyboard language · now: Hebrew": the
/// language of the foreground window's layout (Settings itself while it
/// is open), `None` when unknown.
#[tauri::command]
fn keyboard_language() -> Option<String> {
    crate::keyboard_language::win::foreground_language().map(String::from)
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

// ---------------------------------------------- OpenRouter (D18 / D28)
//
// One page (`crate::openrouter`): key, privacy, speech-to-text, text
// cleanup, usage. Every edit is validated in `crate::openrouter`, saved to
// prefs.json and handed to the router at once. The key is only ever in
// Credential Manager and the router (never returned to the page).

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Save the OpenRouter part of the settings and apply it. A use that was
/// just switched on refreshes the model catalogue (spec §2.2 "When").
fn save_cloud(app: &AppHandle, state: &AppState, dev: DevSettings) -> Result<(), String> {
    let caps = BuildCaps::current();
    let mut prefs = state.prefs.read().unwrap().clone();
    let was = (
        prefs.developer.cloud_effective(caps),
        prefs.developer.cleanup_effective(caps),
    );
    prefs.developer = dev;
    if let Some(r) = state.router.get() {
        r.apply(&prefs.developer)?;
    }
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    let now = (
        prefs.developer.cloud_effective(caps),
        prefs.developer.cleanup_effective(caps),
    );
    *state.prefs.write().unwrap() = prefs;
    if (now.0 && !was.0) || (now.1 && !was.1) {
        refresh_catalog_in_background(app, false);
    }
    Ok(())
}

fn edit_cloud(
    app: &AppHandle,
    state: &AppState,
    f: impl FnOnce(&mut DevSettings) -> Result<(), String>,
) -> Result<crate::openrouter::OrView, String> {
    let mut dev = state.prefs.read().unwrap().developer.clone();
    f(&mut dev)?;
    save_cloud(app, state, dev)?;
    Ok(or_view(state))
}

/// `{source, fetched_at_ms, snapshot_date, models, stt, cleanup}`.
fn catalog_status() -> serde_json::Value {
    #[cfg(feature = "cloud")]
    if let Some(s) = flow_cloud::catalog::default_store() {
        return serde_json::from_str(&s.status_json()).unwrap_or_default();
    }
    serde_json::Value::Null
}

/// The picker row of one model (`None` = not in the catalogue).
fn model_row(purpose: &str, id: &str) -> Option<serde_json::Value> {
    #[cfg(feature = "cloud")]
    {
        let store = flow_cloud::catalog::default_store()?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&store.search_json(purpose, id, r#"{"zdr_only":false}"#)).ok()?;
        rows.into_iter().find(|r| r["id"] == id)
    }
    #[cfg(not(feature = "cloud"))]
    {
        let _ = (purpose, id);
        None
    }
}

fn or_view(state: &AppState) -> crate::openrouter::OrView {
    let dev = state.prefs.read().unwrap().developer.clone();
    let router = state.router.get();
    let or = &dev.cloud.openrouter;
    crate::openrouter::view(
        &dev,
        crate::openrouter::ViewInputs {
            caps: BuildCaps::current(),
            key: credentials::key_state(&*state.creds, credentials::OPENROUTER),
            usage: router.map(|r| r.cloud_usage()).unwrap_or_default(),
            cleanup_status: router
                .map(|r| r.router().cleanup_status())
                .unwrap_or_default(),
            catalog: catalog_status(),
            stt_model_info: model_row("stt", &or.stt.model),
            cleanup_model_info: model_row("cleanup", &or.cleanup.model),
        },
    )
}

/// Blocking (≤ 10 s): refresh the catalogue unless the cache is fresh (or
/// `force`); with a stored key the account's privacy filter applies. Sends
/// no user data. Returns the status, plus `error` (a code) on failure.
fn refresh_catalog(router: &RouterAdapter, force: bool) -> serde_json::Value {
    #[cfg(feature = "cloud")]
    {
        let Some(store) = flow_cloud::catalog::default_store() else {
            return serde_json::json!({"error": "cloud_not_configured"});
        };
        let t0 = Instant::now();
        let err = router
            .router()
            .with_cloud_key(|k| store.refresh(k, force).err());
        let mut v: serde_json::Value =
            serde_json::from_str(&store.status_json()).unwrap_or_default();
        let ms = t0.elapsed().as_millis();
        debug_log::event(
            "cloud_catalog_refresh",
            &format!(
                "source={} models={} ms={ms}{}",
                v["source"].as_str().unwrap_or("?"),
                v["models"],
                err.as_ref()
                    .map(|e| format!(" error={}", e.code()))
                    .unwrap_or_default()
            ),
        );
        if let (Some(e), Some(o)) = (err, v.as_object_mut()) {
            o.insert("error".into(), e.code().into());
        }
        v
    }
    #[cfg(not(feature = "cloud"))]
    {
        let _ = (router, force);
        serde_json::json!({"error": "cloud_not_configured"})
    }
}

fn refresh_catalog_in_background(app: &AppHandle, force: bool) {
    let Some(r) = app.state::<AppState>().router.get().cloned() else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("flow-or-catalog".into())
        .spawn(move || {
            let _ = refresh_catalog(&r, force);
        });
}

#[tauri::command]
fn dev_or_get(state: tauri::State<'_, AppState>) -> crate::openrouter::OrView {
    or_view(&state)
}

/// Picker rows from memory / cache / the baked list (never the network).
/// `filters` = `{zdr_only, fast_only, free_only, stt_path}`.
#[tauri::command]
fn dev_or_catalog(purpose: String, query: String, filters: serde_json::Value) -> serde_json::Value {
    #[cfg(feature = "cloud")]
    if let Some(s) = flow_cloud::catalog::default_store() {
        return serde_json::from_str(&s.search_json(&purpose, &query, &filters.to_string()))
            .unwrap_or_else(|_| serde_json::json!([]));
    }
    let _ = (purpose, query, filters);
    serde_json::json!([])
}

#[tauri::command]
fn dev_or_catalog_status() -> serde_json::Value {
    catalog_status()
}

/// "Refresh models" / the page opening (blocking: off the UI thread).
#[tauri::command]
async fn dev_or_refresh_catalog(app: AppHandle, force: bool) -> Result<serde_json::Value, String> {
    let router = app
        .state::<AppState>()
        .router
        .get()
        .cloned()
        .ok_or("router not ready")?;
    tauri::async_runtime::spawn_blocking(move || refresh_catalog(&router, force))
        .await
        .map_err(|e| e.to_string())
}

/// "Check key": `GET /api/v1/key` with the stored key → credit limit,
/// remaining, monthly usage (never the key), or `{error: code}`.
#[tauri::command]
async fn dev_or_key_info(app: AppHandle) -> Result<serde_json::Value, String> {
    let router = app
        .state::<AppState>()
        .router
        .get()
        .cloned()
        .ok_or("router not ready")?;
    tauri::async_runtime::spawn_blocking(move || key_info(&router))
        .await
        .map_err(|e| e.to_string())
}

fn key_info(router: &RouterAdapter) -> serde_json::Value {
    #[cfg(feature = "cloud")]
    {
        router.router().with_cloud_key(|k| match k {
            None => serde_json::json!({"error": "cloud_not_configured"}),
            Some(k) => match flow_cloud::keyinfo::fetch(k) {
                Ok(i) => {
                    debug_log::event(
                        "cloud_key_info",
                        &format!(
                            "limit_remaining={:?} usage_monthly={:?}",
                            i.limit_remaining, i.usage_monthly
                        ),
                    );
                    let mut v = serde_json::to_value(&i).unwrap_or_default();
                    if let Some(o) = v.as_object_mut() {
                        o.insert("warn_no_limit".into(), i.warn_no_limit().into());
                    }
                    v
                }
                Err(e) => {
                    debug_log::event("cloud_key_info", &format!("error={}", e.code()));
                    serde_json::json!({"error": e.code()})
                }
            },
        })
    }
    #[cfg(not(feature = "cloud"))]
    {
        let _ = router;
        serde_json::json!({"error": "cloud_not_configured"})
    }
}

#[tauri::command]
fn dev_or_set_model(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    purpose: String,
    model: String,
) -> Result<crate::openrouter::OrView, String> {
    edit_cloud(&app, &state, |d| {
        crate::openrouter::apply_model(d, &purpose, &model)
    })
}

#[tauri::command]
fn dev_or_set_stt(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    form: crate::openrouter::SttForm,
) -> Result<crate::openrouter::OrView, String> {
    edit_cloud(&app, &state, |d| {
        crate::openrouter::apply_stt(d, &form, now_unix_ms())
    })
}

#[tauri::command]
fn dev_or_set_cleanup(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    form: crate::openrouter::CleanupForm,
) -> Result<crate::openrouter::OrView, String> {
    edit_cloud(&app, &state, |d| {
        crate::openrouter::apply_cleanup(d, &form, now_unix_ms())
    })
}

/// D36: the cleanup order on the AI cleanup page (order + this PC / your
/// server switches; OpenRouter's switch is `dev_or_set_cleanup`'s). In
/// every build: the answer's `cleanup.engines` is what the page shows.
#[tauri::command]
fn dev_cleanup_set_engines(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    engines: Vec<crate::dev_settings::CleanupEngine>,
) -> Result<crate::openrouter::OrView, String> {
    edit_cloud(&app, &state, |d| {
        crate::openrouter::apply_cleanup_engines(d, &engines)
    })
}

#[tauri::command]
fn dev_or_set_privacy(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    zdr: bool,
    data_collection: String,
) -> Result<crate::openrouter::OrView, String> {
    edit_cloud(&app, &state, |d| {
        crate::openrouter::apply_privacy(d, zdr, &data_collection)
    })
}

#[tauri::command]
fn dev_or_set_budget(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    budget: Option<f64>,
) -> Result<crate::openrouter::OrView, String> {
    edit_cloud(&app, &state, |d| crate::openrouter::apply_budget(d, budget))
}

/// The one-time migration notice was read.
#[tauri::command]
fn dev_or_ack_notice(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<crate::openrouter::OrView, String> {
    if let Some(r) = state.router.get() {
        r.router().ack_cloud_notice();
    }
    edit_cloud(&app, &state, |d| {
        crate::openrouter::ack_notice(d);
        Ok(())
    })
}

/// STT "Test": the bundled 3 s public-domain clip through the saved
/// speech-to-text settings; `key` = typed but not saved yet (`None` = the
/// stored one). Needs no consent (not the user's audio).
#[tauri::command]
async fn dev_or_test_stt(
    app: AppHandle,
    key: Option<String>,
) -> Result<flow_router::HealthReport, String> {
    let state = app.state::<AppState>();
    let router = state.router.get().cloned().ok_or("router not ready")?;
    let cfg = state
        .prefs
        .read()
        .unwrap()
        .developer
        .cloud
        .openrouter
        .clone();
    let key = match key.as_deref().map(str::trim) {
        Some(k) if !k.is_empty() => Some(flow_router::SecretString::new(
            credentials::normalize_key(k)?,
        )),
        _ => None,
    };
    tauri::async_runtime::spawn_blocking(move || router.router().probe_cloud(&cfg, key))
        .await
        .map_err(|e| e.to_string())
}

/// Cleanup "Test" result (a fixed sample sentence, never user text).
#[derive(serde::Serialize, Default)]
struct CleanupTest {
    ok: bool,
    text: Option<String>,
    error: Option<String>,
    message: Option<String>,
    latency_ms: Option<u32>,
    cost_usd: Option<f64>,
    model: String,
}

#[tauri::command]
async fn dev_or_test_cleanup(app: AppHandle) -> Result<CleanupTest, String> {
    let state = app.state::<AppState>();
    let router = state.router.get().cloned().ok_or("router not ready")?;
    let cfg = state
        .prefs
        .read()
        .unwrap()
        .developer
        .cloud
        .openrouter
        .clone();
    tauri::async_runtime::spawn_blocking(move || test_cleanup(&router, &cfg))
        .await
        .map_err(|e| e.to_string())
}

fn test_cleanup(router: &RouterAdapter, cfg: &flow_router::OpenRouterConfig) -> CleanupTest {
    let model = cfg.cleanup.model.clone();
    #[cfg(feature = "cloud")]
    {
        use flow_core::cleanup::Cleaner;
        let Some(key) = router.router().with_cloud_key(|k| k.cloned()) else {
            return CleanupTest {
                error: Some("cloud_not_configured".into()),
                message: Some("save an OpenRouter key first".into()),
                model,
                ..Default::default()
            };
        };
        let c = flow_cloud::OpenRouterCleaner::new(cfg.into(), key);
        let prompt = flow_core::cleanup::build_prompt(
            crate::openrouter::CLEANUP_SAMPLE,
            flow_core::config::CleanupLevel::Light,
            flow_core::config::Style::default(),
            &flow_core::config::FieldContext::default(),
            &flow_core::dictionary::Dictionary::default(),
        );
        let t0 = Instant::now();
        let deadline = t0 + Duration::from_millis(cfg.cleanup.timeout_ms.max(800) * 2);
        let r = c.clean(&prompt, deadline);
        let call = c.last_call().unwrap_or_default();
        let latency_ms = Some(t0.elapsed().as_millis() as u32);
        match r {
            Ok(text) => CleanupTest {
                ok: true,
                text: Some(text),
                latency_ms,
                cost_usd: call.cost_usd,
                model,
                ..Default::default()
            },
            Err(e) => CleanupTest {
                error: Some(
                    call.error
                        .map(str::to_string)
                        .unwrap_or_else(|| e.code().to_string()),
                ),
                latency_ms,
                model,
                ..Default::default()
            },
        }
    }
    #[cfg(not(feature = "cloud"))]
    {
        let _ = router;
        CleanupTest {
            error: Some("cloud_not_configured".into()),
            model,
            ..Default::default()
        }
    }
}

#[tauri::command]
fn dev_or_cleanup_status(state: tauri::State<'_, AppState>) -> flow_router::CleanupStatus {
    state
        .router
        .get()
        .map(|r| r.router().cleanup_status())
        .unwrap_or_default()
}

/// Save the OpenRouter key: Credential Manager (bound to openrouter.ai),
/// then the router. Never echoed back; the page gets `KeyState` only.
#[tauri::command]
fn dev_cloud_set_key(state: tauri::State<'_, AppState>, key: String) -> Result<KeyState, String> {
    let key = credentials::normalize_key(&key)?;
    credentials::store_openrouter_key(&*state.creds, &key)?;
    if let Some(r) = state.router.get() {
        r.load_cloud_keys(&*state.creds);
    }
    let dev = state.prefs.read().unwrap().developer.clone();
    refresh_debug_secrets(&state, &dev);
    Ok(credentials::key_state(
        &*state.creds,
        credentials::OPENROUTER,
    ))
}

#[tauri::command]
fn dev_cloud_clear_key(state: tauri::State<'_, AppState>) -> Result<KeyState, String> {
    state.creds.delete(credentials::OPENROUTER)?;
    if let Some(r) = state.router.get() {
        r.router().set_cloud_key(credentials::OPENROUTER, None);
    }
    Ok(credentials::key_state(
        &*state.creds,
        credentials::OPENROUTER,
    ))
}

#[tauri::command]
fn dev_cloud_usage(state: tauri::State<'_, AppState>) -> crate::cloud::UsageView {
    or_view(&state).usage
}

#[tauri::command]
fn dev_cloud_reset_usage(state: tauri::State<'_, AppState>) -> crate::cloud::UsageView {
    if let Some(r) = state.router.get() {
        r.router().reset_cloud_usage();
        persist_usage(&state, r);
    }
    dev_cloud_usage(state)
}

fn persist_usage(state: &AppState, r: &RouterAdapter) {
    if !r.cloud_capabilities().cloud {
        return;
    }
    if let Err(e) = crate::cloud::save_usage(&state.usage_path, &r.cloud_usage()) {
        eprintln!("[cloud] could not save usage: {e}");
        debug_log::event("cloud", &format!("could not save usage: {e}"));
    }
}

// ---------------------------------------------------------------- windows

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Show the Scratchpad (the Quick note), creating the window if it is closed.
fn show_scratchpad(app: &AppHandle) {
    show_notes_window(app, format!("note={}", notes::SCRATCHPAD));
}

/// Show the notes window on `scratchpad.html?<query>` (`note=<id>` = that
/// note in the editor, `view=notes` = the Notes list), creating it if it is
/// closed and navigating it if it shows something else (the page flushes
/// its note on the way out). Always on the main thread (the shortcuts
/// arrive on the controller thread).
fn show_notes_window(app: &AppHandle, query: String) {
    let h = app.clone();
    let _ = app.run_on_main_thread(move || {
        let page = format!("scratchpad.html?{query}");
        if let Some(w) = h.get_webview_window(SCRATCHPAD) {
            if let Ok(cur) = w.url() {
                if cur.query() != Some(query.as_str()) {
                    if let Ok(url) = cur.join(&page) {
                        let _ = w.navigate(url);
                    }
                }
            }
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            return;
        }
        let built = WebviewWindowBuilder::new(&h, SCRATCHPAD, WebviewUrl::App(page.into()))
            .title("Notes — Murmur")
            .inner_size(520.0, 560.0)
            .min_inner_size(320.0, 240.0)
            .center()
            .focused(true)
            .build();
        if let Err(e) = built {
            eprintln!("[scratchpad] {e}");
            debug_log::event("scratchpad", &e.to_string());
        }
    });
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

/// Click-through except while the cursor is over the locked pill or the
/// bubble's settings link, so the hands-free ✕/✓ and "Fix →" are clickable
/// but the transparent canvas never eats clicks.
fn spawn_hover_watch(
    app: AppHandle,
    state: Arc<Mutex<OverlayState>>,
    link: Arc<Mutex<Option<crate::overlay::Rect>>>,
) {
    let _ = std::thread::Builder::new()
        .name("flow-overlay-hover".into())
        .spawn(move || {
            let mut ignoring = true;
            loop {
                std::thread::sleep(Duration::from_millis(30));
                let st = state.lock().unwrap().clone();
                let link = *link.lock().unwrap();
                if st != OverlayState::ActiveLocked && link.is_none() && ignoring {
                    continue;
                }
                let Some(w) = app.get_webview_window(OVERLAY) else {
                    continue;
                };
                // A hidden overlay has no link to click, whatever the page said last.
                let link = link.filter(|_| w.is_visible().unwrap_or(false));
                let want = match hwnd_of(&w) {
                    Some(h) if st == OverlayState::ActiveLocked || link.is_some() => {
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
                            link,
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

    /// At startup: when launch at login is on, write the Run value again so
    /// it names this exe. `set` otherwise runs only on a prefs save, so after
    /// a reinstall to another folder (the "Flow Clone" → "Murmur"
    /// rename moved it) the value kept pointing at a deleted exe and the app
    /// silently stopped starting. Idempotent; never turns it on when off.
    pub fn refresh(enabled: bool) {
        if enabled {
            if let Err(e) = set(true) {
                eprintln!("[autostart] could not refresh the Run value: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------- entry

pub fn run() {
    // Before anything else starts: a second launch only brings this one's
    // window forward and exits (one tray icon, one hook).
    let instance = crate::single_instance::win::claim_or_exit();
    let appdata = std::env::var("APPDATA").ok();
    let path = Prefs::default_path(appdata.as_deref());
    let (mut prefs, outcome) = Prefs::load(&path);
    if let LoadOutcome::Corrupt(bak) = &outcome {
        eprintln!("[prefs] corrupt prefs moved to {}", bak.display());
    }
    let creds = credentials::system_store();
    let usage_path = crate::cloud::usage_path(appdata.as_deref());
    // D28, once: a D18 provider list becomes the single OpenRouter config,
    // other providers' keys leave Credential Manager (before any router or
    // key load sees them).
    let migrated = crate::cloud::migrate_d28(&mut prefs.developer, &*creds, &usage_path);
    if migrated.is_some() {
        if let Err(e) = prefs.save(&path) {
            eprintln!("[cloud] could not save the migrated settings: {e}");
        }
    }
    autostart::refresh(prefs.launch_at_login);
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
    let recent_apps = Arc::new(Mutex::new(RecentAppStore::open(
        &recent_apps::default_path(appdata.as_deref()),
    )));
    let mut stats = InsightsStore::open(&insights::default_path(appdata.as_deref()));
    if !stats.data.backfilled {
        // First run: start from the transcripts already kept. Attempted
        // once only: a later retry would count live-recorded rows twice.
        if let Some(h) = &history {
            match stats.backfill_once(&h.lock().unwrap(), &insights::local_date) {
                Ok(n) => eprintln!("[insights] counted {n} existing transcripts"),
                Err(e) => eprintln!("[insights] backfill: {e}"),
            }
        }
        stats.data.backfilled = true;
        if let Err(e) = stats.save() {
            eprintln!("[insights] {e}");
        }
    }
    let insights_store = Arc::new(Mutex::new(stats));
    // Notes: the pre-Notes Scratchpad file moves into notes\ (once).
    let notes_dir = notes::default_dir(appdata.as_deref());
    match notes::migrate_scratchpad(&notes::app_dir(appdata.as_deref()), &notes_dir) {
        Ok(true) => eprintln!("[notes] moved scratchpad.txt into {}", notes_dir.display()),
        Ok(false) => {}
        Err(e) => eprintln!("[notes] could not move scratchpad.txt: {e}"),
    }
    let debug_path = debug_log::default_path(appdata.as_deref());
    let debug = Arc::new(DebugLog::file(
        debug_path.clone(),
        prefs.read().unwrap().developer.debug_log,
    ));
    debug_log::install(debug.clone());
    if let Some(m) = &migrated {
        // Ids and counts only (never a key).
        debug_log::event(
            "cloud_migrate_d28",
            &format!(
                "removed={} moved_key={} deleted_keys={} key_errors={} cloud_disabled={} model_changed={}",
                m.report.removed.len(),
                m.keys.moved_from.is_some(),
                m.keys.deleted.len(),
                m.keys.errors.len(),
                m.report.cloud_disabled,
                m.report.model_to.is_some()
            ),
        );
    }
    // The model catalogue's cache (public data; fetched only when the
    // OpenRouter page opens or a cloud use is on).
    #[cfg(feature = "cloud")]
    flow_cloud::catalog::set_default_store(Some(Arc::new(flow_cloud::CatalogStore::new(
        crate::cloud::catalog_cache_path(appdata.as_deref()),
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
            overlay_link: Arc::new(Mutex::new(None)),
            history: history.clone(),
            insights: insights_store.clone(),
            suggestions: suggestions.clone(),
            recent_apps: recent_apps.clone(),
            suggest_item: OnceLock::new(),
            creds,
            usage_path,
            debug: debug.clone(),
            debug_path,
            notes: Arc::new(Mutex::new(AutoSave::new(
                NoteStore::new(notes_dir),
                notes::SAVE_DEBOUNCE_MS,
            ))),
            started: Instant::now(),
            downloads: Arc::new(crate::model_download::Downloads::default()),
        })
        .invoke_handler(tauri::generate_handler![
            get_prefs,
            keyboard_language,
            set_prefs,
            list_microphones,
            get_model_status,
            overlay_action,
            overlay_theme,
            history_list,
            history_delete,
            history_clear,
            history_copy,
            history_repaste,
            history_retry,
            insights_get,
            insights_reset,
            clipboard_copy,
            note_load,
            note_edit,
            note_flush,
            note_list,
            note_create,
            note_delete,
            note_set_pinned,
            note_export,
            note_open,
            suggestions_list,
            suggestion_accept,
            suggestion_dismiss,
            app_style_choices,
            recent_apps_clear,
            dictionary_import,
            dictionary_export,
            dictionary_export_text,
            pick_path,
            open_external,
            model_catalog,
            llm_backend,
            model_download_start,
            model_download_cancel,
            model_delete,
            transforms_reset,
            transform_try,
            hotkey_capture_start,
            hotkey_capture_stop,
            dev_get,
            dev_validate,
            dev_set,
            dev_test_connection,
            dev_test_session,
            dev_route_status,
            dev_reset_circuit,
            dev_reset_defaults,
            dev_unlock,
            dev_cloud_set_key,
            dev_cloud_clear_key,
            dev_cloud_usage,
            dev_cloud_reset_usage,
            dev_or_get,
            dev_or_catalog,
            dev_or_catalog_status,
            dev_or_refresh_catalog,
            dev_or_key_info,
            dev_or_set_model,
            dev_or_set_stt,
            dev_or_set_cleanup,
            dev_cleanup_set_engines,
            dev_or_set_privacy,
            dev_or_set_budget,
            dev_or_ack_notice,
            dev_or_test_stt,
            dev_or_test_cleanup,
            dev_or_cleanup_status,
            debug_log_status,
            debug_log_set,
            debug_log_clear,
            debug_log_open_folder
        ])
        .on_window_event(|window, event| {
            if window.label() == SETTINGS {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // Save on close: the page sent its last edit already.
            if window.label() == SCRATCHPAD && matches!(event, WindowEvent::Destroyed) {
                let st = window.app_handle().state::<AppState>();
                // Whichever note it showed (one window, one note at a time).
                log_note_writes(st.notes.lock().unwrap().flush_all());
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let h = handle.clone();
            instance.listen(move || show_settings(&h));

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
            let scratch = MenuItem::with_id(app, "scratchpad", "Scratchpad…", true, None::<&str>)?;
            let notes_item = MenuItem::with_id(app, "notes", "Notes…", true, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Settings…", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Murmur", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &model_item,
                    &llm_item,
                    &suggest_item,
                    &sep0,
                    &scratch,
                    &notes_item,
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
                    "scratchpad" => show_scratchpad(app),
                    "notes" => show_notes_window(app, "view=notes".into()),
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
            spawn_hover_watch(
                handle.clone(),
                overlay_state.clone(),
                app.state::<AppState>().overlay_link.clone(),
            );
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
                let h = handle.clone();
                core.on_language_status(move |st| {
                    let _ = h.emit_to(SETTINGS, "model:hebrew", st);
                });
                let p = prefs.read().unwrap().clone();
                if want_local(&p) {
                    core.load(ModelSpec::from_prefs(&p, appdata.as_deref()));
                    core.load_language(LanguageSpec::from_prefs(&p, appdata.as_deref()));
                    core.load_llm(LlmSpec::from_prefs(&p, appdata.as_deref()));
                } else {
                    let _ = model_item
                        .set_text("Speech model: loads on demand if the server is unreachable");
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
                debug_log::event("settings", &format!("invalid developer settings: {e}"));
                router_adapter::router_config(&DevSettings::default(), caps)
                    .expect("default router config")
            });
            let router = RouterAdapter::new(cfg, local, router_adapter::remote_factory());
            {
                let p = prefs.read().unwrap();
                router.set_on_demand_with(
                    on_demand_spec(&p, appdata.as_deref()),
                    LanguageSpec::from_prefs(&p, appdata.as_deref()),
                );
            }
            // D18: the key from Credential Manager, last counters from disk.
            {
                let st = app.state::<AppState>();
                router.load_cloud_keys(&*st.creds);
                refresh_debug_secrets(&st, &dev);
                if router.cloud_capabilities().cloud {
                    if let Some(u) = crate::cloud::load_usage(&st.usage_path) {
                        router.router().set_cloud_usage(u);
                    }
                }
            }
            {
                let h = handle.clone();
                let p = prefs.clone();
                let r = Arc::downgrade(&router);
                router.on_route(move |last| {
                    // S/L only when asked for; C and cloud failures always (C7).
                    if p.read().unwrap().developer.show_engine_badge || last.always_badge() {
                        let _ = h.emit_to(OVERLAY, "dictation:route", last.badge());
                    }
                    if let Some(r) = r.upgrade() {
                        if last.engine.as_deref() == Some("cloud")
                            || last.llm_engine.as_deref() == Some("cloud")
                        {
                            persist_usage(&h.state::<AppState>(), &r);
                        }
                        let _ = h.emit_to(SETTINGS, "dev:route", r.route_view());
                    }
                });
            }
            let _ = app.state::<AppState>().router.set(router.clone());
            probe_in_background(&handle);
            // A cloud use is on: keep the catalogue (model paths, prices)
            // current. Never while both uses are off.
            if caps.cloud && (dev.cloud_effective(caps) || dev.cleanup_effective(caps)) {
                refresh_catalog_in_background(&handle, false);
            }
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
            // Our own windows (Scratchpad) are style Other and never
            // watched for edits.
            if let Ok(exe) = std::env::current_exe() {
                controller = controller.with_own_exe(&exe.to_string_lossy());
            }
            controller = controller
                .with_insights(insights_store.clone())
                .with_suggestions(suggestions.clone())
                .with_recent_apps(recent_apps.clone())
                .with_debug_log(debug.clone());
            let pending = suggestions.lock().unwrap().data.pending.len();
            show_suggestion_count(&handle, pending);
            let dictating = controller.dictating_flag();
            let _ = app.state::<AppState>().dictating.set(dictating.clone());
            std::thread::Builder::new()
                .name("flow-controller".into())
                .spawn(move || controller.run(rx))?;

            let (hk_tx, hk_rx) = mpsc::channel();
            let bindings = prefs.read().unwrap().hotkey_bindings();
            match hotkeys::win::install(bindings, dictating, hk_tx) {
                Ok(h) => {
                    *app.state::<AppState>().hook.lock().unwrap() = Some(h);
                }
                Err(e) => {
                    eprintln!("[hotkeys] {e}");
                    debug_log::event("hotkeys", &e.to_string());
                }
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

            spawn_note_saver(handle.clone());
            platform.overlay(&OverlayState::Resting);
            if matches!(outcome, LoadOutcome::Missing) {
                show_settings(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Tauri app")
        .run(|app, event| match event {
            // Closing the settings window must not quit the tray app.
            RunEvent::ExitRequested {
                api, code: None, ..
            } => api.prevent_exit(),
            RunEvent::Exit => {
                let st = app.state::<AppState>();
                if let Some(r) = st.router.get() {
                    persist_usage(&st, r);
                }
                // Quit with the Scratchpad open: its last edits.
                log_note_writes(st.notes.lock().unwrap().flush_all());
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_may_open_only_the_listed_settings_pages() {
        for p in OVERLAY_SETTINGS_PAGES {
            assert_eq!(overlay_settings_page(Some(p)), Ok(*p));
        }
        assert!(overlay_settings_page(Some("settings-developer")).is_err());
        assert!(overlay_settings_page(Some("")).is_err());
        assert!(overlay_settings_page(None).is_err());
    }
}
