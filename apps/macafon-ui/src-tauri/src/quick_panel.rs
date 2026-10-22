//! The panel window: the compact panel by the tray or the centered hotkey overlay, one window
//! for both. It is created hidden when the UI starts and only shown and hidden afterwards, so
//! the daemon's `ui.show` brings it up at once; the page switches between the two layouts on
//! `panel://show`. It hides on Esc, ✕, the trigger again and when it loses focus.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
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
/// The frame the window has now: `FRAME_SYSTEM` (shadow, border, rounded corners) or
/// `FRAME_NONE` (the Sketch look's lines over the desktop, §9.9.2); unset until the first fit.
static FRAME: AtomicU8 = AtomicU8::new(FRAME_UNSET);
const FRAME_UNSET: u8 = 0;
const FRAME_SYSTEM: u8 = 1;
const FRAME_NONE: u8 = 2;

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
    .additional_browser_args(crate::BROWSER_ARGS)
    .inner_size(360.0, 420.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    // Windows 11 gives an undecorated window with a shadow its rounded corners and border;
    // the Sketch look turns both off and paints only inside its lines (`set_frame`).
    .shadow(true)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    // The page paints the panel; around the Sketch frame the desktop shows through.
    .background_color(tauri::window::Color(0, 0, 0, 0))
    .build();
    match built {
        Ok(window) => {
            // A hidden window's WebView2 keeps rendering until its controller is hidden too.
            let _ = window.as_ref().hide();
            memory_target(&window, true);
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
    if mode == Mode::Overlay
        && app
            .state::<crate::Bridge>()
            .call("prefs.get", None)
            .ok()
            .is_some_and(|prefs| prefs["overlay"]["enabled"] == false)
    {
        hide(app);
        return;
    }
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
    // Rendering resumes while the window is still hidden: the page lays itself out for
    // `mode` and answers with `quick_panel_fit`, which shows the window.
    if let Some(window) = app.get_webview_window(LABEL) {
        memory_target(&window, false);
        let _ = window.as_ref().show();
    }
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
        // Stops rendering, timers and meters of the page while nobody sees it.
        let _ = window.as_ref().hide();
        memory_target(&window, true);
    }
}

/// The warm panel spends most of its life hidden: at the low memory target WebView2 trims
/// its working set (NFR-R5 deviation), and the normal target is back before it shows again.
fn memory_target(window: &WebviewWindow, low: bool) {
    #[cfg(windows)]
    let _ = window.with_webview(move |webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
        };
        use windows::core::Interface;
        let level = if low {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        // SAFETY: COM calls on the live controller of this webview, on its UI thread; older
        // runtimes without ICoreWebView2_19 are skipped.
        unsafe {
            if let Ok(core) = webview.controller().CoreWebView2()
                && let Ok(core) = core.cast::<ICoreWebView2_19>()
            {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        }
    });
    #[cfg(not(windows))]
    let _ = (window, low);
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
    anchor: Option<(f64, f64)>,
) -> tauri::Result<()> {
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let margin = (MARGIN * scale).round() as i32;
    let available_width = (area.size.width as i32 - 2 * margin).max(1);
    let available_height = (area.size.height as i32 - 2 * margin).max(1);
    let max_width = match mode {
        Mode::Tray => 360.0,
        Mode::Overlay => 1440.0,
    };
    let width = ((width.clamp(1.0, max_width) * scale).round() as i32).min(available_width);
    let height = ((height.max(1.0) * scale).round() as i32).min(available_height);
    let at = |width: i32, height: i32| {
        let x = match mode {
            Mode::Tray => area.position.x + area.size.width as i32 - width - margin,
            // Centered on its main part, so a side panel grows the window away from it; without
            // an anchor the whole window is centered.
            Mode::Overlay => {
                let center = anchor.map_or(width / 2, |(left, right)| {
                    (((left + right) / 2.0 * scale).round() as i32).clamp(0, width)
                });
                area.position.x + area.size.width as i32 / 2 - center
            }
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
    // Windows retains a hidden caption on shadowed undecorated windows: the frame is larger
    // than the page. Place the frame where it ends up, in one move, and only when it changes:
    // moving to the page's size first and correcting after made the overlay jump on every
    // resize (opening the sounds).
    let (inner, outer) = (window.inner_size()?, window.outer_size()?);
    let extra_width = outer.width as i32 - inner.width as i32;
    let extra_height = outer.height as i32 - inner.height as i32;
    let size = PhysicalSize::new(width as u32, height as u32);
    let target = at(width + extra_width, height + extra_height);
    // A side panel opening or closing resizes the window every frame: position and size change
    // together, or a panel growing to the left shakes.
    #[cfg(windows)]
    if inner != size || window.outer_position().ok() != Some(target) {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos},
        };
        let hwnd = window.hwnd()?;
        // SAFETY: the handle belongs to this live window; SetWindowPos only moves and sizes it.
        let moved = unsafe {
            SetWindowPos(
                HWND(hwnd.0),
                None,
                target.x,
                target.y,
                width + extra_width,
                height + extra_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if moved.is_ok() {
            return Ok(());
        }
    }
    if window.outer_position().ok() != Some(target) {
        window.set_position(target)?;
    }
    if inner != size {
        window.set_size(size)?;
    }
    let outer = window.outer_size()?;
    let settled = at(outer.width as i32, outer.height as i32);
    if settled != target {
        window.set_position(settled)?;
    }
    Ok(())
}

/// The system frame (shadow, border, rounded corners on Windows 11) or none, only on change.
fn set_frame(window: &WebviewWindow, frameless: bool) {
    let wanted = if frameless { FRAME_NONE } else { FRAME_SYSTEM };
    if FRAME.swap(wanted, Ordering::SeqCst) == wanted {
        return;
    }
    if let Err(error) = window.set_shadow(!frameless) {
        tracing::warn!(%error, "cannot change the quick panel shadow");
    }
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::HWND,
            Graphics::Dwm::{
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_DONOTROUND,
                DwmSetWindowAttribute,
            },
        };
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let preference = if frameless {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_DEFAULT
        };
        // SAFETY: the handle belongs to this live window; the attribute value is a live
        // DWM_WINDOW_CORNER_PREFERENCE read during the call.
        let result = unsafe {
            DwmSetWindowAttribute(
                HWND(hwnd.0),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                std::ptr::from_ref(&preference).cast(),
                std::mem::size_of_val(&preference) as u32,
            )
        };
        if let Err(error) = result {
            tracing::debug!(%error, "cannot change the quick panel corners");
        }
    }
}

/// The page measured its layout: size and place the window, and show it if it was asked for.
/// `frameless`: the Sketch look, its lines over the desktop with no system frame.
#[tauri::command]
pub fn quick_panel_fit(
    app: AppHandle,
    width: f64,
    height: f64,
    anchor_left: Option<f64>,
    anchor_right: Option<f64>,
    frameless: Option<bool>,
) {
    if !width.is_finite() || !height.is_finite() {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    set_frame(&window, frameless.unwrap_or(false));
    let target = app.state::<PanelState>().with(|panel| {
        panel
            .as_ref()
            .map(|panel| (panel.mode, panel.monitor.clone(), panel.wanted))
    });
    // A prewarmed page measures itself too; nothing to place until something is asked for.
    let Some((mode, Some(monitor), wanted)) = target else {
        return;
    };
    let anchor = anchor_left.zip(anchor_right).filter(|(left, right)| {
        left.is_finite() && right.is_finite() && *left >= 0.0 && right > left
    });
    if let Err(error) = place(&window, mode, &monitor, width, height, anchor) {
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
