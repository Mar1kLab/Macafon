//! Global hotkeys (PRD §8.3): actions, shortcuts and the state of the registration backend.

use serde::{Deserialize, Serialize};

/// HK-001 actions. Channel actions carry the channel id: `channel_volume_up.voice`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    MicMuteToggle,
    /// Held: microphone on; released: muted.
    PushToTalk,
    /// Held: muted; released: on.
    PushToMute,
    BypassToggle,
    PresetNext,
    PresetPrevious,
    ChannelVolumeUp(String),
    ChannelVolumeDown(String),
    ChannelMute(String),
}

impl HotkeyAction {
    /// Actions that do not depend on the channels.
    pub const GLOBAL: [Self; 6] = [
        Self::MicMuteToggle,
        Self::PushToTalk,
        Self::PushToMute,
        Self::BypassToggle,
        Self::PresetNext,
        Self::PresetPrevious,
    ];

    pub fn id(&self) -> String {
        match self {
            Self::MicMuteToggle => "mic_mute_toggle".into(),
            Self::PushToTalk => "push_to_talk".into(),
            Self::PushToMute => "push_to_mute".into(),
            Self::BypassToggle => "bypass_toggle".into(),
            Self::PresetNext => "preset_next".into(),
            Self::PresetPrevious => "preset_previous".into(),
            Self::ChannelVolumeUp(id) => format!("channel_volume_up.{id}"),
            Self::ChannelVolumeDown(id) => format!("channel_volume_down.{id}"),
            Self::ChannelMute(id) => format!("channel_mute.{id}"),
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "mic_mute_toggle" => Self::MicMuteToggle,
            "push_to_talk" => Self::PushToTalk,
            "push_to_mute" => Self::PushToMute,
            "bypass_toggle" => Self::BypassToggle,
            "preset_next" => Self::PresetNext,
            "preset_previous" => Self::PresetPrevious,
            _ => {
                let (kind, channel) = id.split_once('.')?;
                let channel = channel.to_owned();
                if channel.is_empty() {
                    return None;
                }
                match kind {
                    "channel_volume_up" => Self::ChannelVolumeUp(channel),
                    "channel_volume_down" => Self::ChannelVolumeDown(channel),
                    "channel_mute" => Self::ChannelMute(channel),
                    _ => return None,
                }
            }
        })
    }

    /// HK-004: the equivalent CLI command for desktop shortcut settings.
    pub fn command(&self) -> String {
        match self {
            Self::MicMuteToggle => "macafon ctl mic mute toggle".into(),
            Self::PushToTalk | Self::PushToMute => String::new(),
            Self::BypassToggle => "macafon ctl mic bypass toggle".into(),
            Self::PresetNext => "macafon ctl preset next".into(),
            Self::PresetPrevious => "macafon ctl preset previous".into(),
            Self::ChannelVolumeUp(id) => format!("macafon ctl channel {id} volume +5"),
            Self::ChannelVolumeDown(id) => format!("macafon ctl channel {id} volume -5"),
            Self::ChannelMute(id) => format!("macafon ctl channel {id} mute"),
        }
    }

    /// Hold actions need key release events, which only real hotkey backends deliver.
    pub fn held(&self) -> bool {
        matches!(self, Self::PushToTalk | Self::PushToMute)
    }
}

/// How the daemon registers shortcuts (HK-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HotkeyBackend {
    /// `org.freedesktop.portal.GlobalShortcuts`; the desktop confirms and may change keys.
    Portal,
    /// X11 key grabs.
    X11,
    /// Neither: use the CLI commands in desktop settings (HK-004).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HotkeyBinding {
    pub action: String,
    /// Canonical shortcut, `Ctrl+Alt+M`.
    pub shortcut: Option<String>,
    /// What the desktop actually assigned (portal), if it says.
    pub system_trigger: Option<String>,
    /// The shortcut could not be registered (taken by another app, invalid key).
    pub error: Option<String>,
    /// HK-004 command; empty for hold actions.
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HotkeysState {
    pub backend: HotkeyBackend,
    /// Every action available now, bound or not, in display order.
    pub bindings: Vec<HotkeyBinding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_ids_round_trip() {
        for action in HotkeyAction::GLOBAL.into_iter().chain([
            HotkeyAction::ChannelVolumeUp("voice".into()),
            HotkeyAction::ChannelMute("ch-2".into()),
        ]) {
            assert_eq!(HotkeyAction::parse(&action.id()), Some(action));
        }
        assert_eq!(HotkeyAction::parse("channel_mute."), None);
        assert_eq!(HotkeyAction::parse("explode"), None);
    }
}
