//! Macafon UI (Tauri v2). The daemon keeps this process warm in the background with the
//! hidden quick panel (owner decision 2026-10-17, against NFR-R5), so panels open at once;
//! closing the main window keeps the process, the daemon going away ends it. A second launch
//! shows the main window (BG-006) or toggles a panel (`--quick-panel`, `--overlay`).

// Release builds on Windows open no console window next to the UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod quick_panel;

use bridge::{Bridge, BridgeError, Sink, Status};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

/// Windows that asked for live levels. The daemon streams them per connection and every
/// window shares this one, so the stream stops only when the last window lets it go, and the
/// 60 Hz events reach only the windows that show them (the warm quick panel stays out of that
/// path unless its overlay shows the microphone level).
#[derive(Default)]
struct MeterWindows(std::sync::Mutex<std::collections::BTreeSet<String>>);

impl MeterWindows {
    fn labels(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<String>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

struct AppSink {
    app: AppHandle,
    /// The daemon was reached: losing it later ends a UI that has no window open.
    connected: std::sync::atomic::AtomicBool,
}

impl Sink for AppSink {
    fn event(&self, event: Value) {
        // `ui.show` is for this process, not the pages.
        if event["event"] == "ui.show" {
            let request = match event["data"]["target"].as_str() {
                Some("tray_panel") => Some(quick_panel::Mode::Tray),
                Some("overlay") => Some(quick_panel::Mode::Overlay),
                _ => None,
            };
            open_requested(&self.app, Some(request));
            return;
        }
        let result = if matches!(event["event"].as_str(), Some("meters" | "mixer.meters")) {
            let labels = self.app.state::<MeterWindows>().labels().clone();
            labels
                .iter()
                .try_for_each(|label| self.app.emit_to(label.as_str(), "daemon://event", &event))
        } else if event["event"] == "devices.meter" {
            self.app.emit_to("main", "daemon://event", event)
        } else {
            self.app.emit("daemon://event", event)
        };
        if let Err(error) = result {
            tracing::warn!(%error, "cannot forward a daemon event");
        }
    }

    fn status(&self, status: &Status) {
        use std::sync::atomic::Ordering;
        let connected = matches!(status, Status::Connected { .. });
        let was = self.connected.swap(connected, Ordering::SeqCst);
        if !connected {
            // A new connection streams nothing until a window subscribes again.
            self.app.state::<MeterWindows>().labels().clear();
        }
        // The daemon quit (tray, `ctl quit`, an upgrade): a warm UI without its main window
        // goes too, instead of waiting for a daemon that is not coming back.
        if was && !connected && self.app.get_webview_window("main").is_none() {
            self.app.exit(0);
            return;
        }
        if let Err(error) = self.app.emit("daemon://status", status) {
            tracing::warn!(%error, "cannot forward the daemon status");
        }
    }
}

#[tauri::command]
async fn daemon_call(
    bridge: tauri::State<'_, Bridge>,
    app: AppHandle,
    window: tauri::Window,
    method: String,
    params: Option<Value>,
) -> Result<Value, BridgeError> {
    let meters = app.state::<MeterWindows>();
    match method.as_str() {
        "meters.subscribe" => {
            meters.labels().insert(window.label().to_owned());
        }
        "meters.unsubscribe" => {
            let mut labels = meters.labels();
            labels.remove(window.label());
            if !labels.is_empty() {
                return Ok(Value::Null);
            }
        }
        _ => {}
    }
    let bridge = bridge.inner().clone();
    let quit = method == "daemon.quit";
    let result = tauri::async_runtime::spawn_blocking(move || bridge.call(&method, params))
        .await
        .unwrap_or_else(|error| {
            Err(BridgeError {
                code: "internal".into(),
                message: error.to_string(),
            })
        });
    // "Quit Macafon" stops the daemon and the UI together (BG-005).
    if quit && result.is_ok() {
        app.exit(0);
    }
    result
}

/// The official VB-CABLE page (WIN-MIC-002). Only this fixed URL can be opened, in the
/// user's browser; Macafon itself never downloads the driver.
const VB_CABLE_URL: &str = "https://vb-audio.com/Cable/";

#[tauri::command]
fn open_vb_cable_page() -> Result<(), BridgeError> {
    let opener = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(VB_CABLE_URL)
        .spawn()
        .map(drop)
        .map_err(|error| BridgeError {
            code: "internal".into(),
            message: format!("cannot open {VB_CABLE_URL}: {error}"),
        })
}

/// The system folder picker for the soundpad library (SND-001); any drive. `None` when the
/// user cancels.
#[tauri::command]
async fn pick_folder(app: AppHandle, current: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file();
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    if let Some(current) = current.filter(|path| std::path::Path::new(path).is_dir()) {
        dialog = dialog.set_directory(current);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .ok()??;
    picked
        .into_path()
        .ok()
        .map(|path| path.display().to_string())
}

/// The main window from `tauri.conf.json`, created on demand: the UI starts without it when
/// the daemon warms it up or a panel asks.
fn show_main(app: &AppHandle) -> bool {
    // Serialize startup, second launches and panel requests without blocking the UI thread.
    static MAIN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = MAIN.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(window) = app.get_webview_window("main") {
        if !window.is_visible().unwrap_or(true) {
            place_main(app, &window);
        }
        foreground_main(&window);
        return true;
    }
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else {
        return false;
    };
    match tauri::WebviewWindowBuilder::from_config(app, config)
        .and_then(|b| b.visible(false).build())
    {
        Ok(window) => {
            place_main(app, &window);
            foreground_main(&window);
            let app = app.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    main_closed(&app);
                }
            });
            true
        }
        Err(error) => {
            tracing::warn!(%error, "cannot open the main window");
            false
        }
    }
}

/// A saved monitor only affects a new or hidden window, never a visible workspace.
fn place_main(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Some(name) = app
        .state::<Bridge>()
        .call("prefs.get", None)
        .ok()
        .and_then(|prefs| prefs["main_monitor"].as_str().map(str::to_owned))
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    let Some(monitor) = app.available_monitors().ok().and_then(|monitors| {
        monitors
            .into_iter()
            .find(|monitor| monitor.name() == Some(&name))
    }) else {
        return;
    };
    // Move to the monitor first so the size reflects its DPI before centering.
    let area = monitor.work_area();
    let _ = window.set_position(area.position);
    if let Ok(size) = window.outer_size() {
        let _ = window.set_position(tauri::PhysicalPosition::new(
            area.position.x + (i64::from(area.size.width) - i64::from(size.width)) as i32 / 2,
            area.position.y + (i64::from(area.size.height) - i64::from(size.height)) as i32 / 2,
        ));
    }
}

fn foreground_main(window: &tauri::WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        let hwnd = windows::Win32::Foundation::HWND(hwnd.0);
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SetForegroundWindow, SetWindowPos,
        };
        // SAFETY: the HWND belongs to this live window. No pointers are retained;
        // temporary topmost promotion is immediately undone, preserving normal stacking.
        unsafe {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
            let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
    }
    let _ = window.set_focus();
}

/// The process stays warm without the main window: stop what only the window wanted (meters,
/// device levels; NFR-R7) and tell the daemon, which may quit or show its tray hint. With no
/// daemon to talk to there is nothing to stay warm for.
fn main_closed(app: &AppHandle) {
    let bridge = app.state::<Bridge>().inner().clone();
    if !matches!(bridge.status(), Status::Connected { .. }) {
        app.exit(0);
        return;
    }
    std::thread::spawn(move || {
        for method in [
            "meters.unsubscribe",
            "devices.unsubscribe",
            "ui.window_closed",
        ] {
            if let Err(error) = bridge.call(method, None) {
                tracing::debug!(method, error = %error.message, "after the window closed");
            }
        }
    });
}

/// Launch arguments and `ui.show`: off the event thread, where creating a WebView2 window
/// would deadlock.
fn open_requested(app: &AppHandle, request: Option<Option<quick_panel::Mode>>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || match request {
        Some(Some(mode)) => quick_panel::toggle(&app, mode),
        Some(None) => {
            quick_panel::hide(&app);
            show_main(&app);
        }
        // `--background`: only the hidden panel, already there.
        None => {}
    });
}

/// `None` for `--background`, else the panel mode or `Some(None)` for the main window.
fn request_from_args(args: &[String]) -> Option<Option<quick_panel::Mode>> {
    if args.iter().any(|arg| arg == macafon_ipc::BACKGROUND_ARG) {
        return None;
    }
    Some(quick_panel::Mode::from_args(args))
}

#[tauri::command]
fn daemon_status(bridge: tauri::State<'_, Bridge>) -> Status {
    bridge.status()
}

#[tauri::command]
fn daemon_retry(bridge: tauri::State<'_, Bridge>) {
    bridge.retry();
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "macafon_ui=info".into()),
        )
        .init();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            open_requested(app, request_from_args(&args));
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();
            let endpoint = macafon_ipc::Endpoint::for_current_user()
                .ok_or("cannot find the daemon endpoint for the current user")?;
            let bridge = Bridge::start(
                endpoint,
                AppSink {
                    app: app.handle().clone(),
                    connected: std::sync::atomic::AtomicBool::new(false),
                },
                bridge::launch_daemon,
            );
            app.manage(MeterWindows::default());
            app.manage(bridge);
            app.manage(quick_panel::PanelState::default());
            // The panel page loads now, so `ui.show` later only has to show it.
            quick_panel::prewarm(app.handle());
            open_requested(app.handle(), request_from_args(&args));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daemon_call,
            daemon_status,
            daemon_retry,
            open_vb_cable_page,
            pick_folder,
            quick_panel::quick_panel_fit,
            quick_panel::quick_panel_pending,
            quick_panel::quick_panel_close,
            quick_panel::open_main
        ])
        .build(tauri::generate_context!());
    match result {
        // Closing the last window keeps the process warm; it ends with the daemon or on Quit.
        Ok(app) => app.run(|_, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        }),
        Err(error) => {
            tracing::error!(%error, "Macafon UI failed");
            std::process::exit(1);
        }
    }
}
