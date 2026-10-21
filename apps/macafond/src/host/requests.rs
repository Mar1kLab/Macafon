//! Requests control on the daemon control loop.

use super::microphone::replace_chain;
use super::{DaemonHost, backend_error, reply};
use macafon_ipc::{
    CalibrationState, ErrorBody, ErrorCode, Event, HotkeyBackend, PlaybackDevice,
    PlaybackDeviceKind, Preferences, Request,
};
use std::time::{Duration, Instant};

impl DaemonHost {
    pub(super) fn respond(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        let state = match request {
            Request::OutputControlsList => return Ok(serde_json::json!([])),
            request @ (Request::DictationGet
            | Request::DictationSetPrefs { .. }
            | Request::DictationDownload { .. }
            | Request::DictationCancelDownload { .. }
            | Request::DictationDeleteModel { .. }
            | Request::DictationImportModel { .. }
            | Request::DictationSetModelsDir { .. }
            | Request::DictationCancelMove
            | Request::DictationTestStart
            | Request::DictationTestStop
            | Request::DictationCancel
            | Request::DictationHistoryClear
            | Request::DictationReinsert { .. }) => return self.respond_dictation(request),
            Request::OutputControlsSet { .. } => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "output controls unavailable",
                ));
            }
            Request::MicCyclePreset { step } => self.cycle_preset(step)?,
            Request::HotkeysGet => return Ok(serde_json::json!(self.hotkeys_state())),
            Request::PrefsGet => return Ok(serde_json::json!(self.prefs())),
            Request::PrefsSet {
                quantum_frames,
                volume_step_pct,
                close_action,
                osd,
                overlay,
                main_monitor,
            } => {
                return reply(self.set_prefs(PrefsPatch {
                    quantum_frames,
                    volume_step_pct,
                    close_action,
                    osd,
                    overlay,
                    main_monitor,
                })?);
            }
            Request::HotkeysSet {
                action,
                shortcut,
                secondary,
            } => {
                let id = if secondary {
                    format!("{action}#secondary")
                } else {
                    action
                };
                self.set_hotkey(&id, shortcut)?;
                return Ok(serde_json::json!(self.hotkeys_state()));
            }
            Request::MicGet => self.state(),
            Request::MicSet { settings } => {
                self.update(|config| replace_chain(&mut config.mic.chain, settings))?
            }
            Request::MicSetPreset { preset } => self.load_preset(preset)?,
            Request::MicSetMuted { muted } => {
                self.update(|config| config.mic.chain.muted = muted)?
            }
            Request::MicToggleMute => {
                self.update(|config| config.mic.chain.muted = !config.mic.chain.muted)?
            }
            Request::MicSetBypass { bypass } => {
                self.update(|config| config.mic.chain.bypass = bypass)?
            }
            Request::MicSetDevice { device } => {
                self.update(|config| {
                    config.mic.device = device;
                })?;
                self.refresh_input();
                self.state()
            }
            Request::MicSetInputChannel { channel } => {
                self.update(|config| config.mic.input_channel = channel)?
            }
            Request::MicSetFallbackDevice { device } => {
                self.update(|config| config.mic.fallback_device = device)?;
                self.refresh_input();
                self.state()
            }
            Request::PresetsList => return Ok(serde_json::json!(self.preset_list())),
            request @ (Request::PresetsSave { .. }
            | Request::PresetsUpdate { .. }
            | Request::PresetsRename { .. }
            | Request::PresetsDuplicate { .. }
            | Request::PresetsDelete { .. }
            | Request::PresetsExport { .. }
            | Request::PresetsImport { .. }) => return self.respond_presets(request),
            Request::DevicesList => return Ok(serde_json::json!(self.inputs)),
            Request::MicSetDefaultSource { confirmed } => {
                if !confirmed {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "explicit confirmation is required",
                    ));
                }
                crate::platform::set_default_source().map_err(backend_error)?;
                return Ok(serde_json::Value::Null);
            }
            Request::MicRestoreDefaultSource => {
                crate::platform::restore_default_source().map_err(backend_error)?;
                return Ok(serde_json::Value::Null);
            }
            Request::AbExport { side, path } => {
                self.ab
                    .export(&self.config.mic.chain, side, std::path::Path::new(&path))?;
                return Ok(serde_json::Value::Null);
            }
            Request::AbOutputs => return reply(ab_outputs()?),
            Request::CalibrationStart => {
                self.start_calibration()?;
                return self.calibration_value();
            }
            Request::CalibrationCancel => {
                if self.recording.take().is_some() {
                    if let Some(control) = &mut self.control {
                        control.stop_capture();
                    }
                    self.set_calibration(CalibrationState::Idle);
                }
                return self.calibration_value();
            }
            Request::CalibrationGet => return self.calibration_value(),
            request @ (Request::AbRecord { .. }
            | Request::AbStopRecording
            | Request::AbPlay { .. }
            | Request::AbStop
            | Request::AbSelect { .. }
            | Request::AbToggle
            | Request::AbSeek { .. }
            | Request::AbSetLoudnessMatch { .. }
            | Request::AbGet
            | Request::AbWaveform { .. }) => return self.respond_ab(request),
            #[cfg(any(target_os = "linux", windows))]
            Request::AutostartGet => return Ok(serde_json::json!(crate::autostart::state())),
            #[cfg(not(any(target_os = "linux", windows)))]
            Request::AutostartGet | Request::AutostartSet { .. } => {
                return Err(ErrorBody::new(
                    ErrorCode::NotFound,
                    "start with the system is not available on this platform yet",
                ));
            }
            #[cfg(any(target_os = "linux", windows))]
            Request::AutostartSet { enabled } => {
                let state = crate::autostart::set(enabled)
                    .map_err(|message| ErrorBody::new(ErrorCode::Internal, message))?;
                tracing::info!(?state, "Autostart changed");
                return Ok(serde_json::json!(state));
            }
            request @ (Request::MixerGet
            | Request::MixerSetEnabled { .. }
            | Request::MixerSetMaster { .. }
            | Request::MixerSetOutput { .. }
            | Request::MixerSetOutputPriority { .. }
            | Request::MixerSetAutoClassify { .. }
            | Request::ChannelCreate { .. }
            | Request::ChannelUpdate { .. }
            | Request::ChannelDelete { .. }
            | Request::ChannelSet { .. }
            | Request::ChannelAdjustVolume { .. }
            | Request::ChannelToggleMute { .. }
            | Request::ChannelSetDefault { .. }
            | Request::ChannelReorder { .. }
            | Request::AppsMove { .. }
            | Request::AppsSetManaged { .. }
            | Request::AppsForget { .. }
            | Request::AppsSetVolume { .. }) => return self.respond_mixer(request),
            request @ (Request::SoundpadGet
            | Request::SoundpadRescan
            | Request::SoundpadOpenFolder
            | Request::SoundpadSetFolder { .. }
            | Request::SoundpadPlay { .. }
            | Request::SoundpadPlayRandom { .. }
            | Request::SoundpadStop
            | Request::SoundpadSetPaused { .. }
            | Request::SoundpadSeek { .. }
            | Request::SoundpadSetLooping { .. }
            | Request::SoundpadSetPrefs { .. }
            | Request::SoundpadSetMonitorDevice { .. }
            | Request::SoundpadSetSound { .. }
            | Request::SoundpadRename { .. }
            | Request::SoundpadMove { .. }
            | Request::SoundpadDelete { .. }
            | Request::SoundpadDeleteProblemSounds
            | Request::SoundpadCreateCategory { .. }
            | Request::SoundpadRenameCategory { .. }
            | Request::SoundpadDeleteCategory { .. }
            | Request::SoundpadUpload { .. }) => return self.respond_soundpad(request),
            request @ (Request::DisplaysGet
            | Request::DisplaysSave { .. }
            | Request::DisplaysCreate { .. }
            | Request::DisplaysSuggestions
            | Request::DisplaysUpdate { .. }
            | Request::DisplaysRecapture { .. }
            | Request::DisplaysDelete { .. }
            | Request::DisplaysReorder { .. }
            | Request::DisplaysApply { .. }
            | Request::DisplaysConfirm { .. }
            | Request::DisplaysRevert
            | Request::DisplaysAllOn) => return self.respond_displays(request),
            Request::AppsIcon { icon } => return Ok(serde_json::json!(self.icons.data_url(&icon))),
            Request::DoctorRun => return reply(self.doctor()),
            Request::DoctorFix { check } => {
                return crate::platform::doctor_fix(check)
                    .map(|()| serde_json::Value::Null)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error));
            }
            Request::UiWindowClosed => {
                if self.dictation.test {
                    self.cancel_dictation();
                }
                self.window_closed();
                return Ok(serde_json::Value::Null);
            }
            Request::DaemonQuit => {
                tracing::info!("Quit requested");
                self.quit = true;
                return Ok(serde_json::Value::Null);
            }
            Request::Hello { .. }
            | Request::MetersSubscribe { .. }
            | Request::MetersUnsubscribe
            | Request::DevicesSubscribe
            | Request::DevicesUnsubscribe => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "handled by the connection, not the daemon",
                ));
            }
        };
        reply(state)
    }

    fn set_prefs(&mut self, patch: PrefsPatch) -> Result<Preferences, ErrorBody> {
        if let Some(step) = patch.volume_step_pct
            && !(1..=20).contains(&step)
        {
            return Err(ErrorBody::new(
                ErrorCode::InvalidParams,
                "volume step must be 1..20",
            ));
        }
        if let Some(frames) = patch.quantum_frames
            && !macafon_config::QUANTUM_CHOICES.contains(&frames)
        {
            return Err(ErrorBody::new(
                ErrorCode::InvalidParams,
                format!(
                    "quantum must be one of {:?}",
                    macafon_config::QUANTUM_CHOICES
                ),
            ));
        }
        let ui = &mut self.config.ui;
        if let Some(step) = patch.volume_step_pct {
            ui.volume_step_pct = step;
        }
        if let Some(frames) = patch.quantum_frames {
            self.config.audio.quantum_frames = frames;
        }
        if let Some(action) = patch.close_action {
            ui.quit_on_close = action == macafon_ipc::CloseAction::Quit;
        }
        if let Some(overlay) = patch.overlay {
            ui.overlay = overlay;
        }
        if let Some(monitor) = patch.main_monitor {
            ui.main_monitor = monitor;
        }
        if let Some(osd) = patch.osd {
            ui.osd = super::hotkeys::osd_config(osd);
            self.sample_osd();
        }
        self.changed_at = Some(Instant::now());
        let prefs = self.prefs();
        self.hub.broadcast(&Event::PrefsChanged(prefs.clone()));
        Ok(prefs)
    }

    fn doctor(&mut self) -> macafon_ipc::DoctorReport {
        const WINDOW: Duration = Duration::from_secs(600);
        while self
            .overloads
            .front()
            .is_some_and(|at| at.elapsed() > WINDOW)
        {
            self.overloads.pop_front();
        }
        let (mic, mixer) = (self.state(), self.mixer_state());
        let microphone = mic
            .input_connected
            .then_some(mic.active_device.as_deref())
            .flatten();
        let platform = crate::platform::doctor_checks(microphone);
        crate::doctor::report(crate::doctor::Facts {
            inspection: crate::platform::inspect(),
            mic: &mic,
            mixer: &mixer,
            clock: self.clock,
            overloads: self.overloads.len(),
            inputs: &self.inputs,
            hotkeys: self
                .hotkey_status
                .as_ref()
                .map_or(HotkeyBackend::Unavailable, |status| status.backend),
            platform,
        })
    }

    /// The main window closed; the warm UI process keeps running without it.
    fn window_closed(&mut self) {
        self.ab.clear(self.control.as_mut());
        self.ab_changed();
        // §9.5: the user chose "close quits Macafon".
        if self.config.ui.quit_on_close {
            tracing::info!("Window closed; quitting as configured");
            self.quit = true;
            return;
        }
        // BG-001: the window closed, the sound keeps going; say so once.
        if !self.quit && !self.config.ui.close_hint_shown {
            self.config.ui.close_hint_shown = true;
            self.changed_at = Some(Instant::now());
            crate::notify::send(
                "Macafon keeps running in the tray",
                "Your microphone and mixer stay on. Open Macafon from the tray icon.",
            );
        }
    }
}

/// The optional fields of `Request::PrefsSet`.
struct PrefsPatch {
    quantum_frames: Option<u32>,
    volume_step_pct: Option<u8>,
    close_action: Option<macafon_ipc::CloseAction>,
    osd: Option<macafon_ipc::OsdPrefs>,
    overlay: Option<macafon_ipc::OverlayPrefs>,
    main_monitor: Option<String>,
}

/// Playback devices for A/B listening; anything but headphones may feed back into the mic.
fn ab_outputs() -> Result<Vec<PlaybackDevice>, ErrorBody> {
    let nodes = crate::platform::output_devices().map_err(backend_error)?;
    Ok(nodes
        .into_iter()
        .map(|node| {
            let kind = match node.form_factor.as_str() {
                "headset" | "headphone" => PlaybackDeviceKind::Headphones,
                "speaker" | "speakers" => PlaybackDeviceKind::Speakers,
                _ => PlaybackDeviceKind::Unknown,
            };
            PlaybackDevice {
                name: node.name,
                description: node.description,
                kind,
                speaker_warning: kind != PlaybackDeviceKind::Headphones,
            }
        })
        .collect())
}
