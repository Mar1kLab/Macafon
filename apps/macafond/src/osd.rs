//! What a hotkey changed (HK-006). Windows: Macafon's own popup on the right edge of the active
//! monitor, drawn by the daemon so it shows at once and with the window closed; it replaces the
//! system notifications, which piled up while a volume key was held. Linux keeps a replaceable
//! desktop notification for presets and volumes: Wayland gives no way to place an overlay.

#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod popup;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Osd {
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
#[cfg_attr(not(windows), allow(dead_code))]
pub enum StatusIcon {
    Mic,
    MicOff,
    Processing,
    Preset,
}

#[cfg(windows)]
pub fn show(osd: Osd, setup: macafon_config::OsdConfig) {
    popup::show(osd, setup);
}

/// Desktop notifications have no look or place to set up.
#[cfg(not(windows))]
pub fn show(osd: Osd, _setup: macafon_config::OsdConfig) {
    match osd {
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
        Osd::Status { .. } => {}
    }
}
