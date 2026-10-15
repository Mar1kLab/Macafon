//! Daemon configuration: `$XDG_CONFIG_HOME/macafon/config.toml` (PRD §13).

pub mod mixer;
pub mod presets;
mod storage;

pub(crate) use storage::write_atomically;
pub use storage::{Error, Loaded, Recovered, Store, config_dir, state_dir};

pub use mixer::{AppRule, AppsConfig, ChannelConfig, MixerConfig};

use macafon_dsp::{ChainSettings, InputChannel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub mic: MicConfig,
    pub device_profiles: BTreeMap<String, MicProfile>,
    pub mixer: MixerConfig,
    pub apps: AppsConfig,
    pub ui: UiConfig,
    pub audio: AudioConfig,
    /// `[hotkeys]`: action id → shortcut, `mic_mute_toggle = "Ctrl+Alt+M"` (§8.3).
    pub hotkeys: BTreeMap<String, String>,
}

/// UI facts the daemon remembers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// BG-001: the "keeps running in the tray" notification was shown once.
    pub close_hint_shown: bool,
    /// §9.5: closing the window quits Macafon instead of keeping it in the tray.
    pub quit_on_close: bool,
}

/// `[audio]`: graph preferences (§9.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Preferred PipeWire quantum for Macafon's nodes (`node.latency`), frames at 48 kHz.
    pub quantum_frames: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            quantum_frames: DEFAULT_QUANTUM_FRAMES,
        }
    }
}

/// 128 frames = 2.7 ms at 48 kHz (PRD §9.5).
pub const DEFAULT_QUANTUM_FRAMES: u32 = 128;
/// The choices offered in Settings → Audio.
pub const QUANTUM_CHOICES: [u32; 4] = [64, 128, 256, 512];

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            mic: MicConfig::default(),
            device_profiles: BTreeMap::new(),
            mixer: MixerConfig::default(),
            apps: AppsConfig::default(),
            ui: UiConfig::default(),
            audio: AudioConfig::default(),
            hotkeys: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MicConfig {
    /// Duplicate processed mono audio into FL and FR; applied on daemon startup.
    pub stereo: bool,
    pub input_channel: InputChannel,
    /// PipeWire `node.name` of the physical microphone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Optional second-priority physical microphone by PipeWire `node.name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_device: Option<String>,
    /// Preset the chain was last loaded from (`macafon_dsp::Preset::id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Noise floor of the physical microphone measured by the last calibration (CAL-001).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_floor_dbfs: Option<f32>,
    /// Physical microphone that the persisted noise floor was measured on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_floor_device: Option<String>,
    #[serde(flatten)]
    pub chain: ChainSettings,
}

impl Default for MicConfig {
    fn default() -> Self {
        let preset = macafon_dsp::Preset::Natural;
        Self {
            stereo: false,
            input_channel: InputChannel::Auto,
            device: None,
            fallback_device: None,
            preset: Some(preset.id().to_owned()),
            noise_floor_dbfs: None,
            noise_floor_device: None,
            chain: preset.settings(),
        }
    }
}

/// The sound settings that follow one physical microphone, independent of input priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicProfile {
    pub chain: ChainSettings,
    pub preset: Option<String>,
    pub noise_floor_dbfs: Option<f32>,
    #[serde(default)]
    pub input_channel: InputChannel,
}

impl MicProfile {
    pub fn capture(mic: &MicConfig) -> Self {
        Self {
            chain: mic.chain,
            preset: mic.preset.clone(),
            noise_floor_dbfs: mic.noise_floor_dbfs,
            input_channel: mic.input_channel,
        }
    }
    pub fn apply(&self, mic: &mut MicConfig, device: &str) {
        mic.chain = self.chain;
        mic.preset = self.preset.clone();
        mic.noise_floor_dbfs = self.noise_floor_dbfs;
        mic.noise_floor_device = self.noise_floor_dbfs.map(|_| device.to_owned());
        mic.input_channel = self.input_channel;
    }
}
