//! Optional GTK layer-shell integration. Loading at runtime keeps X11/GNOME installations
//! usable without the library; distributions should ship libgtk-layer-shell0 for Wayland.
use super::quick_panel::Mode;
use gtk::{glib::translate::ToGlibPtr, prelude::*};
use std::{ffi::c_void, sync::OnceLock};
use tauri::WebviewWindow;
type Window = *mut gtk::ffi::GtkWindow;
struct Api {
    _library: libloading::Library,
    supported: unsafe extern "C" fn() -> i32,
    init: unsafe extern "C" fn(Window),
    is_layer: unsafe extern "C" fn(Window) -> i32,
    layer: unsafe extern "C" fn(Window, u32),
    keyboard: unsafe extern "C" fn(Window, u32),
    anchor: unsafe extern "C" fn(Window, u32, i32),
    margin: unsafe extern "C" fn(Window, u32, i32),
    monitor: unsafe extern "C" fn(Window, *mut c_void),
    namespace: unsafe extern "C" fn(Window, *const std::ffi::c_char),
}
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        // SAFETY: fixed library ABI and symbol signatures from gtk-layer-shell's public C
        // header. Keep the library alive for every copied function pointer's lifetime.
        unsafe {
            let lib = libloading::Library::new("libgtk-layer-shell.so.0").ok()?;
            Some(Api {
                supported: *lib.get(b"gtk_layer_is_supported\0").ok()?,
                init: *lib.get(b"gtk_layer_init_for_window\0").ok()?,
                is_layer: *lib.get(b"gtk_layer_is_layer_window\0").ok()?,
                layer: *lib.get(b"gtk_layer_set_layer\0").ok()?,
                keyboard: *lib.get(b"gtk_layer_set_keyboard_mode\0").ok()?,
                anchor: *lib.get(b"gtk_layer_set_anchor\0").ok()?,
                margin: *lib.get(b"gtk_layer_set_margin\0").ok()?,
                monitor: *lib.get(b"gtk_layer_set_monitor\0").ok()?,
                namespace: *lib.get(b"gtk_layer_set_namespace\0").ok()?,
                _library: lib,
            })
        }
    })
    .as_ref()
}
pub(super) fn init(window: &WebviewWindow) {
    let Some(api) = api() else {
        tracing::debug!("gtk-layer-shell unavailable; using normal quick panel window");
        return;
    };
    let Ok(gtk) = window.gtk_window() else {
        return;
    };
    // SAFETY: called on GTK's main thread with a live, initially hidden GTK window. The
    // capability query prevents layer-shell calls on X11 or unsupported compositors.
    unsafe {
        if (api.supported)() == 0 {
            return;
        }
        if gtk.is_realized() {
            tracing::warn!("quick panel already realized; cannot initialize layer shell");
            return;
        }
        let ptr = gtk.upcast_ref::<gtk::Window>().to_glib_none().0;
        (api.init)(ptr);
        (api.layer)(ptr, 3); // OVERLAY
        (api.keyboard)(ptr, 2); // ON_DEMAND: Esc and SoundDeck search, without exclusive input
        (api.namespace)(ptr, c"macafon-quick-panel".as_ptr());
    }
}
pub(super) fn place(
    window: &WebviewWindow,
    mode: Mode,
    width: f64,
    height: f64,
    primary: bool,
) -> bool {
    let Some(api) = api() else {
        return false;
    };
    let Ok(gtk) = window.gtk_window() else {
        return false;
    };
    // SAFETY: the application owns this GTK window; monitor references live through calls.
    unsafe {
        let ptr = gtk.upcast_ref::<gtk::Window>().to_glib_none().0;
        if (api.is_layer)(ptr) == 0 {
            return false;
        }
        let display = gtk.display();
        let monitor = if primary {
            display.primary_monitor().or_else(|| display.monitor(0))
        } else {
            None
        };
        (api.monitor)(
            ptr,
            monitor
                .as_ref()
                .map_or(std::ptr::null_mut(), |m| m.as_ptr() as *mut c_void),
        );
        // C enum: LEFT=0, RIGHT=1, TOP=2, BOTTOM=3. Unanchored axes are centered.
        for edge in 0..4 {
            (api.anchor)(
                ptr,
                edge,
                i32::from(match mode {
                    Mode::Overlay => edge == 2,
                    Mode::Tray => edge == 1 || edge == 3,
                }),
            );
            (api.margin)(ptr, edge, 12);
        }
        if mode == Mode::Overlay {
            (api.margin)(
                ptr,
                2,
                monitor
                    .as_ref()
                    .map_or(48, |m| (m.geometry().height() as f64 * 0.18) as i32),
            );
        }
        gtk.resize(
            width.clamp(1.0, 1440.0).round() as i32,
            height.max(1.0).round() as i32,
        );
    }
    true
}
