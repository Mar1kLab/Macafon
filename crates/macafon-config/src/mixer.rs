//! Output channels, the Master and application rules (PRD §7, §13).

use macafon_dsp::ChannelSettings;
use macafon_dsp::channel::{CHANNEL_VOLUME_PCT, ChannelTemplate};
use serde::{Deserialize, Serialize};

/// CH-002: at most eight channels on Linux.
pub const MAX_CHANNELS: usize = 8;
/// CH-002: the colour palette of a channel (design system §2.1).
pub const CHANNEL_COLORS: [&str; 7] = ["blue", "violet", "pink", "orange", "amber", "teal", "lime"];
/// Channel icons the UI can draw.
pub const CHANNEL_ICONS: [&str; 8] = [
    "speaker",
    "message",
    "music",
    "gamepad",
    "film",
    "bell",
    "globe",
    "headphones",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MixerConfig {
    /// CH-009: off until the user turns it on (onboarding or Settings).
    pub enabled: bool,
    /// OUT-002: physical sinks by `node.name`, most preferred first. Empty: the system
    /// default from before Macafon took over.
    pub output_priority: Vec<String>,
    pub default_channel: String,
    pub master_volume_pct: f32,
    pub master_muted: bool,
    /// APP-006: classify known apps on first sight.
    pub auto_classify: bool,
    pub channels: Vec<ChannelConfig>,
}

impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output_priority: Vec::new(),
            default_channel: "main".into(),
            master_volume_pct: 100.0,
            master_muted: false,
            auto_classify: true,
            channels: vec![
                ChannelConfig::new("main", "Main", "blue", "speaker", ChannelTemplate::Main),
                ChannelConfig::new(
                    "voice",
                    "Voice",
                    "violet",
                    "message",
                    ChannelTemplate::Voice,
                ),
                ChannelConfig::new("music", "Music", "pink", "music", ChannelTemplate::Music),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Stable id: `main`, `voice`, `music`, `ch-<n>`; the node is `macafon_ch_<id>`.
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    /// A channel switched off has no node; its apps play through the default channel until it
    /// is back, and keep their rules meanwhile.
    #[serde(default = "enabled", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: ChannelSettings,
}

fn default_icon() -> String {
    CHANNEL_ICONS[0].to_owned()
}

fn enabled() -> bool {
    true
}

fn is_enabled(enabled: &bool) -> bool {
    *enabled
}

impl ChannelConfig {
    pub fn new(id: &str, name: &str, color: &str, icon: &str, template: ChannelTemplate) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            color: color.to_owned(),
            icon: icon.to_owned(),
            enabled: true,
            settings: ChannelSettings::template(template),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppsConfig {
    pub rules: Vec<AppRule>,
}

/// `[[apps.rules]]`: where an application's streams go (APP-005), or that Macafon leaves the
/// app alone (APP-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRule {
    pub key: String,
    pub channel: String,
    #[serde(default = "managed")]
    pub managed: bool,
    /// CH-008: the app's own fader inside its channel, 0–100 % on the cubic curve.
    #[serde(default = "full_volume", skip_serializing_if = "is_full_volume")]
    pub volume_pct: f32,
}

fn managed() -> bool {
    true
}

fn full_volume() -> f32 {
    100.0
}

fn is_full_volume(volume_pct: &f32) -> bool {
    *volume_pct == 100.0
}

impl AppsConfig {
    pub fn rule(&self, key: &str) -> Option<&AppRule> {
        self.rules.iter().find(|rule| rule.key == key)
    }

    pub fn set(&mut self, key: &str, channel: &str) {
        match self.rules.iter_mut().find(|rule| rule.key == key) {
            Some(rule) => channel.clone_into(&mut rule.channel),
            None => self.rules.push(AppRule {
                key: key.to_owned(),
                channel: channel.to_owned(),
                managed: true,
                volume_pct: full_volume(),
            }),
        }
    }
}

impl MixerConfig {
    pub fn channel(&self, id: &str) -> Option<&ChannelConfig> {
        self.channels.iter().find(|channel| channel.id == id)
    }

    pub fn channel_mut(&mut self, id: &str) -> Option<&mut ChannelConfig> {
        self.channels.iter_mut().find(|channel| channel.id == id)
    }

    /// A channel that exists and is switched on: apps can play through it.
    pub fn active(&self, id: &str) -> bool {
        self.channel(id).is_some_and(|channel| channel.enabled)
    }

    /// Where an app assigned to `id` plays: there while it is on, else the default channel.
    pub fn effective_channel<'a>(&'a self, id: &'a str) -> &'a str {
        if self.active(id) {
            id
        } else {
            &self.default_channel
        }
    }

    /// Linear Master gain applied inside every channel (CH-006).
    pub fn master_gain(&self) -> f32 {
        if self.master_muted {
            0.0
        } else {
            macafon_dsp::channel::volume_gain(self.master_volume_pct)
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.channels.is_empty() || self.channels.len() > MAX_CHANNELS {
            return Err(format!("the mixer needs 1–{MAX_CHANNELS} channels"));
        }
        CHANNEL_VOLUME_PCT
            .check(self.master_volume_pct)
            .map_err(|error| error.to_string())?;
        match self.channel(&self.default_channel) {
            None => {
                return Err(format!(
                    "unknown default channel {:?}",
                    self.default_channel
                ));
            }
            Some(channel) if !channel.enabled => {
                return Err("the default channel cannot be switched off".into());
            }
            Some(_) => {}
        }
        for (index, channel) in self.channels.iter().enumerate() {
            if !valid_id(&channel.id) {
                return Err(format!("invalid channel id {:?}", channel.id));
            }
            if self.channels[..index]
                .iter()
                .any(|other| other.id == channel.id)
            {
                return Err(format!("duplicate channel id {:?}", channel.id));
            }
            if channel.name.trim().is_empty() || channel.name.chars().count() > 32 {
                return Err("channel names have 1–32 characters".into());
            }
            if !CHANNEL_COLORS.contains(&channel.color.as_str()) {
                return Err(format!("unknown channel colour {:?}", channel.color));
            }
            channel
                .settings
                .validate()
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// The next free id for a user channel.
    pub fn next_id(&self) -> String {
        (1..)
            .map(|n| format!("ch-{n}"))
            .find(|id| self.channel(id).is_none())
            .unwrap_or_default()
    }
}

/// Lower-case ASCII letters, digits and `-`: safe inside a PipeWire `node.name`.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 24
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_round_trip_through_toml() {
        let mut mixer = MixerConfig::default();
        assert_eq!(mixer.validate(), Ok(()));
        assert!(!mixer.enabled);
        assert_eq!(mixer.next_id(), "ch-1");
        mixer.channels[1].settings.volume_pct = 42.0;
        let text = toml::to_string(&mixer).expect("serialize");
        assert!(
            text.contains("[[channels]]") && text.contains("volume_pct = 42.0"),
            "{text}"
        );
        assert_eq!(toml::from_str::<MixerConfig>(&text).expect("parse"), mixer);
        mixer.default_channel = "gone".into();
        assert!(mixer.validate().is_err());
        mixer.default_channel = "main".into();
        mixer.channels[2].id = "Main Two".into();
        assert!(mixer.validate().is_err());
    }

    #[test]
    fn switched_off_channels_send_apps_to_the_default_and_stay_off_after_a_reload() {
        let mut mixer = MixerConfig::default();
        mixer.channels[2].enabled = false;
        assert_eq!(mixer.effective_channel("music"), "main");
        assert_eq!(mixer.effective_channel("voice"), "voice");
        assert_eq!(mixer.effective_channel("gone"), "main");
        let text = toml::to_string(&mixer).expect("serialize");
        let parsed = toml::from_str::<MixerConfig>(&text).expect("parse");
        assert_eq!(parsed, mixer);
        // A config written before channels could be switched off has them all on.
        let mut old: toml::Table = toml::from_str(&text).expect("table");
        for channel in old["channels"].as_array_mut().expect("channels") {
            channel.as_table_mut().expect("channel").remove("enabled");
        }
        let old: MixerConfig = old.try_into().expect("old config");
        assert!(old.channels.iter().all(|channel| channel.enabled));
        mixer.channels[0].enabled = false;
        assert!(mixer.validate().is_err(), "the default channel stays on");
    }
}
