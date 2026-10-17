//! Macafon UI (Tauri v2). Closing the window ends this process; audio keeps running in macafond
//! (BG-001). A second launch focuses the existing window (BG-006), or toggles the quick panel
//! when it carries `--quick-panel`.

// Release builds on Windows open no console window next to the UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod quick_panel;

use bridge::{Bridge, BridgeError, Sink, Status};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

struct AppSink(AppHandle);

impl Sink for AppSink {
    fn event(&self, event: Value) {
        if let Err(error) = self.0.emit("daemon://event", event) {
            tracing::warn!(%error, "cannot forward a daemon event");
        }
    }

    fn status(&self, status: &Status) {
        if let Err(error) = self.0.emit("daemon://status", status) {
            tracing::warn!(%error, "cannot forward the daemon status");
        }
    }
}

#[tauri::command]
async fn daemon_call(
    bridge: tauri::State<'_, Bridge>,
    app: AppHandle,
    method: String,
    params: Option<Value>,
) -> Result<Value, BridgeError> {
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

/// The main window from `tauri.conf.json`, created on demand: a launch for the quick panel
/// alone does not open it.
fn show_main(app: &AppHandle) -> bool {
    // Serialize startup, second launches and panel requests without blocking the UI thread.
    static MAIN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = MAIN.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return true;
    }
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else {
        return false;
    };
    // WebView2 can temporarily take focus before Tauri registers the new main window.
    // Keep the sole panel alive until creation finishes, or closing it can exit the process.
    quick_panel::OPENING_MAIN.store(true, std::sync::atomic::Ordering::SeqCst);
    let built = tauri::WebviewWindowBuilder::from_config(app, config).and_then(|b| b.build());
    quick_panel::OPENING_MAIN.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Err(error) = built {
        tracing::warn!(%error, "cannot open the main window");
        return false;
    }
    true
}

fn open_requested(app: &AppHandle, args: &[String]) {
    let mode = quick_panel::Mode::from_args(args);
    let app = app.clone();
    // WebView2 creation must not block Tauri's Windows event/IPC thread.
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(mode) = mode {
            quick_panel::toggle(&app, mode);
        } else if show_main(&app) {
            quick_panel::quick_panel_close(app);
        }
    });
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
            open_requested(app, &args);
        }))
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();
            let endpoint = macafon_ipc::Endpoint::for_current_user()
                .ok_or("cannot find the daemon endpoint for the current user")?;
            let bridge = Bridge::start(
                endpoint,
                AppSink(app.handle().clone()),
                bridge::launch_daemon,
            );
            app.manage(bridge);
            app.manage(quick_panel::PanelState::default());
            open_requested(app.handle(), &args);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daemon_call,
            daemon_status,
            daemon_retry,
            open_vb_cable_page,
            quick_panel::quick_panel_fit,
            quick_panel::quick_panel_close,
            quick_panel::open_main
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|_, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
                && (quick_panel::REPLACING.load(std::sync::atomic::Ordering::SeqCst)
                    || quick_panel::OPENING_MAIN.load(std::sync::atomic::Ordering::SeqCst))
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
