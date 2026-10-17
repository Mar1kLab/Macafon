//! The quick panel: every volume and the microphone switches in a small window above the
//! taskbar, or the centered hotkey overlay. A second launch toggles the requested mode.
//! It closes on Esc, on the hotkey again and when it loses focus; with no main window open
//! the process ends with it.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "quick-panel";
const MARGIN: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Tray,
    Overlay,
}

impl Mode {
    pub fn from_args(args: &[String]) -> Option<Self> {
        if args.iter().any(|arg| arg == macafon_ipc::OVERLAY_ARG) {
            Some(Self::Overlay)
        } else if args.iter().any(|arg| arg == macafon_ipc::QUICK_PANEL_ARG) {
            Some(Self::Tray)
        } else {
            None
        }
    }

    fn url(self) -> &'static str {
        match self {
            Self::Tray => "index.html?view=quick-panel&mode=tray",
            Self::Overlay => "index.html?view=quick-panel&mode=overlay",
        }
    }
}

#[derive(Default)]
pub struct PanelState(Mutex<Option<(Mode, Monitor)>>);

/// A hidden window can gain and lose focus during creation; close only after shown focus.
static SHOWN: AtomicBool = AtomicBool::new(false);
static FOCUSED: AtomicBool = AtomicBool::new(false);
pub static REPLACING: AtomicBool = AtomicBool::new(false);
pub static OPENING_MAIN: AtomicBool = AtomicBool::new(false);

/// Called on a worker: WebView2 creation on the event/IPC thread can deadlock on Windows.
pub fn toggle(app: &AppHandle, mode: Mode) {
    static TOGGLE: Mutex<()> = Mutex::new(());
    let _guard = TOGGLE.lock().unwrap_or_else(|error| error.into_inner());
    let state = app.state::<PanelState>();
    let same_mode = state
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .is_some_and(|(current, _)| *current == mode);
    if let Some(window) = app.get_webview_window(LABEL) {
        if same_mode {
            let _ = window.close();
            return;
        }
        SHOWN.store(false, Ordering::SeqCst);
        FOCUSED.store(false, Ordering::SeqCst);
        REPLACING.store(true, Ordering::SeqCst);
        // destroy() only queues a request. Wait off the event thread until its label is
        // removed, otherwise building the replacement can fail with DuplicateWindowLabel.
        let (destroyed, wait) = std::sync::mpsc::channel();
        let handle = app.clone();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                let done = destroyed.clone();
                let _ = handle.run_on_main_thread(move || {
                    let _ = done.send(());
                });
            }
        });
        if let Err(error) = window.destroy() {
            REPLACING.store(false, Ordering::SeqCst);
            tracing::warn!(%error, "cannot replace the quick panel");
            return;
        }
        if let Err(error) = wait.recv_timeout(std::time::Duration::from_secs(5)) {
            REPLACING.store(false, Ordering::SeqCst);
            tracing::warn!(%error, "timed out replacing the quick panel");
            return;
        }
    }
    let monitor = target_monitor(app, mode);
    *state.0.lock().unwrap_or_else(|error| error.into_inner()) =
        monitor.map(|monitor| (mode, monitor));
    open(app, mode);
    REPLACING.store(false, Ordering::SeqCst);
}

/// Capture the foreground monitor before the new panel takes focus.
fn target_monitor(app: &AppHandle, mode: Mode) -> Option<Monitor> {
    #[cfg(windows)]
    if mode == Mode::Overlay {
        use windows::Win32::{
            Foundation::RECT,
            UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect},
        };
        let mut rect = RECT::default();
        // SAFETY: Windows owns the foreground handle; rect is a live output buffer.
        if unsafe { GetWindowRect(GetForegroundWindow(), &mut rect) }.is_ok()
            && let Ok(Some(monitor)) = app.monitor_from_point(
                (f64::from(rect.left) + f64::from(rect.right)) / 2.0,
                (f64::from(rect.top) + f64::from(rect.bottom)) / 2.0,
            )
        {
            return Some(monitor);
        }
    }
    #[cfg(not(windows))]
    let _ = mode;
    app.cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

fn open(app: &AppHandle, mode: Mode) {
    SHOWN.store(false, Ordering::SeqCst);
    FOCUSED.store(false, Ordering::SeqCst);
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(mode.url().into()))
        .title("Macafon quick panel")
        .inner_size(360.0, 420.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        // Windows 11 gives an undecorated window with a shadow its rounded corners and border.
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        // Shown by `quick_panel_fit` once the content has its height: no empty frame flashes.
        .visible(false)
        .background_color(tauri::window::Color(0x10, 0x10, 0x14, 0xff)) // --bg-raised
        .build();
    match built {
        Ok(window) => {
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(focused) = event {
                    if *focused {
                        if SHOWN.load(Ordering::SeqCst) {
                            FOCUSED.store(true, Ordering::SeqCst);
                        }
                    } else if FOCUSED.load(Ordering::SeqCst) && !OPENING_MAIN.load(Ordering::SeqCst)
                    {
                        let _ = handle.close();
                    }
                }
            });
        }
        Err(error) => tracing::warn!(%error, "cannot open the quick panel"),
    }
}

/// Move first, then size in the destination monitor's pixels to avoid double DPI scaling.
fn place(app: &AppHandle, window: &WebviewWindow, width: f64, height: f64) -> tauri::Result<()> {
    let state = app.state::<PanelState>();
    let placement = state
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let Some((mode, monitor)) = placement else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let margin = (MARGIN * scale).round() as i32;
    let available_width = (area.size.width as i32 - 2 * margin).max(1);
    let available_height = (area.size.height as i32 - 2 * margin).max(1);
    let max_width = match mode {
        Mode::Tray => 360.0,
        Mode::Overlay => 880.0,
    };
    let width = ((width.clamp(1.0, max_width) * scale).round() as i32).min(available_width);
    let height = ((height.max(1.0) * scale).round() as i32).min(available_height);
    let at = |width: i32, height: i32| {
        let x = match mode {
            Mode::Tray => area.position.x + area.size.width as i32 - width - margin,
            Mode::Overlay => area.position.x + (area.size.width as i32 - width) / 2,
        };
        let y = match mode {
            Mode::Tray => area.position.y + area.size.height as i32 - height - margin,
            Mode::Overlay => area.position.y + (f64::from(area.size.height) * 0.18).round() as i32,
        };
        PhysicalPosition::new(
            x.clamp(
                area.position.x + margin,
                (area.position.x + area.size.width as i32 - width - margin)
                    .max(area.position.x + margin),
            ),
            y.clamp(
                area.position.y + margin,
                (area.position.y + area.size.height as i32 - height - margin)
                    .max(area.position.y + margin),
            ),
        )
    };
    window.set_position(at(width, height))?;
    window.set_size(PhysicalSize::new(width as u32, height as u32))?;
    // Windows retains a hidden caption on shadowed undecorated windows. Align the actual frame.
    let outer = window.outer_size()?;
    window.set_position(at(outer.width as i32, outer.height as i32))?;
    Ok(())
}

/// The content measured itself: size the window to it, place it and show it.
#[tauri::command]
pub async fn quick_panel_fit(app: AppHandle, width: f64, height: f64) {
    if !width.is_finite() || !height.is_finite() {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    if let Err(error) = place(&app, &window, width, height) {
        tracing::warn!(%error, "cannot place the quick panel");
    }
    if !window.is_visible().unwrap_or(true) {
        SHOWN.store(true, Ordering::SeqCst);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn quick_panel_close(app: AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.close();
    }
}

/// "Open Macafon" from the panel: the main window, then the panel steps aside.
#[tauri::command]
pub async fn open_main(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !crate::show_main(&app) {
            return Err("cannot open the main window".to_owned());
        }
        quick_panel_close(app);
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_arguments_select_distinct_panel_modes() {
        assert_eq!(Mode::from_args(&[]), None);
        assert_eq!(
            Mode::from_args(&[macafon_ipc::QUICK_PANEL_ARG.into()]),
            Some(Mode::Tray)
        );
        assert_eq!(
            Mode::from_args(&[macafon_ipc::OVERLAY_ARG.into()]),
            Some(Mode::Overlay)
        );
        assert_eq!(Mode::Tray.url(), "index.html?view=quick-panel&mode=tray");
        assert_eq!(
            Mode::Overlay.url(),
            "index.html?view=quick-panel&mode=overlay"
        );
    }
}
