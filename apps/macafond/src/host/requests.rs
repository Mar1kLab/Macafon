//! Requests control on the daemon control loop.

use super::{DaemonHost, backend_error};
use macafon_ipc::{CalibrationState, ErrorBody, ErrorCode, Event, HotkeyBackend, Request};
use std::time::{Duration, Instant};

impl DaemonHost {
    pub(super) fn respond(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        let state = match request {
            Request::MicCyclePreset { step } => self.cycle_preset(step)?,
            Request::HotkeysGet => return Ok(serde_json::json!(self.hotkeys_state())),
            Request::PrefsGet => return Ok(serde_json::json!(self.prefs())),
            Request::PrefsSet {
                quantum_frames,
                close_action,
            } => {
                if let Some(frames) = quantum_frames {
                    if !macafon_config::QUANTUM_CHOICES.contains(&frames) {
                        return Err(ErrorBody::new(
                            ErrorCode::InvalidParams,
                            format!(
                                "quantum must be one of {:?}",
                                macafon_config::QUANTUM_CHOICES
                            ),
                        ));
                    }
                    self.config.audio.quantum_frames = frames;
                }
                if let Some(action) = close_action {
                    self.config.ui.quit_on_close = action == macafon_ipc::CloseAction::Quit;
                }
                self.changed_at = Some(Instant::now());
                let prefs = self.prefs();
                self.hub.broadcast(&Event::PrefsChanged(prefs.clone()));
                return Ok(serde_json::json!(prefs));
            }
            Request::HotkeysSet { action, shortcut } => {
                self.set_hotkey(&action, shortcut)?;
                return Ok(serde_json::json!(self.hotkeys_state()));
            }
            Request::MicGet => self.state(),
            Request::MicSet { settings } => self.update(|config| config.mic.chain = settings)?,
            Request::MicSetPreset { preset } => {
                let settings = self.preset_settings(&preset).ok_or_else(|| {
                    ErrorBody::new(ErrorCode::NotFound, format!("unknown preset {preset:?}"))
                })?;
                self.update(|config| {
                    let mic = &mut config.mic;
                    let (muted, bypass) = (mic.chain.muted, mic.chain.bypass);
                    mic.chain = settings;
                    mic.chain.muted = muted;
                    mic.chain.bypass = bypass;
                    mic.preset = Some(preset);
                })?
            }
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
            Request::AbOutputs => {
                let nodes = crate::platform::output_devices().map_err(backend_error)?;
                let outputs: Vec<macafon_ipc::PlaybackDevice> = nodes
                    .into_iter()
                    .map(|node| {
                        let kind = match node.form_factor.as_str() {
                            "headset" | "headphone" => macafon_ipc::PlaybackDeviceKind::Headphones,
                            "speaker" | "speakers" => macafon_ipc::PlaybackDeviceKind::Speakers,
                            _ => macafon_ipc::PlaybackDeviceKind::Unknown,
                        };
                        macafon_ipc::PlaybackDevice {
                            name: node.name,
                            description: node.description,
                            kind,
                            speaker_warning: kind != macafon_ipc::PlaybackDeviceKind::Headphones,
                        }
                    })
                    .collect();
                return Ok(serde_json::json!(outputs));
            }
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
            Request::AppsIcon { icon } => return Ok(serde_json::json!(self.icons.data_url(&icon))),
            Request::DoctorRun => {
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
                let report = crate::doctor::report(crate::doctor::Facts {
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
                });
                return serde_json::to_value(report)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()));
            }
            Request::DoctorFix { check } => {
                return crate::platform::doctor_fix(check)
                    .map(|()| serde_json::Value::Null)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error));
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
        serde_json::to_value(state)
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }
}
