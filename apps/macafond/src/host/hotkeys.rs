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

/// A held volume key waits this long after its first step before it starts moving.
const HOLD_DELAY: Duration = Duration::from_millis(350);
/// While held, the volume moves on a steady frame clock; each frame adds the distance the
/// eased speed covers since the previous one, so late or uneven frames never change the speed.
const HOLD_FRAME: Duration = Duration::from_millis(30);
/// A stalled control loop resumes from here instead of catching up in one jump.
const HOLD_MAX_GAP: f32 = 0.1;
/// Speed in steps per second: it starts here and eases up to `HOLD_FAST` over `HOLD_RAMP_S`.
const HOLD_SLOW: f32 = 8.0;
pub(super) const HOLD_FAST: f32 = 40.0;
const HOLD_RAMP_S: f32 = 1.6;

/// Volume up and down repeat while held; everything else acts once per press.
fn is_volume_step(action: &HotkeyAction) -> bool {
    matches!(
        action,
        HotkeyAction::ChannelVolumeUp(_)
            | HotkeyAction::ChannelVolumeDown(_)
            | HotkeyAction::MasterVolumeUp
            | HotkeyAction::MasterVolumeDown
    )
}

struct VolumeHold {
    /// When the held motion starts (after `HOLD_DELAY`).
    moving_from: Instant,
    /// Seconds of motion already applied.
    applied_s: f32,
    next: Instant,
}

/// Steps travelled after `moving_s` seconds of motion: the integral of a speed that eases
/// from `HOLD_SLOW` to `HOLD_FAST` along a smoothstep, so the acceleration has no corners.
fn hold_distance(moving_s: f32) -> f32 {
    let moving_s = moving_s.max(0.0);
    let p = (moving_s / HOLD_RAMP_S).min(1.0);
    let ramp = HOLD_RAMP_S * (p * p * p - 0.5 * p * p * p * p);
    let after = (moving_s - HOLD_RAMP_S).max(0.0);
    HOLD_SLOW * moving_s + (HOLD_FAST - HOLD_SLOW) * (ramp + after)
}

pub(super) struct VolumeRepeats(HashMap<String, VolumeHold>, crate::hotkeys::RepeatTimer);

impl VolumeRepeats {
    pub(super) fn new(control: macafon_backend::ControlSender<super::ControlMessage>) -> Self {
        Self(HashMap::new(), crate::hotkeys::RepeatTimer::spawn(control))
    }

    fn schedule(&self) {
        self.1.schedule(self.0.values().map(|hold| hold.next).min());
    }
}

impl DaemonHost {
    /// Every action available now: the global ones and three per channel.
    pub(super) fn hotkey_actions(&self) -> Vec<(HotkeyAction, String)> {
        let sounddeck = self.sounddeck.is_some();
        let displays = self.displays_available();
        let mut actions: Vec<(HotkeyAction, String)> = HotkeyAction::GLOBAL
            .into_iter()
            .filter(|action| {
                sounddeck
                    || !matches!(
                        action,
                        HotkeyAction::SoundDeckStop
                            | HotkeyAction::SoundDeckPauseToggle
                            | HotkeyAction::SoundDeckRandom
                    )
            })
            .filter(|action| {
                displays
                    || !matches!(
                        action,
                        HotkeyAction::DisplayProfileNext
                            | HotkeyAction::DisplayRevert
                            | HotkeyAction::DisplayAllOn
                    )
            })
            .map(|action| {
                let description = match action {
                    HotkeyAction::QuickPanel => "Open the quick panel",
                    HotkeyAction::DictationHold => "Dictation: hold",
                    HotkeyAction::DictationToggle => "Dictation: toggle",
                    HotkeyAction::DictationPasteLast => "Dictation: paste last",
                    HotkeyAction::DictationCopyLast => "Dictation: copy last",
                    HotkeyAction::DictationHoldCopy => "Dictation: hold copy",
                    HotkeyAction::DictationEnableToggle => "Dictation: enable toggle",
                    HotkeyAction::DictationLanguageCycle => "Dictation: language cycle",
                    HotkeyAction::MasterVolumeUp => "Master: volume up",
                    HotkeyAction::MasterVolumeDown => "Master: volume down",
                    HotkeyAction::MasterMute => "Master: mute",
                    HotkeyAction::MicMuteToggle => "Toggle microphone mute",
                    HotkeyAction::PushToTalk => "Push to talk",
                    HotkeyAction::PushToMute => "Push to mute",
                    HotkeyAction::BypassToggle => "Toggle processing bypass",
                    HotkeyAction::PresetNext => "Next microphone preset",
                    HotkeyAction::SoundDeckStop => "SoundDeck: stop",
                    HotkeyAction::SoundDeckPauseToggle => "SoundDeck: pause or resume",
                    HotkeyAction::SoundDeckRandom => "SoundDeck: play a random sound",
                    HotkeyAction::PresetPrevious => "Previous microphone preset",
                    HotkeyAction::DisplayProfileNext => "Displays: next profile",
                    HotkeyAction::DisplayRevert => "Displays: previous layout",
                    HotkeyAction::DisplayAllOn => "Displays: all monitors on",
                    // Not global: described with their channel or sound below.
                    HotkeyAction::ChannelVolumeUp(_)
                    | HotkeyAction::ChannelVolumeDown(_)
                    | HotkeyAction::ChannelMute(_)
                    | HotkeyAction::SoundDeckPlay(_)
                    | HotkeyAction::DisplayProfile(_) => "",
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
        if let Some(sounddeck) = &self.sounddeck {
            for entry in sounddeck.entries() {
                actions.push((
                    HotkeyAction::SoundDeckPlay(entry.id.clone()),
                    format!("SoundDeck: {}", entry.name),
                ));
            }
        }
        if displays {
            for profile in &self.config.displays.profiles {
                actions.push((
                    HotkeyAction::DisplayProfile(profile.id.clone()),
                    format!("Display: {}", profile.name),
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
            // Off, dictation keeps only the shortcut that turns it back on.
            if action.id().starts_with("dictation_")
                && action != HotkeyAction::DictationEnableToggle
                && !self.config.dictation.enabled
            {
                continue;
            }
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
        if self.config.dictation.enabled {
            if let Ok(accel) = crate::hotkeys::accel::Accel::parse("Escape") {
                bindings.push(crate::hotkeys::Binding {
                    action: "dictation_cancel".into(),
                    description: "Cancel dictation".into(),
                    accel,
                });
            }
        }
        if self.bound_hotkeys.as_ref() == Some(&bindings) {
            return;
        }
        self.volume_repeats.0.clear();
        self.volume_repeats.schedule();
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
            overlay: self.config.ui.overlay.clone(),
            main_monitor: self.config.ui.main_monitor.clone(),
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
        let due: Vec<(String, f32)> = self
            .volume_repeats
            .0
            .iter_mut()
            .filter_map(|(binding, hold)| {
                if now < hold.next {
                    return None;
                }
                hold.next = now + HOLD_FRAME;
                let moving_s = now
                    .saturating_duration_since(hold.moving_from)
                    .as_secs_f32();
                // Skip a stall rather than producing a burst after it.
                let from_s = hold.applied_s.max(moving_s - HOLD_MAX_GAP);
                hold.applied_s = moving_s;
                let steps = hold_distance(moving_s) - hold_distance(from_s);
                (steps > 0.0).then(|| (binding.clone(), steps))
            })
            .collect();
        self.volume_repeats.schedule();
        for (binding, steps) in due {
            if let Some(action) = HotkeyAction::parse(base_action(&binding)) {
                self.apply_hotkey(action, true, steps);
            }
        }
    }

    pub(super) fn hotkey(&mut self, action: &str, pressed: bool) {
        self.hotkey_at(action, pressed, Instant::now());
    }

    pub(super) fn hotkey_at(&mut self, action: &str, pressed: bool, now: Instant) {
        if action == "dictation_cancel" {
            if pressed {
                self.cancel_dictation();
            }
            return;
        }
        let binding = action.to_owned();
        let action = base_action(action);
        let Some(action) = HotkeyAction::parse(action) else {
            return;
        };
        if is_volume_step(&action) {
            if !pressed {
                self.volume_repeats.0.remove(&binding);
                self.volume_repeats.schedule();
                return;
            }
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.volume_repeats.0.entry(binding.clone())
            {
                entry.insert(VolumeHold {
                    moving_from: now + HOLD_DELAY,
                    applied_s: 0.0,
                    next: now + HOLD_DELAY + HOLD_FRAME,
                });
                self.volume_repeats.schedule();
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
        self.apply_hotkey(action, pressed, 1.0);
    }

    fn apply_hotkey(&mut self, action: HotkeyAction, pressed: bool, step_scale: f32) {
        if action.id().starts_with("dictation_") {
            self.dictation_hotkey(action, pressed);
            return;
        }
        let step_pct = f32::from(self.config.ui.volume_step_pct) * step_scale;
        let display_name = match &action {
            HotkeyAction::DisplayProfile(id) => {
                self.config.displays.profile(id).map(|p| p.name.clone())
            }
            HotkeyAction::DisplayProfileNext => self
                .next_display_profile()
                .and_then(|id| self.config.displays.profile(&id).map(|p| p.name.clone())),
            HotkeyAction::DisplayRevert => Some("Previous layout".into()),
            HotkeyAction::DisplayAllOn => Some("All monitors".into()),
            _ => None,
        };
        let request = match &action {
            // Handled by `dictation_hotkey` above.
            HotkeyAction::DictationHold
            | HotkeyAction::DictationToggle
            | HotkeyAction::DictationPasteLast
            | HotkeyAction::DictationCopyLast
            | HotkeyAction::DictationHoldCopy
            | HotkeyAction::DictationEnableToggle
            | HotkeyAction::DictationLanguageCycle => return,
            HotkeyAction::QuickPanel => {
                if self.config.ui.overlay.enabled {
                    crate::ui::overlay();
                }
                return;
            }
            HotkeyAction::MasterVolumeUp | HotkeyAction::MasterVolumeDown => {
                let delta_pct = if matches!(action, HotkeyAction::MasterVolumeUp) {
                    step_pct
                } else {
                    -step_pct
                };
                Request::MixerSetMaster {
                    volume_pct: Some(
                        (self.config.mixer.master_volume_pct + delta_pct).clamp(0.0, 100.0),
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
                delta_pct: step_pct,
            },
            HotkeyAction::ChannelVolumeDown(id) => Request::ChannelAdjustVolume {
                id: id.clone(),
                delta_pct: -step_pct,
            },
            HotkeyAction::ChannelMute(id) => Request::ChannelToggleMute { id: id.clone() },
            HotkeyAction::SoundDeckStop => Request::SoundDeckStop,
            HotkeyAction::SoundDeckPauseToggle => Request::SoundDeckSetPaused { paused: None },
            HotkeyAction::SoundDeckRandom => Request::SoundDeckPlayRandom { category: None },
            HotkeyAction::SoundDeckPlay(id) => Request::SoundDeckPlay { id: id.clone() },
            HotkeyAction::DisplayProfile(id) => Request::DisplaysApply { id: id.clone() },
            HotkeyAction::DisplayProfileNext => match self.next_display_profile() {
                Some(id) => Request::DisplaysApply { id },
                None => return,
            },
            HotkeyAction::DisplayRevert => Request::DisplaysRevert,
            HotkeyAction::DisplayAllOn => Request::DisplaysAllOn,
        };
        if let Err(error) = self.respond(request) {
            tracing::warn!(?action, %error, "Hotkey action failed");
            return;
        }
        let chain = &self.config.mic.chain;
        let osd = match &action {
            HotkeyAction::DictationHold
            | HotkeyAction::DictationToggle
            | HotkeyAction::DictationPasteLast
            | HotkeyAction::DictationCopyLast
            | HotkeyAction::DictationHoldCopy
            | HotkeyAction::DictationEnableToggle
            | HotkeyAction::DictationLanguageCycle => return,
            // A sound is heard; a popup on every press would only be in the way.
            HotkeyAction::QuickPanel
            | HotkeyAction::SoundDeckStop
            | HotkeyAction::SoundDeckPauseToggle
            | HotkeyAction::SoundDeckRandom
            | HotkeyAction::SoundDeckPlay(_) => return,
            HotkeyAction::DisplayProfile(_)
            | HotkeyAction::DisplayProfileNext
            | HotkeyAction::DisplayRevert
            | HotkeyAction::DisplayAllOn => Osd::Status {
                icon: StatusIcon::Monitor,
                title: "Display".into(),
                value: display_name.unwrap_or_default(),
                alert: false,
            },
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
        look: match config.look {
            macafon_config::OsdLook::Classic => macafon_ipc::OsdLook::Classic,
            macafon_config::OsdLook::Sketch => macafon_ipc::OsdLook::Sketch,
        },
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
        look: match prefs.look {
            macafon_ipc::OsdLook::Classic => macafon_config::OsdLook::Classic,
            macafon_ipc::OsdLook::Sketch => macafon_config::OsdLook::Sketch,
        },
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

#[cfg(test)]
mod acceleration_tests {
    use super::*;

    #[test]
    fn held_speed_eases_from_slow_to_fast_without_jumps() {
        let speed = |t: f32| (hold_distance(t + 0.005) - hold_distance(t - 0.005)) / 0.01;
        assert!((speed(0.005) - HOLD_SLOW).abs() < 0.1);
        assert!((speed(HOLD_RAMP_S / 2.0) - (HOLD_SLOW + HOLD_FAST) / 2.0).abs() < 0.1);
        assert!((speed(HOLD_RAMP_S + 1.0) - HOLD_FAST).abs() < 0.1);
        // Monotonic speed with a bounded change per 10 ms: no steps in the ramp.
        let mut previous = speed(0.005);
        for ms in (15..3000).step_by(10) {
            let now = speed(ms as f32 / 1000.0);
            assert!(now + 0.02 >= previous, "slows down at {ms} ms");
            assert!(now - previous < 0.5, "jump at {ms} ms");
            previous = now;
        }
    }
}
