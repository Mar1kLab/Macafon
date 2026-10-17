//! The quick panel: every volume and the microphone switches in a small window above the
//! taskbar, like the Windows 11 flyouts. `macafon-ui --quick-panel` opens it (the daemon does
//! on its hotkey); a running UI gets the argument from the second launch and toggles it.
//! It closes on Esc, on the hotkey again and when it loses focus; with no main window open
//! the process ends with it.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "quick-panel";
/// Logical width; the height follows the content (`quick_panel_fit`).
const WIDTH: f64 = 360.0;
/// Room between the panel and the screen edges or the taskbar, logical pixels.
const MARGIN: f64 = 12.0;

/// Set when the panel is shown: a hidden window being created may gain and lose focus, which
/// must not close it.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Set once the shown panel had focus: only then does losing it close the panel.
static FOCUSED: AtomicBool = AtomicBool::new(false);

pub fn toggle(app: &AppHandle) {
    match app.get_webview_window(LABEL) {
        Some(window) => {
            let _ = window.close();
        }
        None => open(app),
    }
}

fn open(app: &AppHandle) {
    SHOWN.store(false, Ordering::SeqCst);
    FOCUSED.store(false, Ordering::SeqCst);
    let built = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?view=quick-panel".into()),
    )
    .title("Macafon quick panel")
    .inner_size(WIDTH, 420.0)
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
                    } else if FOCUSED.load(Ordering::SeqCst) {
                        let _ = handle.close();
                    }
                }
            });
        }
        Err(error) => tracing::warn!(%error, "cannot open the quick panel"),
    }
}

/// Bottom-right of the work area of the monitor under the cursor, where the tray is. Moved
/// first and sized after in that monitor's pixels: a size set before the move would be
/// scaled again when the window crosses to a monitor with another DPI.
fn place(app: &AppHandle, window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let monitor = match app.cursor_position() {
        Ok(cursor) => app.monitor_from_point(cursor.x, cursor.y)?,
        Err(_) => None,
    };
    let Some(monitor) = monitor.or(app.primary_monitor()?) else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let width = (WIDTH * scale).round() as i32;
    let height = (height * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let right = area.position.x + area.size.width as i32;
    let bottom = area.position.y + area.size.height as i32;
    let at = |height: i32| {
        PhysicalPosition::new(
            (right - width - margin).max(area.position.x),
            (bottom - height - margin).max(area.position.y),
        )
    };
    window.set_position(at(height))?;
    window.set_size(PhysicalSize::new(width as u32, height as u32))?;
    // The frame may be taller than the content asked for (Windows keeps a hidden caption on
    // undecorated windows): line its real bottom up with the work area.
    let outer = i32::try_from(window.outer_size()?.height).unwrap_or(height);
    if outer > height {
        window.set_position(at(outer))?;
    }
    Ok(())
}

/// The content measured itself: size the window to it, place it and show it.
#[tauri::command]
pub fn quick_panel_fit(app: AppHandle, height: f64) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let height = height.clamp(120.0, 900.0);
    if let Err(error) = place(&app, &window, height) {
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
pub fn open_main(app: AppHandle) {
    crate::show_main(&app);
    quick_panel_close(app);
}
