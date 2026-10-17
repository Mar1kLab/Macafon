//! Hotkeys control on the daemon control loop.

use super::DaemonHost;
use crate::osd::{Osd, StatusIcon};
use macafon_config::{OsdConfig, OsdEdge, OsdStyle};
use macafon_ipc::{
    ErrorBody, ErrorCode, Event, HotkeyAction, HotkeyBackend, HotkeyBinding, HotkeysState,
    OsdPrefs, Request,
};
use std::time::Instant;

impl DaemonHost {
    /// Every action available now: the global ones and three per channel.
    pub(super) fn hotkey_actions(&self) -> Vec<(HotkeyAction, String)> {
        let mut actions: Vec<(HotkeyAction, String)> = HotkeyAction::GLOBAL
            .into_iter()
            .map(|action| {
                let description = match action {
                    HotkeyAction::QuickPanel => "Open the quick panel",
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

    /// Register the shortcuts again only when they changed: rebinding releases held actions,
    /// so a fader moved during push-to-talk would mute the microphone.
    pub(super) fn rebind_hotkeys(&mut self) {
        let Some(service) = &self.hotkeys else { return };
        let bindings: Vec<crate::hotkeys::Binding> = self
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
        if self.bound_hotkeys.as_ref() == Some(&bindings) {
            return;
        }
        service.bind(bindings.clone());
        self.bound_hotkeys = Some(bindings);
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
            osd: osd_prefs(self.config.ui.osd),
        }
    }

    /// A popup with the given setup, so Settings shows where and how it appears.
    pub(super) fn sample_osd(&self) {
        let Some(channel) = self.config.mixer.channels.first() else {
            return;
        };
        crate::osd::show(
            Osd::Volume {
                name: channel.name.clone(),
                color: channel.color.clone(),
                icon: channel.icon.clone(),
                volume_pct: channel.settings.volume_pct,
                muted: channel.settings.muted,
            },
            self.config.ui.osd,
        );
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

    /// Run a hotkey action like the matching request (HK-001), then show what it changed
    /// (HK-006).
    pub(super) fn hotkey(&mut self, action: &str, pressed: bool) {
        let Some(action) = HotkeyAction::parse(action) else {
            return;
        };
        if !pressed && !action.held() {
            return;
        }
        let request = match &action {
            HotkeyAction::QuickPanel => {
                crate::ui::quick_panel();
                return;
            }
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
        let chain = &self.config.mic.chain;
        let osd = match &action {
            HotkeyAction::QuickPanel => return,
            HotkeyAction::MicMuteToggle | HotkeyAction::PushToTalk | HotkeyAction::PushToMute => {
                Osd::Status {
                    icon: if chain.muted {
                        StatusIcon::MicOff
                    } else {
                        StatusIcon::Mic
                    },
                    title: "Microphone".to_owned(),
                    value: if chain.muted { "Muted" } else { "On" }.to_owned(),
                    alert: chain.muted,
                }
            }
            HotkeyAction::BypassToggle => Osd::Status {
                icon: StatusIcon::Processing,
                title: "Processing".to_owned(),
                value: if chain.bypass { "Bypassed" } else { "On" }.to_owned(),
                alert: chain.bypass,
            },
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
                Osd::Status {
                    icon: StatusIcon::Preset,
                    title: "Microphone preset".to_owned(),
                    value: name,
                    alert: false,
                }
            }
            HotkeyAction::ChannelVolumeUp(id)
            | HotkeyAction::ChannelVolumeDown(id)
            | HotkeyAction::ChannelMute(id) => {
                let Some(channel) = self.config.mixer.channel(id) else {
                    return;
                };
                Osd::Volume {
                    name: channel.name.clone(),
                    color: channel.color.clone(),
                    icon: channel.icon.clone(),
                    volume_pct: channel.settings.volume_pct,
                    muted: channel.settings.muted,
                }
            }
        };
        crate::osd::show(osd, self.config.ui.osd);
    }
}

pub(super) fn osd_prefs(config: OsdConfig) -> OsdPrefs {
    OsdPrefs {
        style: match config.style {
            OsdStyle::Vertical => macafon_ipc::OsdStyle::Vertical,
            OsdStyle::Horizontal => macafon_ipc::OsdStyle::Horizontal,
        },
        edge: match config.edge {
            OsdEdge::Left => macafon_ipc::OsdEdge::Left,
            OsdEdge::Right => macafon_ipc::OsdEdge::Right,
            OsdEdge::Top => macafon_ipc::OsdEdge::Top,
            OsdEdge::Bottom => macafon_ipc::OsdEdge::Bottom,
        },
        offset_pct: config.offset_pct,
    }
}

/// Settings may send any combination: the edge is fixed up to the style, the offset clamped.
pub(super) fn osd_config(prefs: OsdPrefs) -> OsdConfig {
    OsdConfig {
        style: match prefs.style {
            macafon_ipc::OsdStyle::Vertical => OsdStyle::Vertical,
            macafon_ipc::OsdStyle::Horizontal => OsdStyle::Horizontal,
        },
        edge: match prefs.edge {
            macafon_ipc::OsdEdge::Left => OsdEdge::Left,
            macafon_ipc::OsdEdge::Right => OsdEdge::Right,
            macafon_ipc::OsdEdge::Top => OsdEdge::Top,
            macafon_ipc::OsdEdge::Bottom => OsdEdge::Bottom,
        },
        offset_pct: prefs.offset_pct,
    }
    .normalized()
}
