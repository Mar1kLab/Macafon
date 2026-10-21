//! Display profiles (PRD §8.5): saved monitor layouts applied from the UI, the tray, the
//! overlay, hotkeys and the CLI. Windows only (`Capabilities::displays`).

use serde::{Deserialize, Serialize};

/// What identifies a monitor across reboots, driver updates and ports (ADR-0006). A profile
/// finds its monitor by `path` first, then by `edid` + `serial`, then by an `edid` that only
/// one connected monitor has.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayIdentity {
    /// EDID manufacturer and product code, `GSM5B7F`.
    pub edid: String,
    /// The EDID serial number, when the monitor reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// The monitor's device path (`monitorDevicePath`); it changes with the port.
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz: 59940 for 59.94 Hz.
    pub refresh_mhz: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DisplayRotation {
    #[default]
    None,
    Rotate90,
    Rotate180,
    Rotate270,
}

/// Where and how an active monitor shows the desktop.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayLayout {
    /// Desktop position of the top-left corner; the primary monitor is at 0, 0.
    pub x: i32,
    pub y: i32,
    pub mode: DisplayMode,
    #[serde(default)]
    pub rotation: DisplayRotation,
    #[serde(default)]
    pub primary: bool,
    /// Duplicates another monitor: the `DisplayIdentity::path` of the one it mirrors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<String>,
    /// HDR (advanced color); `false` on monitors without it.
    #[serde(default)]
    pub hdr: bool,
}

/// A monitor as Windows reports it now; connected monitors that are off are listed too.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayMonitor {
    pub identity: DisplayIdentity,
    /// The monitor's own name (`LG ULTRAGEAR`), or its connector when the EDID has none.
    pub name: String,
    /// A laptop panel.
    pub built_in: bool,
    /// `None` while the monitor is off.
    pub layout: Option<DisplayLayout>,
    /// Modes the monitor supports, largest and fastest first.
    pub modes: Vec<DisplayMode>,
    pub hdr_supported: bool,
}

/// One monitor of a profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProfileMonitor {
    pub identity: DisplayIdentity,
    /// The name when the profile was saved, shown while the monitor is not connected.
    pub name: String,
    /// `None` turns the monitor off.
    pub layout: Option<DisplayLayout>,
}

/// A saved layout (DISP-001). Monitors connected later and not in the profile are turned off
/// when it applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayProfile {
    /// Assigned by the daemon; ignored in `displays.create`.
    pub id: String,
    pub name: String,
    /// One of `DISPLAY_PROFILE_ICONS`.
    pub icon: String,
    pub monitors: Vec<ProfileMonitor>,
    /// DISP-009: Macafon's output device while the profile is on (as `mixer.set_output`);
    /// `None` leaves the output alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
}

/// Icons a profile can have (lucide names).
pub const DISPLAY_PROFILE_ICONS: [&str; 8] = [
    "monitor",
    "monitors",
    "laptop",
    "gamepad",
    "tv",
    "film",
    "briefcase",
    "moon",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplayProfileInfo {
    pub profile: DisplayProfile,
    /// Names of its monitors that are not connected now (DISP-007); they are skipped.
    pub missing: Vec<String>,
}

/// A layout applied on a set of monitors it was never kept on waits for "Keep" (DISP-004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplaysPending {
    /// The profile id; `None` for "All monitors on".
    pub profile: Option<String>,
    /// The previous layout comes back at this moment unless kept.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub revert_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisplaysState {
    pub monitors: Vec<DisplayMonitor>,
    pub profiles: Vec<DisplayProfileInfo>,
    /// The profile the current layout matches.
    pub active: Option<String>,
    pub pending: Option<DisplaysPending>,
    /// `displays.revert` has a previous layout to go back to (DISP-005).
    pub can_revert: bool,
}
