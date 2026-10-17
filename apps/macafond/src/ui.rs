//! Show the UI (BG-004): the main window, the tray panel or the hotkey overlay. A connected UI
//! process gets a `ui.show` event and shows the window it keeps ready; only without one is
//! the UI started, next to the daemon or from PATH. On Windows the daemon also starts the UI
//! hidden at launch, so panels open at once instead of waiting for WebView2.

use crate::server::Hub;
use macafon_ipc::{Event, UiTarget};
use std::sync::{Arc, OnceLock};

/// Windows starts `macafon-ui.exe` itself: `macafon.exe` is the console CLI there.
#[cfg(windows)]
const PROGRAM: &str = "macafon-ui.exe";
/// Linux goes through the CLI, which finds the UI the same way for desktop launchers.
#[cfg(not(windows))]
const PROGRAM: &str = "macafon";

static HUB: OnceLock<Arc<Hub>> = OnceLock::new();

/// The IPC hub, once it serves clients: from then on a running UI is told, not launched.
pub fn attach(hub: Arc<Hub>) {
    let _ = HUB.set(hub);
}

pub fn open() {
    show(UiTarget::Main);
}

pub fn quick_panel() {
    show(UiTarget::TrayPanel);
}

pub fn overlay() {
    show(UiTarget::Overlay);
}

/// Start the UI without a window unless one is connected already.
#[cfg(windows)]
pub fn prewarm() {
    if !HUB.get().is_some_and(|hub| hub.ui_connected()) {
        launch(&[macafon_ipc::BACKGROUND_ARG]);
    }
}

fn show(target: UiTarget) {
    allow_foreground();
    if let Some(hub) = HUB.get().filter(|hub| hub.ui_connected()) {
        hub.broadcast(&Event::UiShow { target });
        return;
    }
    #[cfg(windows)]
    let args: &[&str] = match target {
        UiTarget::Main => &[],
        UiTarget::TrayPanel => &[macafon_ipc::QUICK_PANEL_ARG],
        UiTarget::Overlay => &[macafon_ipc::OVERLAY_ARG],
    };
    #[cfg(not(windows))]
    let args: &[&str] = match target {
        UiTarget::Main => &[],
        UiTarget::TrayPanel => &["quick-panel", "--tray"],
        UiTarget::Overlay => &["quick-panel"],
    };
    launch(args);
}

/// The hotkey or tray click gave the daemon the right to bring a window forward; pass it on,
/// or the UI could not focus what it shows.
fn allow_foreground() {
    #[cfg(windows)]
    // SAFETY: no pointers; failure only means the window may open without focus.
    let _ = unsafe {
        windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        )
    };
}

fn program() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(PROGRAM)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PROGRAM.into())
}

/// `CreateProcessW` with `STARTF_FORCEOFFFEEDBACK`: `std::process::Command` cannot turn off the
/// "app starting" cursor, which spun next to the pointer while the UI started.
#[cfg(windows)]
fn launch(args: &[&str]) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTF_FORCEOFFFEEDBACK,
        STARTUPINFOW,
    };
    use windows::core::PWSTR;
    let program = program();
    let mut line = format!("\"{}\"", program.display());
    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }
    let mut line: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_FORCEOFFFEEDBACK,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: the command line is a writable, NUL-terminated buffer that outlives the call;
    // the returned handles are closed right away, the child runs on its own.
    let started = unsafe {
        CreateProcessW(
            None,
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            None,
            &startup,
            &mut process,
        )
    };
    match started {
        Ok(()) => {
            // SAFETY: handles returned by CreateProcessW, closed once.
            unsafe {
                let _ = CloseHandle(process.hThread);
                let _ = CloseHandle(process.hProcess);
            }
        }
        Err(error) => {
            tracing::warn!(%error, program = %program.display(), "Cannot open Macafon")
        }
    }
}

#[cfg(not(windows))]
fn launch(args: &[&str]) {
    let program = program();
    match std::process::Command::new(&program).args(args).spawn() {
        // Reap the child in the background so it does not linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => {
            tracing::warn!(%error, program = %program.display(), "Cannot open Macafon")
        }
    }
}
