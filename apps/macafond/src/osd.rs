//! What a hotkey changed (HK-006) and the dictation popup (§9.9.3), drawn by the daemon so they
//! show at once and with the window closed. Windows: Macafon's own popup (Direct2D in a layered
//! window); it replaces the system notifications, which piled up while a volume key was held.
//! Linux: the same popup in a wlr-layer-shell overlay, a replaceable notification without one.

pub mod dictation;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(windows, target_os = "linux"))]
mod motion;

#[cfg(windows)]
mod fonts;
#[cfg(any(windows, target_os = "linux"))]
mod icons;
#[cfg(windows)]
mod popup;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub enum Osd {
    Dictation(dictation::View),
    /// A channel volume or mute: a vertical slider in the channel's color, icon and name.
    Volume {
        name: String,
        /// One of `CHANNEL_COLORS`.
        color: String,
        /// One of `CHANNEL_ICONS`.
        icon: String,
        volume_pct: f32,
        muted: bool,
    },
    /// Any other action: what changed and its new state; `alert` marks an "off" state.
    Status {
        icon: StatusIcon,
        title: String,
        value: String,
        alert: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub enum StatusIcon {
    Mic,
    MicOff,
    Processing,
    Preset,
    Monitor,
}

#[cfg(windows)]
pub fn show(osd: Osd, setup: macafon_config::OsdConfig) {
    popup::show(osd, setup);
}

/// Linux: the same popup in a layer-shell overlay; where the compositor has none (GNOME,
/// X11) a replaceable desktop notification for the volumes, presets and display profiles.
#[cfg(not(windows))]
pub fn show(osd: Osd, setup: macafon_config::OsdConfig) {
    #[cfg(target_os = "linux")]
    let Some(osd) = linux::show(osd, setup) else {
        return;
    };
    #[cfg(not(target_os = "linux"))]
    let _ = setup;
    notification(osd);
}

#[cfg(not(windows))]
fn notification(osd: Osd) {
    match osd {
        Osd::Dictation(view) => {
            #[cfg(target_os = "linux")]
            crate::notify::dictation(&view);
            #[cfg(not(target_os = "linux"))]
            let _ = view;
        }
        Osd::Volume {
            name, muted: true, ..
        } => crate::notify::osd(format!("{name}: muted")),
        Osd::Volume {
            name, volume_pct, ..
        } => crate::notify::osd(format!("{name}: {volume_pct:.0}%")),
        Osd::Status {
            icon: StatusIcon::Preset,
            value,
            ..
        } => crate::notify::osd(format!("Preset: {value}")),
        Osd::Status {
            icon: StatusIcon::Monitor,
            value,
            ..
        } => crate::notify::osd(format!("Display: {value}")),
        // Push-to-talk toggles these on every press: a notification each time is in the way.
        Osd::Status { .. } => {}
    }
}

static DICTATION_CONTROL: std::sync::OnceLock<
    macafon_backend::ControlSender<crate::host::ControlMessage>,
> = std::sync::OnceLock::new();
pub fn set_dictation_control(control: macafon_backend::ControlSender<crate::host::ControlMessage>) {
    let _ = DICTATION_CONTROL.set(control);
}
#[cfg(any(windows, target_os = "linux"))]
pub fn dictation_click(cancel: bool) {
    if let Some(control) = DICTATION_CONTROL.get() {
        let (reply, _) = std::sync::mpsc::channel();
        let _ = control.send(crate::host::ControlMessage::Request {
            request: if cancel {
                macafon_ipc::Request::DictationCancel
            } else {
                macafon_ipc::Request::DictationTestStop
            },
            reply,
        });
    }
}

#[cfg(windows)]
pub fn clipboard_owner() -> Option<windows::Win32::Foundation::HWND> {
    popup::clipboard_owner()
}

/// Whether the hotkey popup and its Settings apply here (`Capabilities::hotkey_popup`).
#[cfg(target_os = "linux")]
pub fn popup_supported() -> bool {
    linux::supported()
}
