//! Private Wayland QA harness. Run Vite on :1437 first, then run this example with optional
//! `tray` argument. Uses the browser mock, never starts/connects to the installed daemon.
#[cfg(target_os = "linux")]
mod quick_panel {
    #[derive(Clone, Copy, PartialEq)]
    pub enum Mode {
        Tray,
        Overlay,
    }
}
#[cfg(target_os = "linux")]
#[path = "../src/quick_panel_layer.rs"]
mod quick_panel_layer;
#[cfg(target_os = "linux")]
fn main() {
    use gtk::prelude::*;
    let tray = std::env::args().any(|arg| arg == "tray");
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .setup(move |app| {
            let mode = if tray { "tray" } else { "overlay" };
            let url = format!(
                "http://127.0.0.1:1437/?view=quick-panel&mode={mode}&dictation=ready&mock=1"
            );
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "layer-qa",
                tauri::WebviewUrl::External(url.parse()?),
            )
            .title("Macafon private layer-shell QA")
            .inner_size(
                if tray { 360.0 } else { 1050.0 },
                if tray { 480.0 } else { 280.0 },
            )
            .decorations(false)
            .transparent(true)
            .visible(false)
            .build()?;
            quick_panel_layer::init(&window);
            assert!(
                quick_panel_layer::place(
                    &window,
                    if tray {
                        quick_panel::Mode::Tray
                    } else {
                        quick_panel::Mode::Overlay
                    },
                    if tray { 360.0 } else { 1050.0 },
                    if tray { 480.0 } else { 280.0 },
                    false
                ),
                "layer-shell unavailable"
            );
            window.as_ref().show()?;
            window.show()?;
            let gtk = window.gtk_window()?;
            gtk.show_all();
            let handle = app.handle().clone();
            gtk.connect_key_press_event(move |_, event| {
                if event.keyval() == gtk::gdk::keys::constants::Escape {
                    handle.exit(0);
                }
                gtk::glib::Propagation::Proceed
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "../../../.artifacts/screenshots/2026-10-31-linux-panel-{mode}.png"
                ));
                let _ = std::process::Command::new("spectacle")
                    .args(["-b", "-n", "-f", "-o"])
                    .arg(path)
                    .status();
                std::thread::sleep(std::time::Duration::from_secs(8));
                handle.exit(0);
            });
            Ok(())
        })
        .run(context)
        .expect("private layer-shell QA app");
}
#[cfg(not(target_os = "linux"))]
fn main() {}
