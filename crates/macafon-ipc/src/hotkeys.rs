//! Global hotkeys (PRD §8.3): actions, shortcuts and the state of the registration backend.

use serde::{Deserialize, Serialize};

/// HK-001 actions. Channel actions carry the channel id: `channel_volume_up.voice`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    /// Open or close the quick panel: every volume and the microphone switches at hand.
    QuickPanel,
    MasterVolumeUp,
    MasterVolumeDown,
    MasterMute,
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
    /// Soundpad (§7.6): stop whatever plays.
    SoundpadStop,
    SoundpadPauseToggle,
    SoundpadRandom,
    /// Play one sound by its library id: `soundpad_play.Memes/airhorn.mp3`.
    SoundpadPlay(String),
}

impl HotkeyAction {
    /// Actions that do not depend on the channels.
    pub const GLOBAL: [Self; 13] = [
        Self::QuickPanel,
        Self::MasterVolumeUp,
        Self::MasterVolumeDown,
        Self::MasterMute,
        Self::MicMuteToggle,
        Self::PushToTalk,
        Self::PushToMute,
        Self::BypassToggle,
        Self::PresetNext,
        Self::PresetPrevious,
        Self::SoundpadStop,
        Self::SoundpadPauseToggle,
        Self::SoundpadRandom,
    ];

    pub fn id(&self) -> String {
        match self {
            Self::QuickPanel => "quick_panel".into(),
            Self::MasterVolumeUp => "master_volume_up".into(),
            Self::MasterVolumeDown => "master_volume_down".into(),
            Self::MasterMute => "master_mute".into(),
            Self::MicMuteToggle => "mic_mute_toggle".into(),
            Self::PushToTalk => "push_to_talk".into(),
            Self::PushToMute => "push_to_mute".into(),
            Self::BypassToggle => "bypass_toggle".into(),
            Self::PresetNext => "preset_next".into(),
            Self::PresetPrevious => "preset_previous".into(),
            Self::ChannelVolumeUp(id) => format!("channel_volume_up.{id}"),
            Self::ChannelVolumeDown(id) => format!("channel_volume_down.{id}"),
            Self::ChannelMute(id) => format!("channel_mute.{id}"),
            Self::SoundpadStop => "soundpad_stop".into(),
            Self::SoundpadPauseToggle => "soundpad_pause".into(),
            Self::SoundpadRandom => "soundpad_random".into(),
            Self::SoundpadPlay(id) => format!("soundpad_play.{id}"),
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "quick_panel" => Self::QuickPanel,
            "master_volume_up" => Self::MasterVolumeUp,
            "master_volume_down" => Self::MasterVolumeDown,
            "master_mute" => Self::MasterMute,
            "mic_mute_toggle" => Self::MicMuteToggle,
            "push_to_talk" => Self::PushToTalk,
            "push_to_mute" => Self::PushToMute,
            "bypass_toggle" => Self::BypassToggle,
            "preset_next" => Self::PresetNext,
            "preset_previous" => Self::PresetPrevious,
            "soundpad_stop" => Self::SoundpadStop,
            "soundpad_pause" => Self::SoundpadPauseToggle,
            "soundpad_random" => Self::SoundpadRandom,
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
                    "soundpad_play" => Self::SoundpadPlay(channel),
                    _ => return None,
                }
            }
        })
    }

    /// HK-004: the equivalent CLI command for desktop shortcut settings.
    pub fn command(&self) -> String {
        match self {
            Self::QuickPanel => "macafon quick-panel".into(),
            Self::MasterVolumeUp => "macafon ctl channel master volume +5".into(),
            Self::MasterVolumeDown => "macafon ctl channel master volume -5".into(),
            Self::MasterMute => "macafon ctl channel master mute".into(),
            Self::MicMuteToggle => "macafon ctl mic mute toggle".into(),
            Self::PushToTalk | Self::PushToMute => String::new(),
            Self::BypassToggle => "macafon ctl mic bypass toggle".into(),
            Self::PresetNext => "macafon ctl preset next".into(),
            Self::PresetPrevious => "macafon ctl preset previous".into(),
            Self::ChannelVolumeUp(id) => format!("macafon ctl channel {id} volume +5"),
            Self::ChannelVolumeDown(id) => format!("macafon ctl channel {id} volume -5"),
            Self::ChannelMute(id) => format!("macafon ctl channel {id} mute"),
            Self::SoundpadStop => "macafon ctl soundpad stop".into(),
            Self::SoundpadPauseToggle => "macafon ctl soundpad pause".into(),
            Self::SoundpadRandom => "macafon ctl soundpad random".into(),
            Self::SoundpadPlay(id) => {
                format!("macafon ctl soundpad play -- {}", quote_argument(id))
            }
        }
    }

    /// Hold actions need key release events, which only real hotkey backends deliver.
    pub fn held(&self) -> bool {
        matches!(self, Self::PushToTalk | Self::PushToMute)
    }
}

// Desktop shortcut commands use PowerShell on Windows and a POSIX shell elsewhere.
fn quote_argument(value: &str) -> String {
    #[cfg(windows)]
    let escaped = value.replace('\'', "''");
    #[cfg(not(windows))]
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

/// Toggle the tray panel in the UI.
pub const QUICK_PANEL_ARG: &str = "--quick-panel";
/// Toggle the centered hotkey overlay in the UI.
pub const OVERLAY_ARG: &str = "--overlay";
/// Start the UI with no window: the daemon keeps it warm so panels open at once.
pub const BACKGROUND_ARG: &str = "--background";

/// What the daemon asks a running UI to show (`ui.show`); the same as the launch arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UiTarget {
    Main,
    /// Toggle the panel by the tray.
    TrayPanel,
    /// Toggle the centered hotkey overlay.
    Overlay,
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
    /// Windows RegisterHotKey with a daemon-owned message loop.
    Windows,
    /// Neither: use the CLI commands in desktop settings (HK-004).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HotkeyBinding {
    pub secondary_shortcut: Option<String>,
    pub secondary_error: Option<String>,
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
    #[cfg(windows)]
    fn soundpad_command_preserves_shell_metacharacters() {
        let id = "-it's $HOME `test` #????.wav";
        let command = HotkeyAction::SoundpadPlay(id.into()).command();
        assert!(command.starts_with("macafon ctl soundpad play -- "));
        let argument = command
            .strip_prefix("macafon ctl soundpad play -- ")
            .expect("prefix");
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &format!(
                "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); & {{ param([string] $id) [Console]::Write($id) }} {argument}"
            )]).output().expect("PowerShell");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout)
                .expect("UTF-8")
                .trim_start_matches('\u{feff}'),
            id
        );
    }
    #[test]
    fn action_ids_round_trip() {
        for action in HotkeyAction::GLOBAL.into_iter().chain([
            HotkeyAction::ChannelVolumeUp("voice".into()),
            HotkeyAction::ChannelMute("ch-2".into()),
            HotkeyAction::SoundpadPlay("Memes/air horn.v2.mp3".into()),
        ]) {
            assert_eq!(HotkeyAction::parse(&action.id()), Some(action));
        }
        assert_eq!(HotkeyAction::parse("channel_mute."), None);
        assert_eq!(HotkeyAction::parse("soundpad_play."), None);
        assert_eq!(HotkeyAction::parse("explode"), None);
    }
}
