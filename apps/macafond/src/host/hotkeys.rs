//! Hotkeys control on the daemon control loop.

use super::DaemonHost;
use crate::osd::{Osd, StatusIcon};
use macafon_config::{OsdConfig, OsdEdge, OsdStyle};
use macafon_ipc::{
    ErrorBody, ErrorCode, Event, HotkeyAction, HotkeyBackend, HotkeyBinding, HotkeysState,
    OsdPrefs, Request,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct VolumeRepeats(HashMap<String, Instant>);

impl DaemonHost {
    /// Every action available now: the global ones and three per channel.
    pub(super) fn hotkey_actions(&self) -> Vec<(HotkeyAction, String)> {
        let soundpad = self.soundpad.is_some();
        let mut actions: Vec<(HotkeyAction, String)> = HotkeyAction::GLOBAL
            .into_iter()
            .filter(|action| {
                soundpad
                    || !matches!(
                        action,
                        HotkeyAction::SoundpadStop
                            | HotkeyAction::SoundpadPauseToggle
                            | HotkeyAction::SoundpadRandom
                    )
            })
            .map(|action| {
                let description = match action {
                    HotkeyAction::QuickPanel => "Open the quick panel",
                    HotkeyAction::MasterVolumeUp => "Master: volume up",
                    HotkeyAction::MasterVolumeDown => "Master: volume down",
                    HotkeyAction::MasterMute => "Master: mute",
                    HotkeyAction::MicMuteToggle => "Toggle microphone mute",
                    HotkeyAction::PushToTalk => "Push to talk",
                    HotkeyAction::PushToMute => "Push to mute",
                    HotkeyAction::BypassToggle => "Toggle processing bypass",
                    HotkeyAction::PresetNext => "Next microphone preset",
                    HotkeyAction::SoundpadStop => "Soundpad: stop",
                    HotkeyAction::SoundpadPauseToggle => "Soundpad: pause or resume",
                    HotkeyAction::SoundpadRandom => "Soundpad: play a random sound",
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
        if let Some(soundpad) = &self.soundpad {
            for entry in soundpad.entries() {
                actions.push((
                    HotkeyAction::SoundpadPlay(entry.id.clone()),
                    format!("Soundpad: {}", entry.name),
                ));
            }
        }
        actions
    }

    /// Register the shortcuts again only when they changed: rebinding releases held actions,
    /// so a fader moved during push-to-talk would mute the microphone.
    pub(super) fn rebind_hotkeys(&mut self) {
        let Some(service) = &self.hotkeys else { return };
        let mut bindings = Vec::new();
        for (action, description) in self.hotkey_actions() {
            let id = action.id();
            for id in [id.clone(), format!("{id}#secondary")] {
                if let Some(shortcut) = self.config.hotkeys.get(&id)
                    && let Ok(accel) = crate::hotkeys::accel::Accel::parse(shortcut)
                {
                    bindings.push(crate::hotkeys::Binding {
                        action: id,
                        description: description.clone(),
                        accel,
                    });
                }
            }
        }
        if self.bound_hotkeys.as_ref() == Some(&bindings) {
            return;
        }
        self.volume_repeats.0.clear();
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
                        secondary_shortcut: self
                            .config
                            .hotkeys
                            .get(&format!("{id}#secondary"))
                            .cloned(),
                        secondary_error: status.and_then(|status| {
                            status.errors.get(&format!("{id}#secondary")).cloned()
                        }),
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
            volume_step_pct: self.config.ui.volume_step_pct,
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
            .any(|(known, _)| known.id() == base_action(action))
        {
            return Err(invalid(format!("unknown action {action:?}")));
        }
        match shortcut {
            None => {
                self.config.hotkeys.remove(action);
            }
            Some(text) => {
                let accel = crate::hotkeys::accel::Accel::parse(&text).map_err(invalid)?;
                if !cfg!(windows) && accel.sided() {
                    return Err(invalid("Side-specific modifiers require Windows".into()));
                }
                let canonical = accel.to_string();
                if let Some((other, _)) = self.config.hotkeys.iter().find(|(other, bound)| {
                    other.as_str() != action
                        && crate::hotkeys::accel::Accel::parse(bound)
                            .is_ok_and(|other| accel.overlaps(&other))
                }) {
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
        self.preload_sounds();
        Ok(())
    }

    /// Run a hotkey action like the matching request (HK-001), then show what it changed
    /// (HK-006).
    pub(super) fn repeat_hotkeys(&mut self, now: Instant) {
        let due: Vec<String> = self
            .volume_repeats
            .0
            .iter_mut()
            .filter_map(|(binding, next)| {
                if now < *next {
                    return None;
                }
                // Skip missed ticks rather than producing a burst after a stalled control loop.
                *next = now + REPEAT_INTERVAL;
                Some(binding.clone())
            })
            .collect();
        for binding in due {
            if let Some(action) = HotkeyAction::parse(base_action(&binding)) {
                self.apply_hotkey(action, true);
            }
        }
    }

    pub(super) fn hotkey(&mut self, action: &str, pressed: bool) {
        self.hotkey_at(action, pressed, Instant::now());
    }

    pub(super) fn hotkey_at(&mut self, action: &str, pressed: bool, now: Instant) {
        let binding = action.to_owned();
        let action = base_action(action);
        let Some(action) = HotkeyAction::parse(action) else {
            return;
        };
        if matches!(
            action,
            HotkeyAction::ChannelVolumeUp(_)
                | HotkeyAction::ChannelVolumeDown(_)
                | HotkeyAction::MasterVolumeUp
                | HotkeyAction::MasterVolumeDown
        ) {
            if !pressed {
                self.volume_repeats.0.remove(&binding);
                return;
            }
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.volume_repeats.0.entry(binding.clone())
            {
                entry.insert(now + REPEAT_DELAY);
            } else {
                return;
            }
        }
        if action.held() {
            if pressed {
                self.pressed_hotkeys.insert(binding);
            } else {
                self.pressed_hotkeys.remove(&binding);
            }
            let active = self
                .pressed_hotkeys
                .iter()
                .any(|id| base_action(id) == action.id());
            if active != pressed {
                return;
            }
        }
        if !pressed && !action.held() {
            return;
        }
        self.apply_hotkey(action, pressed);
    }

    fn apply_hotkey(&mut self, action: HotkeyAction, pressed: bool) {
        let request = match &action {
            HotkeyAction::QuickPanel => {
                crate::ui::overlay();
                return;
            }
            HotkeyAction::MasterVolumeUp | HotkeyAction::MasterVolumeDown => {
                Request::MixerSetMaster {
                    volume_pct: Some(
                        (self.config.mixer.master_volume_pct
                            + f32::from(self.config.ui.volume_step_pct)
                                * if matches!(action, HotkeyAction::MasterVolumeUp) {
                                    1.0
                                } else {
                                    -1.0
                                })
                        .clamp(0.0, 100.0),
                    ),
                    muted: None,
                }
            }
            HotkeyAction::MasterMute => Request::MixerSetMaster {
                volume_pct: None,
                muted: Some(!self.config.mixer.master_muted),
            },
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
                delta_pct: f32::from(self.config.ui.volume_step_pct),
            },
            HotkeyAction::ChannelVolumeDown(id) => Request::ChannelAdjustVolume {
                id: id.clone(),
                delta_pct: -f32::from(self.config.ui.volume_step_pct),
            },
            HotkeyAction::ChannelMute(id) => Request::ChannelToggleMute { id: id.clone() },
            HotkeyAction::SoundpadStop => Request::SoundpadStop,
            HotkeyAction::SoundpadPauseToggle => Request::SoundpadSetPaused { paused: None },
            HotkeyAction::SoundpadRandom => Request::SoundpadPlayRandom { category: None },
            HotkeyAction::SoundpadPlay(id) => Request::SoundpadPlay { id: id.clone() },
        };
        if let Err(error) = self.respond(request) {
            tracing::warn!(?action, %error, "Hotkey action failed");
            return;
        }
        let chain = &self.config.mic.chain;
        let osd = match &action {
            // A sound is heard; a popup on every press would only be in the way.
            HotkeyAction::QuickPanel
            | HotkeyAction::SoundpadStop
            | HotkeyAction::SoundpadPauseToggle
            | HotkeyAction::SoundpadRandom
            | HotkeyAction::SoundpadPlay(_) => return,
            HotkeyAction::MasterVolumeUp
            | HotkeyAction::MasterVolumeDown
            | HotkeyAction::MasterMute => Osd::Volume {
                name: "Master".into(),
                color: "teal".into(),
                icon: "speaker".into(),
                volume_pct: self.config.mixer.master_volume_pct,
                muted: self.config.mixer.master_muted,
            },
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

/// The action of a binding id: secondary shortcuts are stored as `<action>#secondary`. Sound
/// ids may contain `#` themselves, so only that suffix is cut.
pub(super) fn base_action(binding: &str) -> &str {
    binding.strip_suffix("#secondary").unwrap_or(binding)
}

pub(super) fn osd_prefs(config: OsdConfig) -> OsdPrefs {
    OsdPrefs {
        primary_monitor: config.primary_monitor,
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
        primary_monitor: prefs.primary_monitor,
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
