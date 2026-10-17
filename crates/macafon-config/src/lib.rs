//! Daemon configuration: `$XDG_CONFIG_HOME/macafon/config.toml` (PRD §13).

pub mod mixer;
pub mod presets;
pub mod soundpad;
mod storage;

pub(crate) use storage::write_atomically;
pub use storage::{Error, Loaded, Recovered, Store, config_dir, state_dir};

pub use mixer::{AppRule, AppsConfig, ChannelConfig, MixerConfig};
pub use soundpad::{SoundConfig, SoundpadConfig};

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
    pub soundpad: SoundpadConfig,
}

/// UI facts the daemon remembers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// BG-001: the "keeps running in the tray" notification was shown once.
    pub close_hint_shown: bool,
    /// §9.5: closing the window quits Macafon instead of keeping it in the tray.
    pub quit_on_close: bool,
    /// `[ui.osd]`: the hotkey popup (HK-006).
    pub osd: OsdConfig,
    /// The `DEFAULT_HOTKEYS` revision already offered, so a shortcut the user cleared stays
    /// cleared.
    pub hotkey_defaults: u32,
    #[serde(default = "default_volume_step")]
    pub volume_step_pct: u8,
}

pub fn default_volume_step() -> u8 {
    5
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            close_hint_shown: false,
            quit_on_close: false,
            osd: OsdConfig::default(),
            hotkey_defaults: 0,
            volume_step_pct: default_volume_step(),
        }
    }
}

/// Default shortcuts for one hand on the left of the keyboard: Ctrl+Alt (pinky and thumb) plus
/// a key under the other fingers. The number row's left and the letters below it work like
/// faders: `W`/`S` Main, `E`/`D` Voice, `R`/`F` Music, up on top. `Q` mutes the microphone and
/// `` ` `` (the console key) opens the quick panel. None of them is a Windows, GNOME or KDE
/// default or a common overlay key (Game Bar, GeForce, Steam, Discord, AMD). Each group is
/// added whole or not at all.
pub const DEFAULT_HOTKEYS: [&[(&str, &str)]; 5] = [
    &[("quick_panel", "Ctrl+Alt+Backquote")],
    &[("mic_mute_toggle", "Ctrl+Alt+Q")],
    &[
        ("channel_volume_up.main", "Ctrl+Alt+W"),
        ("channel_volume_down.main", "Ctrl+Alt+S"),
    ],
    &[
        ("channel_volume_up.voice", "Ctrl+Alt+E"),
        ("channel_volume_down.voice", "Ctrl+Alt+D"),
    ],
    &[
        ("channel_volume_up.music", "Ctrl+Alt+R"),
        ("channel_volume_down.music", "Ctrl+Alt+F"),
    ],
];
/// Raised when `DEFAULT_HOTKEYS` gains shortcuts existing configs should get.
pub const HOTKEY_DEFAULTS_REVISION: u32 = 1;

/// How the hotkey popup looks and where it sits (HK-006, Windows).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdConfig {
    pub primary_monitor: bool,
    pub style: OsdStyle,
    /// Left or right for the vertical style, top or bottom for the horizontal one.
    pub edge: OsdEdge,
    /// Distance from that edge in percent of the screen, up to `OSD_MAX_OFFSET_PCT`.
    pub offset_pct: f32,
}

/// The farthest the popup moves from its edge toward the center.
pub const OSD_MAX_OFFSET_PCT: f32 = 40.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdStyle {
    /// A vertical slider at a side, like Android.
    #[default]
    Vertical,
    /// A horizontal bar at the top or bottom, like Windows 11.
    Horizontal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdEdge {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

impl OsdConfig {
    /// The edge the style can use: a mismatched one falls back to the style's default, a
    /// broken offset to 0.
    pub fn normalized(self) -> Self {
        let edge = match (self.style, self.edge) {
            (OsdStyle::Vertical, OsdEdge::Left | OsdEdge::Right)
            | (OsdStyle::Horizontal, OsdEdge::Top | OsdEdge::Bottom) => self.edge,
            (OsdStyle::Vertical, _) => OsdEdge::Right,
            (OsdStyle::Horizontal, _) => OsdEdge::Bottom,
        };
        let offset_pct = if self.offset_pct.is_finite() {
            self.offset_pct.clamp(0.0, OSD_MAX_OFFSET_PCT)
        } else {
            0.0
        };
        Self {
            primary_monitor: self.primary_monitor,
            style: self.style,
            edge,
            offset_pct,
        }
    }
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

impl Config {
    /// Offer `DEFAULT_HOTKEYS` once per revision: a group is added only if none of its
    /// actions is bound and none of its shortcuts is taken, and `usable` accepts them all (on
    /// Windows a Ctrl+Alt shortcut must not swallow an AltGr character). Returns whether the
    /// config changed.
    pub fn offer_default_hotkeys(&mut self, usable: impl Fn(&str) -> bool) -> bool {
        if self.ui.hotkey_defaults >= HOTKEY_DEFAULTS_REVISION {
            return false;
        }
        for group in DEFAULT_HOTKEYS {
            let free = group.iter().all(|(action, shortcut)| {
                !self.hotkeys.contains_key(*action)
                    && !self.hotkeys.values().any(|bound| bound == shortcut)
                    && usable(shortcut)
            });
            if free {
                for (action, shortcut) in group {
                    self.hotkeys
                        .insert((*action).to_owned(), (*shortcut).to_owned());
                }
            }
        }
        self.ui.hotkey_defaults = HOTKEY_DEFAULTS_REVISION;
        true
    }
}

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
            soundpad: SoundpadConfig::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkeys_are_offered_once_and_never_take_a_used_key() {
        let mut config = Config::default();
        config
            .hotkeys
            .insert("bypass_toggle".into(), "Ctrl+Alt+W".into());
        assert!(config.offer_default_hotkeys(|shortcut| shortcut != "Ctrl+Alt+E"));
        assert_eq!(config.hotkeys["quick_panel"], "Ctrl+Alt+Backquote");
        assert_eq!(config.hotkeys["mic_mute_toggle"], "Ctrl+Alt+Q");
        assert_eq!(config.hotkeys["channel_volume_down.music"], "Ctrl+Alt+F");
        // Main's up key is taken and Voice's would type a character: both pairs stay unset.
        assert!(!config.hotkeys.contains_key("channel_volume_down.main"));
        assert!(!config.hotkeys.contains_key("channel_volume_down.voice"));
        config.hotkeys.remove("mic_mute_toggle");
        assert!(!config.offer_default_hotkeys(|_| true));
        assert!(!config.hotkeys.contains_key("mic_mute_toggle"));
    }

    #[test]
    fn osd_setup_keeps_an_edge_of_its_style_and_a_sane_offset() {
        let fixed = OsdConfig {
            primary_monitor: false,
            style: OsdStyle::Horizontal,
            edge: OsdEdge::Right,
            offset_pct: 90.0,
        }
        .normalized();
        assert_eq!(fixed.edge, OsdEdge::Bottom);
        assert_eq!(fixed.offset_pct, OSD_MAX_OFFSET_PCT);
        let broken = OsdConfig {
            offset_pct: f32::NAN,
            ..OsdConfig::default()
        };
        assert_eq!(broken.normalized().offset_pct, 0.0);
        let kept = OsdConfig {
            primary_monitor: true,
            style: OsdStyle::Vertical,
            edge: OsdEdge::Left,
            offset_pct: 12.5,
        };
        assert_eq!(kept.normalized(), kept);
    }
}
