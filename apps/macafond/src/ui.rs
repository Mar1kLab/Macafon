//! Start the UI next to the daemon, else from PATH (BG-004): the main window, or just the
//! quick panel, which a running UI toggles instead.

/// Windows starts `macafon-ui.exe` itself: `macafon.exe` is the console CLI there.
#[cfg(windows)]
const PROGRAM: &str = "macafon-ui.exe";
/// Linux goes through the CLI, which finds the UI the same way for desktop launchers.
#[cfg(not(windows))]
const PROGRAM: &str = "macafon";

pub fn open() {
    launch(&[]);
}

pub fn quick_panel() {
    #[cfg(windows)]
    launch(&[macafon_ipc::QUICK_PANEL_ARG]);
    #[cfg(not(windows))]
    launch(&["quick-panel", "--tray"]);
}

pub fn overlay() {
    #[cfg(windows)]
    launch(&[macafon_ipc::OVERLAY_ARG]);
    #[cfg(not(windows))]
    launch(&["quick-panel"]);
}

fn launch(args: &[&str]) {
    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(PROGRAM)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PROGRAM.into());
    let mut command = std::process::Command::new(&program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up next to the window.
        command.creation_flags(0x08000000);
        // The hotkey or tray click gave the daemon the right to bring a window forward; pass
        // it on, or an already running UI could not focus the panel it opens.
        // SAFETY: no pointers; failure only means the window may open without focus.
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
            )
        };
    }
    match command.spawn() {
        // Reap the child in the background so it does not linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => {
            tracing::warn!(%error, program = %program.display(), "Cannot open Macafon")
        }
    }
}
