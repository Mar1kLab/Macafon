//! The panel window: the compact panel by the tray or the centered hotkey overlay, one window
//! for both. It is created hidden when the UI starts and only shown and hidden afterwards, so
//! the daemon's `ui.show` brings it up at once; the page switches between the two layouts on
//! `panel://show`. It hides on Esc, ✕, the trigger again and when it loses focus.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "quick-panel";
const MARGIN: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
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
}

/// What the panel shows and where; `wanted` until `quick_panel_fit` has shown it.
struct Panel {
    mode: Mode,
    monitor: Option<Monitor>,
    wanted: bool,
}

#[derive(Default)]
pub struct PanelState(Mutex<Option<Panel>>);

impl PanelState {
    fn with<T>(&self, f: impl FnOnce(&mut Option<Panel>) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

/// Set once the shown panel had focus: only then does losing it hide the panel. A hidden
/// window being shown can gain and lose focus on the way.
static FOCUSED: AtomicBool = AtomicBool::new(false);
/// When the panel last hid because focus left it. A tray click takes the focus first, then
/// asks for the panel: that click means "close", not "open again".
static BLURRED_AT: Mutex<Option<(Mode, Instant)>> = Mutex::new(None);
const BLUR_GRACE: Duration = Duration::from_millis(400);

/// Create the hidden panel window if it is missing; the page loads and connects meanwhile.
pub fn prewarm(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_some() {
        return;
    }
    let built = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?view=quick-panel".into()),
    )
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
    .visible(false)
    .background_color(tauri::window::Color(0x10, 0x10, 0x14, 0xff)) // --bg-raised
    .build();
    match built {
        Ok(window) => {
            let handle = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(focused) = event {
                    if *focused {
                        if handle.is_visible().unwrap_or(false) {
                            FOCUSED.store(true, Ordering::SeqCst);
                        }
                    } else if FOCUSED.swap(false, Ordering::SeqCst) {
                        let app = handle.app_handle();
                        let mode = app
                            .state::<PanelState>()
                            .with(|panel| panel.as_ref().map(|panel| panel.mode));
                        if let Some(mode) = mode {
                            *BLURRED_AT.lock().unwrap_or_else(|error| error.into_inner()) =
                                Some((mode, Instant::now()));
                        }
                        hide(app);
                    }
                }
            });
        }
        Err(error) => tracing::warn!(%error, "cannot create the quick panel"),
    }
}

/// The same mode again hides the panel; otherwise it shows in `mode`.
pub fn toggle(app: &AppHandle, mode: Mode) {
    let visible = app
        .get_webview_window(LABEL)
        .is_some_and(|window| window.is_visible().unwrap_or(false));
    let state = app.state::<PanelState>();
    let current = state.with(|panel| panel.as_ref().map(|panel| panel.mode));
    if visible && current == Some(mode) {
        hide(app);
        return;
    }
    let just_blurred = BLURRED_AT
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .is_some_and(|(blurred, at)| blurred == mode && at.elapsed() < BLUR_GRACE);
    if !visible && just_blurred {
        return;
    }
    let monitor = target_monitor(app, mode);
    state.with(|panel| {
        *panel = Some(Panel {
            mode,
            monitor,
            wanted: true,
        })
    });
    prewarm(app);
    // The page lays itself out for `mode` and answers with `quick_panel_fit`, which shows it.
    if let Err(error) = app.emit_to(LABEL, "panel://show", mode) {
        tracing::warn!(%error, "cannot show the quick panel");
    }
}

pub fn hide(app: &AppHandle) {
    app.state::<PanelState>().with(|panel| {
        if let Some(panel) = panel {
            panel.wanted = false;
        }
    });
    FOCUSED.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

/// Capture the foreground monitor before the panel takes focus.
fn target_monitor(app: &AppHandle, mode: Mode) -> Option<Monitor> {
    if mode == Mode::Overlay
        && app
            .state::<crate::Bridge>()
            .call("prefs.get", None)
            .ok()
            .is_some_and(|prefs| prefs["osd"]["primary_monitor"] == true)
    {
        return app.primary_monitor().ok().flatten();
    }
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

/// Move first, then size in the destination monitor's pixels to avoid double DPI scaling.
fn place(
    window: &WebviewWindow,
    mode: Mode,
    monitor: &Monitor,
    width: f64,
    height: f64,
) -> tauri::Result<()> {
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

/// The page measured its layout: size and place the window, and show it if it was asked for.
#[tauri::command]
pub fn quick_panel_fit(app: AppHandle, width: f64, height: f64) {
    if !width.is_finite() || !height.is_finite() {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let target = app.state::<PanelState>().with(|panel| {
        panel
            .as_ref()
            .map(|panel| (panel.mode, panel.monitor.clone(), panel.wanted))
    });
    // A prewarmed page measures itself too; nothing to place until something is asked for.
    let Some((mode, Some(monitor), wanted)) = target else {
        return;
    };
    if let Err(error) = place(&window, mode, &monitor, width, height) {
        tracing::warn!(%error, "cannot place the quick panel");
    }
    if wanted && !window.is_visible().unwrap_or(true) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// A page that just loaded asks what it was opened for: an event sent before it listened is
/// lost.
#[tauri::command]
pub fn quick_panel_pending(app: AppHandle) -> Option<Mode> {
    app.state::<PanelState>().with(|panel| {
        panel
            .as_ref()
            .filter(|panel| panel.wanted)
            .map(|panel| panel.mode)
    })
}

#[tauri::command]
pub fn quick_panel_close(app: AppHandle) {
    hide(&app);
}

/// "Open Macafon" from the panel: the main window, then the panel steps aside. Async, so the
/// main window is not created on the event thread (that deadlocked WebView2: black window).
#[tauri::command]
pub async fn open_main(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        hide(&app);
        if crate::show_main(&app) {
            Ok(())
        } else {
            Err("cannot open the main window".to_owned())
        }
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
        assert_eq!(Mode::from_args(&[macafon_ipc::BACKGROUND_ARG.into()]), None);
    }
}
