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
    /// Preferred PipeWire quantum for Macafon's nodes; other apps may make it smaller.
    pub quantum_frames: u32,
    pub close_action: CloseAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Hello {
    pub protocol: u32,
    pub daemon_version: String,
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
    pub latency_samples: u32,
    /// Measured by the last calibration; the Expander Sensitivity macro is relative to it.
    pub noise_floor_dbfs: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutostartMethod {
    /// `systemctl --user enable macafond.service`.
    Systemd,
    /// `$XDG_CONFIG_HOME/autostart/macafond.desktop`.
    XdgAutostart,
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
