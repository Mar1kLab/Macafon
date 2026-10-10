//! micEQ UI (Tauri v2). Closing the window ends this process; audio keeps running in miceqd
//! (BG-001). A second launch focuses the existing window (BG-006).

mod bridge;

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
    // "Quit micEQ" stops the daemon and the UI together (BG-005).
    if quit && result.is_ok() {
        app.exit(0);
    }
    result
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
                .unwrap_or_else(|_| "miceq_ui=info".into()),
        )
        .init();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let socket = miceq_ipc::socket_path().ok_or("XDG_RUNTIME_DIR is not set")?;
            let bridge =
                Bridge::start(socket, AppSink(app.handle().clone()), bridge::launch_daemon);
            app.manage(bridge);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daemon_call,
            daemon_status,
            daemon_retry
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        tracing::error!(%error, "micEQ UI failed");
        std::process::exit(1);
    }
}
