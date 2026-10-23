//! Daemon configuration: `$XDG_CONFIG_HOME/macafon/config.toml` (PRD §13).

pub mod dictation;
pub mod displays;
pub mod mixer;
pub mod presets;
pub mod sounddeck;
mod storage;

pub(crate) use storage::write_atomically;
pub use storage::{Error, Loaded, Recovered, Store, config_dir, state_dir};

pub use displays::DisplaysConfig;
pub use mixer::{AppRule, AppsConfig, ChannelConfig, MixerConfig};
pub use sounddeck::{SoundConfig, SoundDeckConfig};

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
    /// `[sounddeck]`; configs before 0.9.0 call it `[soundpad]`.
    #[serde(alias = "soundpad")]
    pub sounddeck: SoundDeckConfig,
    pub dictation: dictation::DictationConfig,
    pub displays: DisplaysConfig,
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
    /// `[ui.overlay]`: hotkey overlay controls and placement.
    pub overlay: macafon_ipc::OverlayPrefs,
    /// The `DEFAULT_HOTKEYS` revision already offered, so a shortcut the user cleared stays
    /// cleared.
    pub hotkey_defaults: u32,
    #[serde(default = "default_volume_step")]
    pub volume_step_pct: u8,
    /// The monitor the main window opens on (its system name); empty: the system's choice.
    #[serde(default)]
    pub main_monitor: String,
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
            overlay: Default::default(),
            hotkey_defaults: 0,
            volume_step_pct: default_volume_step(),
            main_monitor: String::new(),
        }
    }
}

/// Defaults are offered as whole groups, only for channels that still exist.
pub const DEFAULT_HOTKEYS: [&[(&str, &str)]; 5] = [
    &[("quick_panel", "LCtrl+LAlt+Z")],
    &[("mic_mute_toggle", "Ctrl+Alt+Q")],
    &[
        ("channel_volume_up.main", "RCtrl+PageUp"),
        ("channel_volume_down.main", "RCtrl+PageDown"),
    ],
    &[
        ("channel_volume_up.voice", "RAlt+PageUp"),
        ("channel_volume_down.voice", "RAlt+PageDown"),
    ],
    &[
        ("channel_volume_up.music", "Ctrl+Alt+R"),
        ("channel_volume_down.music", "Ctrl+Alt+F"),
    ],
];
/// Offered once on Windows, one action at a time: an action that has no shortcut gets its
/// default when that shortcut is free, also in customized maps — the actions are new, so
/// nothing the user chose is replaced (STT-003).
pub const DICTATION_HOTKEYS: [(&str, &str); 5] = [
    ("dictation_hold", "Ctrl+Win"),
    ("dictation_toggle", "Ctrl+Win+Space"),
    ("dictation_paste_last", "Shift+Alt+Z"),
    ("dictation_copy_last", "Shift+Alt+X"),
    ("dictation_hold_copy", "Ctrl+Win+Shift"),
];
/// Offered once on Windows without replacing customized shortcuts.
pub const DISPLAY_HOTKEYS: [(&str, &str); 2] = [
    ("display_all_on", "Ctrl+Alt+Shift+F12"),
    ("display_revert", "Ctrl+Alt+Shift+F11"),
];
pub const HOTKEY_DEFAULTS_REVISION: u32 = 5;
const DISPLAY_DEFAULTS_REVISION: u32 = 5;
/// The revision that first offered `DICTATION_HOTKEYS` to every map.
const DICTATION_DEFAULTS_REVISION: u32 = 4;

/// Exact previous shipped set: partial or customized maps cannot safely be identified as
/// untouched, including maps with deliberately cleared or secondary bindings.
const LEGACY_HOTKEYS: [(&str, &str); 8] = [
    ("quick_panel", "Ctrl+Alt+Backquote"),
    ("mic_mute_toggle", "Ctrl+Alt+Q"),
    ("channel_volume_up.main", "Ctrl+Alt+W"),
    ("channel_volume_down.main", "Ctrl+Alt+S"),
    ("channel_volume_up.voice", "Ctrl+Alt+E"),
    ("channel_volume_down.voice", "Ctrl+Alt+D"),
    ("channel_volume_up.music", "Ctrl+Alt+R"),
    ("channel_volume_down.music", "Ctrl+Alt+F"),
];

/// How the hotkey popup looks and where it sits (HK-006, Windows).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdConfig {
    /// Sketch on a new install (owner decision, 2026-10-22); a `[ui.osd]` saved before the
    /// looks existed keeps Classic.
    #[serde(default = "classic_look")]
    pub look: OsdLook,
    pub primary_monitor: bool,
    pub style: OsdStyle,
    /// Left or right for the vertical style, top or bottom for the horizontal one.
    pub edge: OsdEdge,
    /// Distance from that edge in percent of the screen, up to `OSD_MAX_OFFSET_PCT`.
    pub offset_pct: f32,
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            look: OsdLook::Sketch,
            primary_monitor: false,
            style: OsdStyle::default(),
            edge: OsdEdge::default(),
            offset_pct: 0.0,
        }
    }
}

fn classic_look() -> OsdLook {
    OsdLook::Classic
}

/// The farthest the popup moves from its edge toward the center.
pub const OSD_MAX_OFFSET_PCT: f32 = 40.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsdLook {
    #[default]
    Classic,
    Sketch,
}

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
            look: self.look,
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
    /// Offer defaults to fresh configs or migrate the exact untouched previous set. All
    /// other maps are preserved, including cleared bindings. `usable` protects typing on
    /// installed keyboard layouts. Returns whether the config changed.
    pub fn offer_default_hotkeys(&mut self, usable: impl Fn(&str) -> bool) -> bool {
        if self.ui.hotkey_defaults >= HOTKEY_DEFAULTS_REVISION {
            return false;
        }
        let offer_dictation =
            cfg!(windows) && self.ui.hotkey_defaults < DICTATION_DEFAULTS_REVISION;
        let offer_displays = cfg!(windows) && self.ui.hotkey_defaults < DISPLAY_DEFAULTS_REVISION;
        let fresh = self.ui.hotkey_defaults == 0 && self.hotkeys.is_empty();
        let previous = self.ui.hotkey_defaults == 2
            && self.hotkeys.len()
                == DEFAULT_HOTKEYS
                    .iter()
                    .map(|group| group.len())
                    .sum::<usize>()
            && DEFAULT_HOTKEYS
                .iter()
                .flat_map(|group| group.iter())
                .all(|(id, shortcut)| self.hotkeys.get(*id).is_some_and(|s| s == shortcut));
        let legacy = self.hotkeys.len() == LEGACY_HOTKEYS.len()
            && LEGACY_HOTKEYS.iter().all(|(action, shortcut)| {
                self.hotkeys
                    .get(*action)
                    .is_some_and(|bound| bound == shortcut)
            });
        self.ui.hotkey_defaults = HOTKEY_DEFAULTS_REVISION;
        if offer_dictation {
            for (action, shortcut) in DICTATION_HOTKEYS {
                if !self.hotkeys.contains_key(action)
                    && !self.hotkeys.values().any(|bound| bound == shortcut)
                    && usable(shortcut)
                {
                    self.hotkeys.insert(action.to_owned(), shortcut.to_owned());
                }
            }
        }
        if offer_displays {
            for (action, shortcut) in DISPLAY_HOTKEYS {
                if !self.hotkeys.contains_key(action)
                    && !self.hotkeys.values().any(|bound| bound == shortcut)
                    && usable(shortcut)
                {
                    self.hotkeys.insert(action.to_owned(), shortcut.to_owned());
                }
            }
        }
        if !(fresh || legacy || previous) {
            return true;
        }
        if legacy {
            for (action, _) in DEFAULT_HOTKEYS[0]
                .iter()
                .chain(DEFAULT_HOTKEYS[2])
                .chain(DEFAULT_HOTKEYS[3])
            {
                self.hotkeys.remove(*action);
            }
        }
        for group in DEFAULT_HOTKEYS {
            let free = group.iter().all(|(action, shortcut)| {
                action.split_once('.').is_none_or(|(_, channel)| {
                    self.mixer.channels.iter().any(|item| item.id == channel)
                }) && !self.hotkeys.contains_key(*action)
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
            sounddeck: SoundDeckConfig::default(),
            dictation: dictation::DictationConfig::default(),
            displays: DisplaysConfig::default(),
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
    fn display_defaults_are_offered_once_without_overwriting_bindings() {
        let mut config = Config::default();
        config.ui.hotkey_defaults = 4;
        config.hotkeys.insert("display_revert".into(), "F7".into());
        config.offer_default_hotkeys(|_| true);
        assert_eq!(config.hotkeys["display_revert"], "F7");
        assert_eq!(config.hotkeys.contains_key("display_all_on"), cfg!(windows));
        config.hotkeys.remove("display_all_on");
        assert!(!config.offer_default_hotkeys(|_| true));
        assert!(!config.hotkeys.contains_key("display_all_on"));
        let mut taken = Config::default();
        taken.ui.hotkey_defaults = 4;
        taken
            .hotkeys
            .insert("custom".into(), "Ctrl+Alt+Shift+F12".into());
        taken.offer_default_hotkeys(|_| true);
        assert!(!taken.hotkeys.contains_key("display_all_on"));
    }

    #[test]
    fn default_hotkeys_migrate_only_fresh_or_exact_legacy_maps() {
        let mut config = Config::default();
        assert!(config.offer_default_hotkeys(|_| true));
        assert_eq!(config.hotkeys["quick_panel"], "LCtrl+LAlt+Z");
        assert_eq!(config.hotkeys["channel_volume_up.main"], "RCtrl+PageUp");
        assert_eq!(config.hotkeys["channel_volume_down.voice"], "RAlt+PageDown");
        assert_eq!(config.hotkeys["mic_mute_toggle"], "Ctrl+Alt+Q");
        assert_eq!(config.hotkeys["channel_volume_down.music"], "Ctrl+Alt+F");
        let expected = config.hotkeys.clone();
        config.hotkeys.remove("mic_mute_toggle");
        assert!(!config.offer_default_hotkeys(|_| true));
        assert!(!config.hotkeys.contains_key("mic_mute_toggle"));

        let legacy: BTreeMap<String, String> = LEGACY_HOTKEYS
            .iter()
            .map(|(action, shortcut)| ((*action).into(), (*shortcut).into()))
            .collect();
        for revision in [0, 1] {
            let mut old = Config::default();
            old.ui.hotkey_defaults = revision;
            old.hotkeys = legacy.clone();
            assert!(old.offer_default_hotkeys(|_| true));
            assert_eq!(old.hotkeys, expected);
            assert!(!old.offer_default_hotkeys(|_| true));
        }
        for edit in ["custom", "cleared", "secondary", "all_cleared"] {
            let mut old = Config::default();
            old.ui.hotkey_defaults = 1;
            old.hotkeys = legacy.clone();
            match edit {
                "custom" => {
                    old.hotkeys
                        .insert("quick_panel".into(), "Ctrl+Alt+Z".into());
                }
                "cleared" => {
                    old.hotkeys.remove("quick_panel");
                }
                "secondary" => {
                    old.hotkeys
                        .insert("quick_panel#secondary".into(), "F8".into());
                }
                _ => old.hotkeys.clear(),
            }
            let before = old.hotkeys.clone();
            assert!(old.offer_default_hotkeys(|_| true));
            // New dictation/display actions may be added; existing choices stay intact.
            old.hotkeys.retain(|action, _| {
                !action.starts_with("dictation_") && !action.starts_with("display_")
            });
            assert_eq!(old.hotkeys, before, "{edit}");
        }
        let mut missing = Config::default();
        missing
            .mixer
            .channels
            .retain(|channel| channel.id != "voice");
        missing.offer_default_hotkeys(|shortcut| shortcut != "RCtrl+PageUp");
        assert!(!missing.hotkeys.contains_key("channel_volume_down.main"));
        assert!(!missing.hotkeys.contains_key("channel_volume_down.voice"));
    }

    #[cfg(windows)]
    #[test]
    fn customized_maps_get_free_dictation_defaults_once() {
        let mut config = Config::default();
        config.ui.hotkey_defaults = 3;
        config.hotkeys = BTreeMap::from([
            ("quick_panel".into(), "Ctrl+F1".into()),
            ("mic_mute_toggle".into(), "Shift+Alt+Z".into()),
            ("dictation_copy_last".into(), "F7".into()),
        ]);
        assert!(config.offer_default_hotkeys(|_| true));
        assert_eq!(config.hotkeys["quick_panel"], "Ctrl+F1");
        assert_eq!(config.hotkeys["dictation_hold"], "Ctrl+Win");
        assert_eq!(config.hotkeys["dictation_toggle"], "Ctrl+Win+Space");
        assert_eq!(config.hotkeys["dictation_hold_copy"], "Ctrl+Win+Shift");
        // Taken by another action, or already set by the user: left alone.
        assert!(!config.hotkeys.contains_key("dictation_paste_last"));
        assert_eq!(config.hotkeys["dictation_copy_last"], "F7");
        // Offered once: a cleared default stays cleared.
        config.hotkeys.remove("dictation_hold");
        assert!(!config.offer_default_hotkeys(|_| true));
        assert!(!config.hotkeys.contains_key("dictation_hold"));
    }

    #[test]
    fn old_overlay_preferences_keep_the_classic_layout() {
        let config: Config = toml::from_str(
            "[ui.osd]\nstyle = 'horizontal'\n[sounddeck]\nshow_in_overlay = false\n",
        )
        .expect("old config");
        assert_eq!(config.ui.osd.look, OsdLook::Classic);
        assert_eq!(Config::default().ui.osd.look, OsdLook::Sketch);
        assert_eq!(
            Config::default().ui.overlay.expand,
            macafon_ipc::OverlayExpand::Right
        );
        assert_eq!(config.ui.overlay, macafon_ipc::OverlayPrefs::default());
        let mut changed = config.clone();
        changed.ui.overlay.expand = macafon_ipc::OverlayExpand::Down;
        changed.ui.overlay.placement = macafon_ipc::OverlayPlacement::Centered;
        changed.ui.overlay.output_device = true;
        changed.ui.overlay.volumes = false;
        changed.ui.overlay.dictation_history = false;
        changed.ui.overlay.order = vec![macafon_ipc::OverlayItem::DictationHistory];
        let encoded = toml::to_string(&changed).expect("serialize overlay");
        assert!(encoded.contains("[ui.overlay]"));
        assert_eq!(
            toml::from_str::<Config>(&encoded).expect("decode overlay"),
            changed
        );
        assert_eq!(
            config.sounddeck.overlay_view,
            sounddeck::SoundDeckOverlayView::Grid
        );
        assert_eq!(
            config.sounddeck.overlay_controls,
            sounddeck::SoundDeckOverlayControls::Top
        );
        assert!(!config.sounddeck.show_in_overlay);
    }

    #[test]
    fn osd_setup_keeps_an_edge_of_its_style_and_a_sane_offset() {
        let fixed = OsdConfig {
            look: Default::default(),
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
            look: Default::default(),
            primary_monitor: true,
            style: OsdStyle::Vertical,
            edge: OsdEdge::Left,
            offset_pct: 12.5,
        };
        assert_eq!(kept.normalized(), kept);
    }
}
