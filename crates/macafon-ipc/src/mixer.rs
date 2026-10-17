//! Mixer part of the protocol (PRD §7, §9.4): channels, applications, Master and output.

use crate::PlaybackDeviceKind;
use macafon_dsp::{ChannelMeters, ChannelSettings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChannelState {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    /// Off: no node, its apps play through the default channel.
    pub enabled: bool,
    pub settings: ChannelSettings,
    pub latency_samples: u32,
}

/// One application chip (APP-002/003): all its playback streams together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppState {
    /// `bin:discord`, `app:com.spotify.Client`, `name:…`, `system:sounds`.
    pub key: String,
    pub name: String,
    /// Freedesktop icon name, if known.
    pub icon: Option<String>,
    pub channel: String,
    /// Playing right now (APP-003).
    pub active: bool,
    /// Live streams; 0 for a remembered app that is not running (APP-007).
    pub streams: u32,
    /// `false`: "Don't manage this app" (APP-009).
    pub managed: bool,
    /// The app forbids moving its streams (`node.dont-move`).
    pub movable: bool,
    /// CH-008: the app's volume inside its channel, 0–100 %.
    pub volume_pct: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutputDevice {
    pub name: String,
    pub description: String,
    pub kind: PlaybackDeviceKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MixerState {
    pub enabled: bool,
    pub default_channel: String,
    pub master_volume_pct: f32,
    pub master_muted: bool,
    pub auto_classify: bool,
    pub channels: Vec<ChannelState>,
    pub apps: Vec<AppState>,
    /// Real output devices, Macafon's own channels excluded (OUT-004).
    pub outputs: Vec<OutputDevice>,
    /// OUT-002 priority list by `node.name`.
    pub output_priority: Vec<String>,
    /// The device the channels play into right now.
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChannelMeterFrame {
    pub id: String,
    pub meters: ChannelMeters,
}

/// Post-fader peaks of every channel and of the Master (CH-005, CH-006).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MixerMeters {
    pub channels: Vec<ChannelMeterFrame>,
    /// The loudest channel after the Master fader, per side.
    pub master_left_dbfs: f32,
    pub master_right_dbfs: f32,
}

/// Partial channel change: absent fields stay.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChannelPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Switch the channel on or off; the default channel stays on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}
