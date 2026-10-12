//! Hotkeys control on the daemon control loop.

use super::DaemonHost;
use macafon_ipc::{
    ErrorBody, ErrorCode, Event, HotkeyAction, HotkeyBackend, HotkeyBinding, HotkeysState, Request,
};
use std::time::Instant;

impl DaemonHost {
    /// Every action available now: the global ones and three per channel.
    pub(super) fn hotkey_actions(&self) -> Vec<(HotkeyAction, String)> {
        let mut actions: Vec<(HotkeyAction, String)> = HotkeyAction::GLOBAL
            .into_iter()
            .map(|action| {
                let description = match action {
                    HotkeyAction::MicMuteToggle => "Toggle microphone mute",
                    HotkeyAction::PushToTalk => "Push to talk",
                    HotkeyAction::PushToMute => "Push to mute",
                    HotkeyAction::BypassToggle => "Toggle processing bypass",
                    HotkeyAction::PresetNext => "Next microphone preset",
                    _ => "Previous microphone preset",
                };
                (action, description.to_owned())
            })
            .collect();
        for channel in &self.config.mixer.channels {
            let name = &channel.name;
            actions.push((
                HotkeyAction::ChannelVolumeUp(channel.id.clone()),
                format!("{name}: volume up"),
            ));
            actions.push((
                HotkeyAction::ChannelVolumeDown(channel.id.clone()),
                format!("{name}: volume down"),
            ));
            actions.push((
                HotkeyAction::ChannelMute(channel.id.clone()),
                format!("{name}: mute"),
            ));
        }
        actions
    }

    pub(super) fn rebind_hotkeys(&self) {
        let Some(service) = &self.hotkeys else { return };
        let bindings = self
            .hotkey_actions()
            .into_iter()
            .filter_map(|(action, description)| {
                let shortcut = self.config.hotkeys.get(&action.id())?;
                let accel = crate::hotkeys::accel::Accel::parse(shortcut).ok()?;
                Some(crate::hotkeys::Binding {
                    action: action.id(),
                    description,
                    accel,
                })
            })
            .collect();
        service.bind(bindings);
    }

    pub(super) fn hotkeys_state(&self) -> HotkeysState {
        let status = self.hotkey_status.as_ref();
        HotkeysState {
            backend: status.map_or(HotkeyBackend::Unavailable, |status| status.backend),
            bindings: self
                .hotkey_actions()
                .into_iter()
                .map(|(action, _)| {
                    let id = action.id();
                    HotkeyBinding {
                        shortcut: self.config.hotkeys.get(&id).cloned(),
                        system_trigger: status.and_then(|status| status.triggers.get(&id).cloned()),
                        error: status.and_then(|status| status.errors.get(&id).cloned()),
                        command: action.command(),
                        action: id,
                    }
                })
                .collect(),
        }
    }

    pub(super) fn prefs(&self) -> macafon_ipc::Preferences {
        macafon_ipc::Preferences {
            quantum_frames: self.config.audio.quantum_frames,
            close_action: if self.config.ui.quit_on_close {
                macafon_ipc::CloseAction::Quit
            } else {
                macafon_ipc::CloseAction::Tray
            },
        }
    }

    pub(super) fn hotkeys_changed(&self) {
        self.hub
            .broadcast(&Event::HotkeysChanged(self.hotkeys_state()));
    }

    /// HK-002: canonical shortcut, unique within Macafon.
    pub(super) fn set_hotkey(
        &mut self,
        action: &str,
        shortcut: Option<String>,
    ) -> Result<(), ErrorBody> {
        let invalid = |message: String| ErrorBody::new(ErrorCode::InvalidParams, message);
        if !self
            .hotkey_actions()
            .iter()
            .any(|(known, _)| known.id() == action)
        {
            return Err(invalid(format!("unknown action {action:?}")));
        }
        match shortcut {
            None => {
                self.config.hotkeys.remove(action);
            }
            Some(text) => {
                let canonical = crate::hotkeys::accel::Accel::parse(&text)
                    .map_err(invalid)?
                    .to_string();
                if let Some((other, _)) = self
                    .config
                    .hotkeys
                    .iter()
                    .find(|(other, bound)| other.as_str() != action && **bound == canonical)
                {
                    return Err(ErrorBody::new(
                        ErrorCode::AlreadyExists,
                        format!("{canonical} is already used by {other}"),
                    ));
                }
                self.config.hotkeys.insert(action.to_owned(), canonical);
            }
        }
        self.changed_at = Some(Instant::now());
        self.rebind_hotkeys();
        self.hotkeys_changed();
        Ok(())
    }

    /// Run a hotkey action like the matching request (HK-001); OSD for presets and volume
    /// (HK-006).
    pub(super) fn hotkey(&mut self, action: &str, pressed: bool) {
        let Some(action) = HotkeyAction::parse(action) else {
            return;
        };
        if !pressed && !action.held() {
            return;
        }
        let request = match &action {
            HotkeyAction::MicMuteToggle => Request::MicToggleMute,
            HotkeyAction::PushToTalk => Request::MicSetMuted { muted: !pressed },
            HotkeyAction::PushToMute => Request::MicSetMuted { muted: pressed },
            HotkeyAction::BypassToggle => Request::MicSetBypass {
                bypass: !self.config.mic.chain.bypass,
            },
            HotkeyAction::PresetNext => Request::MicCyclePreset { step: 1 },
            HotkeyAction::PresetPrevious => Request::MicCyclePreset { step: -1 },
            HotkeyAction::ChannelVolumeUp(id) => Request::ChannelAdjustVolume {
                id: id.clone(),
                delta_pct: 5.0,
            },
            HotkeyAction::ChannelVolumeDown(id) => Request::ChannelAdjustVolume {
                id: id.clone(),
                delta_pct: -5.0,
            },
            HotkeyAction::ChannelMute(id) => Request::ChannelToggleMute { id: id.clone() },
        };
        if let Err(error) = self.respond(request) {
            tracing::warn!(?action, %error, "Hotkey action failed");
            return;
        }
        match &action {
            HotkeyAction::PresetNext | HotkeyAction::PresetPrevious => {
                let name = self
                    .config
                    .mic
                    .preset
                    .as_deref()
                    .and_then(|id| {
                        self.preset_list()
                            .into_iter()
                            .find(|preset| preset.id == id)
                    })
                    .map_or_else(|| "Custom".to_owned(), |preset| preset.name);
                crate::notify::osd(format!("Preset: {name}"));
            }
            HotkeyAction::ChannelVolumeUp(id)
            | HotkeyAction::ChannelVolumeDown(id)
            | HotkeyAction::ChannelMute(id) => {
                if let Some(channel) = self.config.mixer.channel(id) {
                    let s = &channel.settings;
                    let text = if s.muted {
                        format!("{}: muted", channel.name)
                    } else {
                        format!("{}: {:.0}%", channel.name, s.volume_pct)
                    };
                    crate::notify::osd(text);
                }
            }
            _ => {}
        }
    }
}
