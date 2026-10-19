//! State payloads shared by clients and the daemon.

use crate::{CalibrationReport, ChainSettings, InputChannel};
use serde::{Deserialize, Serialize};

/// What the window's close button does (§9.5, BG-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    /// The window goes away, sound keeps going in the daemon.
    Tray,
    /// Closing the window quits Macafon like the Quit menu item (BG-005).
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Preferences {
    pub volume_step_pct: u8,
    /// Preferred PipeWire quantum for Macafon's nodes; other apps may make it smaller.
    pub quantum_frames: u32,
    pub close_action: CloseAction,
    /// The hotkey popup; applies where `Capabilities::hotkey_popup` is set.
    #[serde(default)]
    pub osd: OsdPrefs,
    #[serde(default)]
    pub overlay: OverlayPrefs,
}

/// The hotkey overlay (§9.9): visible controls, expansion direction and placement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct OverlayPrefs {
    pub volumes: bool,
    pub mic_switch: bool,
    pub processing_switch: bool,
    pub mixer_switch: bool,
    pub preset: bool,
    pub output_device: bool,
    pub mic_level: bool,
    pub expand: OverlayExpand,
    pub placement: OverlayPlacement,
}
impl Default for OverlayPrefs {
    fn default() -> Self {
        Self {
            volumes: true,
            mic_switch: true,
            processing_switch: true,
            mixer_switch: true,
            preset: true,
            output_device: false,
            mic_level: false,
            expand: OverlayExpand::Right,
            placement: OverlayPlacement::Anchored,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OverlayExpand {
    #[default]
    Right,
    Left,
    Down,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OverlayPlacement {
    #[default]
    Anchored,
    Centered,
}

/// The hotkey popup (HK-006): its look, the edge it sits at and how far from it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OsdPrefs {
    #[serde(default)]
    pub look: OsdLook,
    #[serde(default)]
    pub primary_monitor: bool,
    pub style: OsdStyle,
    /// `left`/`right` for `vertical`, `top`/`bottom` for `horizontal`.
    pub edge: OsdEdge,
    /// Percent of the screen from the edge toward the center, 0…`OSD_MAX_OFFSET_PCT`.
    pub offset_pct: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OsdLook {
    #[default]
    Classic,
    Sketch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OsdStyle {
    /// A vertical slider at a side, like Android.
    #[default]
    Vertical,
    /// A horizontal bar at the top or bottom, like Windows 11.
    Horizontal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OsdEdge {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Hello {
    pub protocol: u32,
    pub daemon_version: String,
    #[serde(default)]
    pub platform: Platform,
    /// What this platform can do; the UI hides the rest by these flags, not by the OS (§19.1).
    #[serde(default)]
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    #[default]
    Linux,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Capabilities {
    /// The daemon creates Macafon Microphone itself; on Windows it is the user-installed
    /// VB-CABLE (WIN-MIC-001…003).
    pub virtual_mic_device: bool,
    /// Mono or stereo output of the virtual microphone (MIC-011); the cable is always stereo.
    pub stereo_option: bool,
    /// EQ and effects on channels and Master (CFX-001…007, WIN-CH-006).
    pub channel_fx: bool,
    /// The preferred graph quantum in Settings applies (PipeWire).
    pub graph_quantum: bool,
    /// Hotkeys show Macafon's own popup, set up in Settings (HK-006); otherwise a desktop
    /// notification.
    #[serde(default)]
    pub hotkey_popup: bool,
    /// Sounds play into the virtual microphone and the headphones (§7.6).
    #[serde(default)]
    pub soundpad: bool,
    /// Local speech to text (§8.4).
    #[serde(default)]
    pub dictation: bool,
}

impl Capabilities {
    pub const LINUX: Self = Self {
        virtual_mic_device: true,
        stereo_option: true,
        channel_fx: true,
        graph_quantum: true,
        hotkey_popup: false,
        soundpad: false,
        dictation: false,
    };
    pub const WINDOWS: Self = Self {
        virtual_mic_device: false,
        stereo_option: false,
        channel_fx: false,
        graph_quantum: false,
        hotkey_popup: true,
        soundpad: true,
        dictation: true,
    };
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::LINUX
    }
}

impl Platform {
    /// The platform this binary was built for.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub const fn capabilities(self) -> Capabilities {
        match self {
            Self::Linux => Capabilities::LINUX,
            Self::Windows => Capabilities::WINDOWS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MicState {
    /// Virtual microphone output uses identical FL/FR channels instead of MONO.
    #[serde(default)]
    pub stereo: bool,
    #[serde(default, skip_serializing_if = "input_channel_is_auto")]
    pub input_channel: InputChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_recovery: Option<ConfigRecovery>,
    pub settings: ChainSettings,
    /// Preset the chain was last loaded from or saved to.
    pub preset: Option<String>,
    /// The chain differs from that preset (UI: "modified", enables Save).
    pub preset_modified: bool,
    pub device: Option<String>,
    #[serde(default)]
    pub fallback_device: Option<String>,
    /// Microphone currently linked; `null` while disconnected.
    #[serde(default)]
    pub active_device: Option<String>,
    /// The selected microphone is present and linked (MIC-005).
    pub input_connected: bool,
    /// Macafon Microphone is the system default input now (MIC-012).
    #[serde(default)]
    pub is_default_source: bool,
    /// Processed sound reaches an app-visible microphone. On Windows `false` until the
    /// VB-CABLE is installed (WIN-MIC-001); meters, calibration and A/B work either way.
    #[serde(default = "yes")]
    pub virtual_mic_ready: bool,
    pub latency_samples: u32,
    /// Measured by the last calibration; the Expander Sensitivity macro is relative to it.
    pub noise_floor_dbfs: Option<f32>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutostartMethod {
    /// `systemctl --user enable macafond.service`.
    Systemd,
    /// `$XDG_CONFIG_HOME/autostart/macafond.desktop`.
    XdgAutostart,
    /// Per-user `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value.
    WindowsRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutostartState {
    pub enabled: bool,
    /// What enabling uses on this system.
    pub method: AutostartMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AbSide {
    /// Raw microphone.
    A,
    /// Through the current chain.
    B,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AbRecording {
    pub elapsed_ms: u32,
    pub max_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AbState {
    /// A recording in progress.
    pub recording: Option<AbRecording>,
    /// Length of the recorded clip, kept in memory only (AB-007).
    pub clip_ms: Option<u32>,
    pub playing: bool,
    pub side: AbSide,
    pub position_ms: u32,
    pub loudness_match: bool,
    /// Listening sink `node.name`; `None` is the default output.
    pub device: Option<String>,
}

/// Peak envelopes, 0…1 (linear), aligned in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AbWaveform {
    pub a: Vec<f32>,
    pub b: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CalibrationStep {
    /// CAL-001: "Stay silent", 4 s.
    Silence,
    /// CAL-002: "Read this sentence", 6 s.
    Speech,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CalibrationFailure {
    /// CAL-005: the input clipped; lower the system or interface level and retry.
    Clipped,
    NoSpeech,
    /// The microphone disconnected or PipeWire restarted.
    Disconnected,
    /// No audio arrived from the microphone.
    Stalled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CalibrationState {
    Idle,
    Measuring {
        step: CalibrationStep,
        /// 0…1 within the step.
        progress: f32,
        step_duration_ms: u32,
    },
    /// The calibrated settings are already applied (see `mic.changed`).
    Done {
        report: CalibrationReport,
    },
    Failed {
        reason: CalibrationFailure,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PresetInfo {
    /// Built-in: `natural`, …; user: `user:<slug>`.
    pub id: String,
    pub name: String,
    /// Built-in presets are read-only.
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Device {
    /// PipeWire `node.name`, stable across reconnects.
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stable_id: String,
    #[serde(default, skip_serializing_if = "device_kind_is_unknown")]
    pub kind: DeviceKind,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Usb,
    Builtin,
    Headset,
    Bluetooth,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PlaybackDeviceKind {
    Headphones,
    Speakers,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlaybackDevice {
    pub name: String,
    pub description: String,
    pub kind: PlaybackDeviceKind,
    pub speaker_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConfigRecovery {
    pub backup: String,
    pub reason: String,
}

fn input_channel_is_auto(channel: &InputChannel) -> bool {
    *channel == InputChannel::Auto
}
fn device_kind_is_unknown(kind: &DeviceKind) -> bool {
    *kind == DeviceKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_osd_preferences_default_to_classic() {
        let prefs: OsdPrefs = serde_json::from_value(serde_json::json!({
            "style": "horizontal", "edge": "bottom", "offset_pct": 10.0
        }))
        .expect("old preferences");
        assert_eq!(prefs.look, OsdLook::Classic);
        let sketch = OsdPrefs {
            look: OsdLook::Sketch,
            ..prefs
        };
        let json = serde_json::to_value(sketch).expect("encode");
        assert_eq!(json["look"], "sketch");
        assert_eq!(
            serde_json::from_value::<OsdPrefs>(json).expect("decode"),
            sketch
        );
    }
}
