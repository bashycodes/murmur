//! Tauri 2 shell (Windows only): tray, overlay pill, settings window, and
//! the Win32 [`Platform`] implementation the controller drives.

use crate::audio::capture::{self, Capture};
use crate::context::{uia, RawSnapshot};
use crate::controller::{AudioSink, Controller, Platform};
use crate::core_adapter::{self, FlowCoreAdapter, ModelSpec, ModelStatus};
use crate::dictation::{Input, OverlayState, Sound};
use crate::ducking::win::Ducker;
use crate::hotkeys::{self, win::HookHandle};
use crate::paste::{self, PasteOptions};
use crate::prefs::{LoadOutcome, Prefs};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow, WindowEvent};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, ShowWindow, SystemParametersInfoW, GWL_EXSTYLE,
    SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

const OVERLAY: &str = "overlay";
const SETTINGS: &str = "settings";

struct AppState {
    prefs: Arc<RwLock<Prefs>>,
    path: PathBuf,
    appdata: Option<String>,
    hook: Mutex<Option<HookHandle>>,
    core: OnceLock<Arc<FlowCoreAdapter>>,
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

    fn send_enter(&self) -> Result<(), String> {
        use crate::input::{vk, Stroke};
        let s = [Stroke::down(vk::RETURN), Stroke::up(vk::RETURN)];
        if crate::input::win::send(&s) == s.len() {
            Ok(())
        } else {
            Err("could not send Enter".into())
        }
    }
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

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_prefs(state: tauri::State<'_, AppState>) -> Prefs {
    state.prefs.read().unwrap().clone()
}

#[tauri::command]
fn set_prefs(state: tauri::State<'_, AppState>, prefs: Prefs) -> Result<(), String> {
    prefs.validate()?;
    prefs.save(&state.path).map_err(|e| e.to_string())?;
    if let Some(h) = state.hook.lock().unwrap().as_ref() {
        h.set_bindings(prefs.shortcuts.clone());
    }
    let _ = autostart::set(prefs.launch_at_login);
    let reload = {
        let old = state.prefs.read().unwrap();
        core_adapter::needs_reload(&old, &prefs) || !model_ready(&state)
    };
    if reload {
        if let Some(core) = state.core.get() {
            core.load(ModelSpec::from_prefs(&prefs, state.appdata.as_deref()));
        }
    }
    *state.prefs.write().unwrap() = prefs;
    Ok(())
}

fn model_ready(state: &AppState) -> bool {
    state.core.get().is_some_and(|c| c.status().is_ready())
}

#[derive(serde::Serialize)]
struct ModelInfo {
    status: ModelStatus,
    default_path: String,
    models_dir: String,
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
        models_dir: core_adapter::models_dir(ad).display().to_string(),
    }
}

#[tauri::command]
fn list_microphones() -> Vec<String> {
    capture::list_input_devices()
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

    tauri::Builder::default()
        .manage(AppState {
            prefs: prefs.clone(),
            path,
            appdata: appdata.clone(),
            hook: Mutex::new(None),
            core: OnceLock::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_prefs,
            set_prefs,
            list_microphones,
            get_model_status
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
            let sep0 = PredefinedMenuItem::separator(app)?;
            let open = MenuItem::with_id(app, "open", "Settings…", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Flow Clone", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&model_item, &sep0, &open, &sep, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip(TRAY_TIP)
                .menu(&menu)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "open" | "model" => show_settings(app),
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
            let platform = Arc::new(WinPlatform {
                app: handle.clone(),
                prefs: prefs.clone(),
                capture: Mutex::new(None),
                ducker: Ducker::spawn(),
            });
            // Load the model in the background; the tray notice tracks it.
            let core = core_adapter::make_core(&prefs.read().unwrap(), appdata.as_deref());
            {
                let h = handle.clone();
                let item = model_item.clone();
                core.on_status(move |st| apply_model_status(&h, &item, st));
            }
            let _ = app.state::<AppState>().core.set(core.clone());
            let start = Instant::now();
            let mut controller = Controller::new(
                platform.clone(),
                core,
                prefs.clone(),
                tx.clone(),
                Box::new(move || start.elapsed().as_millis() as u64),
            );
            if let Some(h) = history.clone() {
                controller = controller.with_history(h);
            }
            let dictating = controller.dictating_flag();
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
