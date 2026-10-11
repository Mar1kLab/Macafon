//! Daemon state on the PipeWire control loop: the desired microphone configuration, the live
//! engine of the current graph session and config persistence.

use crate::ab::Ab;
use crate::calibration::{Poll, Recording};
use crate::mixer::Mixer;
use crate::server::{self, Hub, SocketGuard};
use crate::tray::{Tray, TrayState};
use macafon_backend_pipewire::ControlSender;
use macafon_backend_pipewire::{
    ChannelRequest, Error, MicRt, MicrophoneHost, MixerHost, Node, PlaybackStream, PlayerRequest,
    Route,
};
use macafon_config::presets::{self, PresetError, PresetStore};
use macafon_config::{Config, Store};
use macafon_dsp::calibration::CalibrationError;
use macafon_dsp::{ChainSettings, Preset, Telemetry};
use macafon_engine::{EngineError, MicControl};
use macafon_ipc::{
    CalibrationFailure, CalibrationState, Device, ErrorBody, ErrorCode, Event, HotkeyAction,
    HotkeyBackend, HotkeyBinding, HotkeysState, MicState, Outcome, PresetInfo, Request,
};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// CFG-001: write the config 500 ms after the last change.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[allow(clippy::large_enum_variant)] // One transient request; settings are already bounded by the protocol.
pub enum ControlMessage {
    Request {
        request: Request,
        reply: mpsc::Sender<Outcome>,
    },
    Disconnected,
    /// A global shortcut went down or up (HK-001).
    Hotkey {
        action: String,
        pressed: bool,
    },
    /// The hotkey backend registered (or failed to register) the shortcuts.
    HotkeyStatus(crate::hotkeys::Status),
}

pub struct DaemonHost {
    config: Config,
    config_recovery: Option<macafon_ipc::ConfigRecovery>,
    store: Option<Store>,
    presets: PresetStore,
    changed_at: Option<Instant>,
    control: Option<MicControl>,
    telemetry: Arc<Telemetry>,
    inputs: Vec<Device>,
    input_connected: bool,
    selected_device: Option<String>,
    selected_stable_id: Option<String>,
    active_device: Option<String>,
    recording: Option<Recording>,
    calibration: CalibrationState,
    detected_channel: macafon_dsp::InputChannel,
    ab: Ab,
    mixer: Mixer,
    /// Graph `(rate, quantum)` from the microphone node (Doctor).
    clock: Option<(u32, u32)>,
    /// Noise suppression overload fallbacks, for the Doctor's 10-minute window.
    overloads: std::collections::VecDeque<Instant>,
    hub: Arc<Hub>,
    tray: Option<Tray>,
    tray_wanted: bool,
    hotkeys: Option<crate::hotkeys::Hotkeys>,
    hotkey_status: Option<crate::hotkeys::Status>,
    quit: bool,
    server: Option<(PathBuf, ControlSender<ControlMessage>)>,
    _socket: Option<SocketGuard>,
}

impl DaemonHost {
    pub fn new(
        mut config: Config,
        store: Option<Store>,
        presets: PresetStore,
        socket: PathBuf,
        control: ControlSender<ControlMessage>,
        tray: bool,
    ) -> Self {
        let hub = Hub::new();
        if config.mic.noise_floor_dbfs.is_some() && config.mic.noise_floor_device.is_none() {
            // Older configurations only supported calibration of the primary input.
            config.mic.noise_floor_device = config.mic.device.clone();
        }
        let selected_device = config.mic.device.clone();
        Self {
            selected_device,
            selected_stable_id: None,
            active_device: None,
            config,
            config_recovery: None,
            store,
            presets,
            changed_at: None,
            control: None,
            telemetry: hub.telemetry(),
            inputs: Vec::new(),
            input_connected: false,
            recording: None,
            calibration: CalibrationState::Idle,
            detected_channel: macafon_dsp::InputChannel::Left,
            ab: Ab::default(),
            mixer: Mixer::default(),
            clock: None,
            overloads: std::collections::VecDeque::new(),
            hub,
            tray: None,
            tray_wanted: tray,
            hotkeys: None,
            hotkey_status: None,
            quit: false,
            server: Some((socket, control)),
            _socket: None,
        }
    }

    pub fn set_config_recovery(&mut self, recovery: Option<macafon_ipc::ConfigRecovery>) {
        self.config_recovery = recovery;
    }

    /// Persist pending changes now (shutdown).
    pub fn flush(&mut self) {
        if self.changed_at.take().is_some() {
            self.save();
        }
    }

    fn save(&self) {
        if let Some(store) = &self.store {
            match store.save(&self.config) {
                Ok(()) => tracing::debug!(path = %store.path().display(), "Config saved"),
                Err(error) => tracing::error!(%error, "Cannot save config"),
            }
        }
    }

    /// Settings of a built-in or user preset.
    fn preset_settings(&self, id: &str) -> Option<ChainSettings> {
        Preset::from_id(id)
            .map(Preset::settings)
            .or_else(|| self.presets.get(id).map(|preset| preset.settings))
    }

    fn preset_list(&self) -> Vec<PresetInfo> {
        let builtin = Preset::ALL.into_iter().map(|preset| PresetInfo {
            id: preset.id().to_owned(),
            name: preset.name().to_owned(),
            builtin: true,
        });
        let user = self.presets.list().iter().map(|preset| PresetInfo {
            id: preset.id.clone(),
            name: preset.name.clone(),
            builtin: false,
        });
        builtin.chain(user).collect()
    }

    fn preset_info(&self, id: &str) -> Result<serde_json::Value, ErrorBody> {
        let info = self
            .preset_list()
            .into_iter()
            .find(|preset| preset.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::Internal, "preset vanished"))?;
        Ok(serde_json::json!(info))
    }

    fn state(&self) -> MicState {
        let preset_modified = self.config.mic.preset.as_deref().is_some_and(|id| {
            self.preset_settings(id)
                .is_none_or(|preset| !presets::same_sound(&self.config.mic.chain, &preset))
        });
        MicState {
            stereo: self.config.mic.stereo,
            input_channel: self.config.mic.input_channel,
            config_recovery: self.config_recovery.clone(),
            settings: self.config.mic.chain,
            preset: self.config.mic.preset.clone(),
            preset_modified,
            device: self.config.mic.device.clone(),
            fallback_device: self.config.mic.fallback_device.clone(),
            active_device: self.active_device.clone(),
            input_connected: self.input_connected,
            latency_samples: self.config.mic.chain.latency_samples(),
            noise_floor_dbfs: self.config.mic.noise_floor_dbfs,
        }
    }

    fn refresh_input(&mut self) {
        let selected = select_input(&self.config.mic, &self.inputs).map(str::to_owned);
        let stable_id = selected
            .as_ref()
            .and_then(|name| self.inputs.iter().find(|d| &d.name == name))
            .map(|d| {
                if d.stable_id.is_empty() {
                    d.name.clone()
                } else {
                    d.stable_id.clone()
                }
            });
        let selection_changed =
            self.selected_device != selected || self.selected_stable_id != stable_id;
        if selection_changed {
            let previous_id = self.selected_stable_id.take();
            if let Some(previous) = &previous_id {
                self.config.device_profiles.insert(
                    previous.clone(),
                    macafon_config::MicProfile::capture(&self.config.mic),
                );
            }
            if let (Some(id), Some(name)) = (&stable_id, &selected) {
                if let Some(profile) = self.config.device_profiles.get(id).cloned() {
                    let result = self.update(|config| profile.apply(&mut config.mic, name));
                    if let Err(error) = result {
                        self.selected_stable_id = previous_id;
                        tracing::warn!(%error, "Cannot load device profile yet");
                        return;
                    }
                } else if previous_id.is_some() {
                    let defaults = macafon_config::MicConfig::default();
                    let result = self.update(|config| {
                        config.mic.chain = defaults.chain;
                        config.mic.preset = defaults.preset;
                        config.mic.noise_floor_dbfs = None;
                        config.mic.noise_floor_device = None;
                        config.mic.input_channel = defaults.input_channel;
                    });
                    if let Err(error) = result {
                        self.selected_stable_id = previous_id;
                        tracing::warn!(%error, "Cannot initialize device profile yet");
                        return;
                    }
                }
            }
            self.selected_stable_id = stable_id;
        }
        let floor_invalid = self.config.mic.noise_floor_dbfs.is_some()
            && self.config.mic.noise_floor_device != selected;
        if floor_invalid {
            self.config.mic.noise_floor_dbfs = None;
            self.config.mic.noise_floor_device = None;
            self.changed_at = Some(Instant::now());
        }
        if selection_changed {
            self.selected_device = selected;
            self.active_device = None;
            self.input_connected = false;
            if self.recording.is_some() {
                self.fail_calibration(
                    CalibrationFailure::Disconnected,
                    "the microphone changed during calibration",
                );
            }
            if self.ab.is_recording() {
                self.ab.abort_recording(self.control.as_mut());
                self.ab_changed();
            }
        }
        if selection_changed || floor_invalid {
            self.changed();
        }
    }

    fn set_calibration(&mut self, state: CalibrationState) {
        self.calibration = state;
        self.hub
            .broadcast(&Event::CalibrationChanged(self.calibration.clone()));
    }

    fn ab_changed(&self) {
        self.hub.broadcast(&Event::AbChanged(self.ab.state()));
    }

    /// Calibration and the A/B recorder share the raw capture.
    fn capture_free(&self) -> Result<(), ErrorBody> {
        if self.recording.is_some() {
            Err(ErrorBody::new(ErrorCode::Busy, "calibration is running"))
        } else if self.ab.is_recording() {
            Err(ErrorBody::new(
                ErrorCode::Busy,
                "an A/B recording is running",
            ))
        } else {
            Ok(())
        }
    }

    fn respond_ab(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        match request {
            Request::AbRecord { max_seconds } => {
                self.capture_free()?;
                let control = connected(&mut self.control, self.input_connected)?;
                self.ab.record(control, max_seconds)?;
            }
            Request::AbStopRecording => {
                let control = connected(&mut self.control, self.input_connected)?;
                self.ab.stop_recording(control)?;
            }
            Request::AbPlay { device } => {
                // With the mixer on the default output is a channel; play to the real device.
                let device = device.or_else(|| {
                    self.config
                        .mixer
                        .enabled
                        .then(|| self.mixer.output_device(&self.config.mixer))
                        .flatten()
                });
                self.ab.play(&self.config.mic.chain, device)?
            }
            Request::AbStop => self.ab.stop(),
            Request::AbSelect { side } => self.ab.select(side),
            Request::AbToggle => self.ab.toggle(),
            Request::AbSeek { position_ms } => self.ab.seek(position_ms)?,
            Request::AbSetLoudnessMatch { enabled } => self.ab.set_loudness_match(enabled),
            Request::AbWaveform { points } => {
                let waveform = self.ab.waveform(&self.config.mic.chain, points)?;
                return serde_json::to_value(waveform)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()));
            }
            _ => {}
        }
        let state = self.ab.state();
        self.hub.broadcast(&Event::AbChanged(state.clone()));
        serde_json::to_value(state)
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }

    fn start_calibration(&mut self) -> Result<(), ErrorBody> {
        if self.ab.is_recording() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "an A/B recording is running",
            ));
        }
        if self.recording.is_some() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "calibration is already running",
            ));
        }
        let control = connected(&mut self.control, self.input_connected)?;
        self.recording = Some(Recording::start(control));
        self.set_calibration(Recording::initial_state());
        Ok(())
    }

    fn calibration_value(&self) -> Result<serde_json::Value, ErrorBody> {
        serde_json::to_value(&self.calibration)
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }

    fn fail_calibration(&mut self, reason: CalibrationFailure, message: impl Into<String>) {
        if let Some(control) = &mut self.control {
            control.stop_capture();
        }
        self.recording = None;
        let message = message.into();
        tracing::warn!(?reason, %message, "Calibration failed");
        self.set_calibration(CalibrationState::Failed { reason, message });
    }

    fn poll_calibration(&mut self) {
        let (Some(recording), Some(control)) = (&mut self.recording, &mut self.control) else {
            return;
        };
        match recording.poll(control) {
            Poll::Measuring(state) => self.set_calibration(state),
            Poll::Failed(reason, message) => self.fail_calibration(reason, message),
            Poll::Complete(samples) => {
                self.recording = None;
                self.finish_calibration(&samples);
            }
        }
    }

    /// CAL-003: analyse, then apply the calibrated chain like any other change.
    fn finish_calibration(&mut self, samples: &[f32]) {
        let (settings, report) =
            match macafon_dsp::calibration::calibrate(samples, &self.config.mic.chain) {
                Ok(result) => result,
                Err(error) => {
                    let reason = match error {
                        CalibrationError::Clipped { .. } => CalibrationFailure::Clipped,
                        CalibrationError::NoSpeech => CalibrationFailure::NoSpeech,
                        CalibrationError::TooShort => CalibrationFailure::Internal,
                    };
                    return self.fail_calibration(reason, error.to_string());
                }
            };
        let measured_device = self.active_device.clone();
        let detected_channel = self.detected_channel;
        let applied = self.update(|config| {
            config.mic.chain = settings;
            config.mic.noise_floor_dbfs = Some(report.noise_floor_dbfs);
            config.mic.noise_floor_device = measured_device;
            if config.mic.input_channel == macafon_dsp::InputChannel::Auto {
                config.mic.input_channel = detected_channel;
            }
        });
        match applied {
            Ok(_) => {
                tracing::info!(
                    noise_floor_dbfs = report.noise_floor_dbfs,
                    speech_rms_dbfs = report.speech_rms_dbfs,
                    snr_db = report.snr_db,
                    snr_after_db = report.snr_after_db,
                    gain_db = report.gain_db,
                    "Calibration applied"
                );
                self.set_calibration(CalibrationState::Done { report });
            }
            Err(error) => self.fail_calibration(CalibrationFailure::Internal, error.message),
        }
    }

    fn update_tray(&mut self) {
        if self.tray.is_none() {
            return;
        }
        let state = self.state();
        let mixer = self.mixer_state();
        let tray_state = TrayState {
            muted: state.settings.muted,
            bypass: state.settings.bypass,
            connected: state.input_connected,
            preset: state.preset.clone(),
            preset_modified: state.preset_modified,
            presets: self.preset_list(),
            mixer: mixer.enabled,
            outputs: mixer
                .outputs
                .into_iter()
                .map(|output| (output.name, output.description))
                .collect(),
            output: mixer.output,
        };
        if let Some(tray) = &mut self.tray {
            tray.update(tray_state);
        }
    }

    fn changed(&mut self) {
        let state = self.state();
        self.update_tray();
        self.hub.broadcast(&Event::MicChanged(state));
    }

    fn noise_fallback(&mut self, settings: macafon_dsp::ChainSettings) {
        self.config.mic.chain = settings;
        if let Some(id) = &self.selected_stable_id {
            self.config.device_profiles.insert(
                id.clone(),
                macafon_config::MicProfile::capture(&self.config.mic),
            );
        }
        self.ab.settings_changed(&settings);
        self.changed_at = Some(Instant::now());
        self.changed();
        self.overloads.push_back(Instant::now());
        self.hub.broadcast(&Event::NoiseFallback {
            engine: settings.noise_suppression.params.engine,
        });
    }

    /// Apply a new configuration to the running engine first; keep it only if accepted.
    fn update(&mut self, update: impl FnOnce(&mut Config)) -> Result<MicState, ErrorBody> {
        let mut config = self.config.clone();
        update(&mut config);
        config
            .mic
            .chain
            .validate()
            .map_err(|error| ErrorBody::new(ErrorCode::InvalidParams, error.to_string()))?;
        if let Some(control) = &mut self.control {
            control
                .apply(&config.mic.chain)
                .map_err(|error| match error {
                    EngineError::Invalid(error) => {
                        ErrorBody::new(ErrorCode::InvalidParams, error.to_string())
                    }
                    EngineError::Busy => ErrorBody::new(ErrorCode::Busy, error.to_string()),
                    EngineError::Build(_) | EngineError::Studio(_) => {
                        ErrorBody::new(ErrorCode::Internal, error.to_string())
                    }
                })?;
        }
        if config != self.config {
            if config.mic.chain != self.config.mic.chain {
                self.ab.settings_changed(&config.mic.chain);
            }
            if let Some(id) = &self.selected_stable_id {
                config
                    .device_profiles
                    .insert(id.clone(), macafon_config::MicProfile::capture(&config.mic));
            }
            self.config = config;
            self.changed_at = Some(Instant::now());
            self.changed();
        }
        Ok(self.state())
    }

    /// Select `id` as the current preset without touching the chain.
    fn mark_preset(&mut self, id: Option<String>) {
        if self.config.mic.preset != id {
            self.config.mic.preset = id;
            self.changed_at = Some(Instant::now());
        }
    }

    fn respond_presets(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        let selected = match request {
            Request::PresetsSave { name } => {
                let id = self
                    .presets
                    .create(&name, &self.config.mic.chain)
                    .map_err(preset_error)?
                    .id
                    .clone();
                self.mark_preset(Some(id.clone()));
                id
            }
            Request::PresetsUpdate { id } => {
                self.presets
                    .update(&id, &self.config.mic.chain)
                    .map_err(preset_error)?;
                self.mark_preset(Some(id.clone()));
                id
            }
            Request::PresetsRename { id, name } => {
                self.presets.rename(&id, &name).map_err(preset_error)?;
                id
            }
            Request::PresetsDuplicate { id } => {
                let (name, settings) = match Preset::from_id(&id) {
                    Some(preset) => (preset.name().to_owned(), preset.settings()),
                    None => self
                        .presets
                        .get(&id)
                        .map(|preset| (preset.name.clone(), preset.settings))
                        .ok_or_else(|| preset_error(PresetError::NotFound(id.clone())))?,
                };
                self.presets
                    .add_unique(&name, &settings)
                    .map_err(preset_error)?
                    .id
                    .clone()
            }
            Request::PresetsDelete { id } => {
                self.presets.delete(&id).map_err(preset_error)?;
                if self.config.mic.preset.as_deref() == Some(id.as_str()) {
                    self.mark_preset(None);
                }
                self.presets_changed();
                return Ok(serde_json::Value::Null);
            }
            Request::PresetsExport { id } => {
                let document = match Preset::from_id(&id) {
                    Some(preset) => presets::export(preset.name(), &preset.settings()),
                    None => self
                        .presets
                        .get(&id)
                        .map(|preset| presets::export(&preset.name, &preset.settings))
                        .ok_or_else(|| preset_error(PresetError::NotFound(id.clone())))?,
                };
                return serde_json::to_value(document)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()));
            }
            Request::PresetsImport { document } => {
                let file = presets::parse(&document.to_string()).map_err(preset_error)?;
                self.presets.import(&file).map_err(preset_error)?.id.clone()
            }
            _ => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "not a preset request",
                ));
            }
        };
        self.presets_changed();
        self.preset_info(&selected)
    }

    fn presets_changed(&mut self) {
        self.hub
            .broadcast(&Event::PresetsChanged(self.preset_list()));
        // Saving or editing a preset can change `preset` and `preset_modified`.
        self.changed();
    }

    fn mixer_state(&self) -> macafon_ipc::MixerState {
        self.mixer.state(&self.config)
    }

    fn mixer_changed(&mut self) {
        self.hub.broadcast(&Event::MixerChanged(self.mixer_state()));
        self.update_tray();
    }

    fn mixer_telemetry_changed(&self) {
        self.hub
            .set_mixer_telemetry(self.mixer.telemetry(&self.config.mixer));
    }

    /// Apply a mixer change to the live channels first; keep it only if accepted.
    fn update_mixer(
        &mut self,
        update: impl FnOnce(&mut Config) -> Result<(), ErrorBody>,
    ) -> Result<serde_json::Value, ErrorBody> {
        let mut config = self.config.clone();
        update(&mut config)?;
        config
            .mixer
            .validate()
            .map_err(|message| ErrorBody::new(ErrorCode::InvalidParams, message))?;
        self.mixer.sync(&config.mixer).map_err(engine_error)?;
        let was_enabled = self.config.mixer.enabled;
        for channel in &config.mixer.channels {
            if self
                .config
                .mixer
                .channel(&channel.id)
                .is_some_and(|old| old.name != channel.name)
            {
                self.mixer.rename(channel);
            }
        }
        if config != self.config {
            self.config = config;
            self.changed_at = Some(Instant::now());
        }
        if was_enabled && !self.config.mixer.enabled {
            // Give the default output back while the channels still exist (OUT-005).
            self.mixer.restore_default_sink();
        }
        self.mixer_telemetry_changed();
        self.update_tray();
        self.rebind_hotkeys();
        self.hotkeys_changed();
        let state = self.mixer_state();
        self.hub.broadcast(&Event::MixerChanged(state.clone()));
        serde_json::to_value(state)
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }

    fn respond_mixer(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        let missing = |id: &str| ErrorBody::new(ErrorCode::NotFound, format!("no channel {id:?}"));
        match request {
            Request::MixerGet => serde_json::to_value(self.mixer_state())
                .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string())),
            Request::MixerSetEnabled { enabled } => {
                tracing::info!(enabled, "Mixer switched");
                self.update_mixer(|config| {
                    config.mixer.enabled = enabled;
                    Ok(())
                })
            }
            Request::MixerSetMaster { volume_pct, muted } => self.update_mixer(|config| {
                if let Some(volume) = volume_pct {
                    config.mixer.master_volume_pct = volume;
                }
                if let Some(muted) = muted {
                    config.mixer.master_muted = muted;
                }
                Ok(())
            }),
            Request::MixerSetOutput { device } => self.update_mixer(|config| {
                let priority = &mut config.mixer.output_priority;
                match device {
                    Some(device) => {
                        priority.retain(|name| name != &device);
                        priority.insert(0, device);
                    }
                    None => priority.clear(),
                }
                Ok(())
            }),
            Request::MixerSetOutputPriority { devices } => self.update_mixer(|config| {
                config.mixer.output_priority = devices;
                Ok(())
            }),
            Request::MixerSetAutoClassify { enabled } => self.update_mixer(|config| {
                config.mixer.auto_classify = enabled;
                Ok(())
            }),
            Request::ChannelCreate { name, color, icon } => self.update_mixer(|config| {
                if config.mixer.channels.len() >= macafon_config::mixer::MAX_CHANNELS {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        format!("at most {} channels", macafon_config::mixer::MAX_CHANNELS),
                    ));
                }
                let id = config.mixer.next_id();
                let icon = icon.unwrap_or_else(|| macafon_config::mixer::CHANNEL_ICONS[0].into());
                config
                    .mixer
                    .channels
                    .push(macafon_config::ChannelConfig::new(
                        &id,
                        name.trim(),
                        &color,
                        &icon,
                        macafon_dsp::channel::ChannelTemplate::Custom,
                    ));
                Ok(())
            }),
            Request::ChannelUpdate { id, patch } => self.update_mixer(|config| {
                let channel = config.mixer.channel_mut(&id).ok_or_else(|| missing(&id))?;
                if let Some(name) = patch.name {
                    channel.name = name.trim().to_owned();
                }
                if let Some(color) = patch.color {
                    channel.color = color;
                }
                if let Some(icon) = patch.icon {
                    channel.icon = icon;
                }
                Ok(())
            }),
            Request::ChannelDelete { id } => self.update_mixer(|config| {
                if config.mixer.default_channel == id {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "the default channel cannot be deleted",
                    ));
                }
                config.mixer.channel(&id).ok_or_else(|| missing(&id))?;
                config.mixer.channels.retain(|channel| channel.id != id);
                // CH-002: its apps move to the default channel.
                let default = config.mixer.default_channel.clone();
                for rule in &mut config.apps.rules {
                    if rule.channel == id {
                        rule.channel.clone_from(&default);
                    }
                }
                Ok(())
            }),
            Request::ChannelSet { id, settings } => self.update_mixer(|config| {
                config
                    .mixer
                    .channel_mut(&id)
                    .ok_or_else(|| missing(&id))?
                    .settings = settings;
                Ok(())
            }),
            Request::ChannelAdjustVolume { id, delta_pct } => self.update_mixer(|config| {
                let settings = &mut config
                    .mixer
                    .channel_mut(&id)
                    .ok_or_else(|| missing(&id))?
                    .settings;
                settings.volume_pct = (settings.volume_pct + delta_pct).clamp(0.0, 100.0);
                Ok(())
            }),
            Request::ChannelToggleMute { id } => self.update_mixer(|config| {
                let settings = &mut config
                    .mixer
                    .channel_mut(&id)
                    .ok_or_else(|| missing(&id))?
                    .settings;
                settings.muted = !settings.muted;
                Ok(())
            }),
            Request::ChannelSetDefault { id } => self.update_mixer(|config| {
                config.mixer.channel(&id).ok_or_else(|| missing(&id))?;
                config.mixer.default_channel = id;
                Ok(())
            }),
            Request::ChannelReorder { ids } => self.update_mixer(|config| {
                let channels = &mut config.mixer.channels;
                let mut sorted = ids.clone();
                sorted.sort();
                let mut current: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
                current.sort();
                if sorted != current {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "the new order must list every channel once",
                    ));
                }
                channels.sort_by_key(|channel| ids.iter().position(|id| id == &channel.id));
                Ok(())
            }),
            Request::AppsMove { app_key, channel } => self.update_mixer(|config| {
                config
                    .mixer
                    .channel(&channel)
                    .ok_or_else(|| missing(&channel))?;
                crate::mixer::assign(&mut config.apps, &app_key, &channel);
                if let Some(rule) = config
                    .apps
                    .rules
                    .iter_mut()
                    .find(|rule| rule.key == app_key)
                {
                    rule.managed = true;
                }
                Ok(())
            }),
            Request::AppsSetManaged { app_key, managed } => self.update_mixer(|config| {
                let channel = config
                    .apps
                    .rule(&app_key)
                    .map(|rule| rule.channel.clone())
                    .unwrap_or_else(|| config.mixer.default_channel.clone());
                crate::mixer::assign(&mut config.apps, &app_key, &channel);
                if let Some(rule) = config
                    .apps
                    .rules
                    .iter_mut()
                    .find(|rule| rule.key == app_key)
                {
                    rule.managed = managed;
                }
                Ok(())
            }),
            Request::AppsForget { app_key } => self.update_mixer(|config| {
                config.apps.rules.retain(|rule| rule.key != app_key);
                Ok(())
            }),
            _ => Err(ErrorBody::new(ErrorCode::BadRequest, "not a mixer request")),
        }
    }

    // --- Hotkeys (§8.3) ------------------------------------------------------------------------

    /// Every action available now: the global ones and three per channel.
    fn hotkey_actions(&self) -> Vec<(HotkeyAction, String)> {
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

    fn rebind_hotkeys(&self) {
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

    fn hotkeys_state(&self) -> HotkeysState {
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

    fn hotkeys_changed(&self) {
        self.hub
            .broadcast(&Event::HotkeysChanged(self.hotkeys_state()));
    }

    /// HK-002: canonical shortcut, unique within Macafon.
    fn set_hotkey(&mut self, action: &str, shortcut: Option<String>) -> Result<(), ErrorBody> {
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
    fn hotkey(&mut self, action: &str, pressed: bool) {
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

    fn cycle_preset(&mut self, step: i32) -> Result<MicState, ErrorBody> {
        let presets = self.preset_list();
        if presets.is_empty() {
            return Err(ErrorBody::new(ErrorCode::NotFound, "no presets"));
        }
        let current = self
            .config
            .mic
            .preset
            .as_deref()
            .and_then(|id| presets.iter().position(|preset| preset.id == id));
        let count = presets.len() as i32;
        let index = match current {
            Some(index) => (index as i32 + step).rem_euclid(count),
            None if step >= 0 => 0,
            None => count - 1,
        };
        let preset = presets[index as usize].id.clone();
        let settings = self
            .preset_settings(&preset)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "preset vanished"))?;
        self.update(|config| {
            let mic = &mut config.mic;
            let (muted, bypass) = (mic.chain.muted, mic.chain.bypass);
            mic.chain = settings;
            mic.chain.muted = muted;
            mic.chain.bypass = bypass;
            mic.preset = Some(preset);
        })
    }

    fn respond(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        let state = match request {
            Request::MicCyclePreset { step } => self.cycle_preset(step)?,
            Request::HotkeysGet => return Ok(serde_json::json!(self.hotkeys_state())),
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
                let journal =
                    macafon_backend_pipewire::default_source_journal().map_err(backend_error)?;
                macafon_backend_pipewire::set_default_source(&journal).map_err(backend_error)?;
                return Ok(serde_json::Value::Null);
            }
            Request::MicRestoreDefaultSource => {
                let journal =
                    macafon_backend_pipewire::default_source_journal().map_err(backend_error)?;
                macafon_backend_pipewire::restore_default_source(&journal)
                    .map_err(backend_error)?;
                return Ok(serde_json::Value::Null);
            }
            Request::AbExport { side, path } => {
                self.ab
                    .export(&self.config.mic.chain, side, std::path::Path::new(&path))?;
                return Ok(serde_json::Value::Null);
            }
            Request::AbOutputs => {
                let nodes = macafon_backend_pipewire::list_nodes().map_err(backend_error)?;
                let outputs: Vec<macafon_ipc::PlaybackDevice> = nodes
                    .into_iter()
                    .filter(|node| {
                        node.media_class == "Audio/Sink"
                            && !node
                                .name
                                .starts_with(macafon_backend_pipewire::CHANNEL_PREFIX)
                    })
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
            Request::AutostartGet => return Ok(serde_json::json!(crate::autostart::state())),
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
            | Request::AppsForget { .. }) => return self.respond_mixer(request),
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
                let report = crate::doctor::report(crate::doctor::Facts {
                    inspection: macafon_backend_pipewire::inspect()
                        .map_err(|error| error.to_string()),
                    mic: &mic,
                    mixer: &mixer,
                    clock: self.clock,
                    overloads: self.overloads.len(),
                    inputs: &self.inputs,
                    hotkeys: self
                        .hotkey_status
                        .as_ref()
                        .map_or(HotkeyBackend::Unavailable, |status| status.backend),
                });
                return serde_json::to_value(report)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()));
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

fn engine_error(error: EngineError) -> ErrorBody {
    match error {
        EngineError::Invalid(error) => ErrorBody::new(ErrorCode::InvalidParams, error.to_string()),
        EngineError::Busy => ErrorBody::new(ErrorCode::Busy, error.to_string()),
        EngineError::Build(_) | EngineError::Studio(_) => {
            ErrorBody::new(ErrorCode::Internal, error.to_string())
        }
    }
}

fn backend_error(error: Error) -> ErrorBody {
    ErrorBody::new(ErrorCode::Internal, error.to_string())
}

fn preset_error(error: PresetError) -> ErrorBody {
    let code = match &error {
        PresetError::DuplicateName(_) => ErrorCode::AlreadyExists,
        PresetError::NotFound(_) => ErrorCode::NotFound,
        PresetError::Storage(_) => ErrorCode::Internal,
        PresetError::InvalidName
        | PresetError::Malformed(_)
        | PresetError::BuiltIn
        | PresetError::UnsupportedVersion(_)
        | PresetError::Invalid(_) => ErrorCode::InvalidParams,
    };
    ErrorBody::new(code, error.to_string())
}

/// The live microphone engine, if the selected microphone is connected.
fn connected(
    control: &mut Option<MicControl>,
    input_connected: bool,
) -> Result<&mut MicControl, ErrorBody> {
    control.as_mut().filter(|_| input_connected).ok_or_else(|| {
        ErrorBody::new(
            ErrorCode::NotFound,
            "the selected microphone is not connected",
        )
    })
}

impl MixerHost for DaemonHost {
    fn channel_requests(&mut self) -> Vec<ChannelRequest> {
        self.mixer.take_requests()
    }

    fn outputs_changed(&mut self, outputs: &[Node]) {
        self.mixer.outputs_changed(outputs);
        self.mixer_changed();
    }

    fn default_sink_changed(&mut self, name: Option<&str>) {
        self.mixer.default_sink_changed(name);
    }

    fn output_device(&self) -> Option<String> {
        self.mixer.output_device(&self.config.mixer)
    }

    fn streams_changed(&mut self, streams: &[PlaybackStream]) {
        self.mixer.streams_changed(streams);
        self.mixer_changed();
    }

    fn route(&self, stream: &PlaybackStream) -> Route {
        self.mixer.route(&self.config, stream)
    }
}

impl MicrophoneHost for DaemonHost {
    type Message = ControlMessage;

    fn started(&mut self) -> Result<(), Error> {
        if let Some((path, control)) = self.server.take() {
            if self.tray_wanted {
                // Spawned after signals are blocked, like the IPC threads.
                self.tray = Tray::spawn(TrayState::default(), control.clone());
                self.changed();
            }
            // Spawned after signals are blocked, like the tray (HK-003).
            self.hotkeys = Some(crate::hotkeys::Hotkeys::spawn(control.clone()));
            self.rebind_hotkeys();
            let socket = server::start(&path, self.hub.clone(), control)
                .map_err(|error| Error::Message(format!("{error:#}")))?;
            self._socket = Some(socket);
        }
        Ok(())
    }

    fn start_session(&mut self) -> Result<MicRt, Error> {
        if let Ok(journal) = macafon_backend_pipewire::default_source_journal() {
            if let Err(error) = macafon_backend_pipewire::restore_default_source(&journal) {
                tracing::warn!(%error, "Cannot recover previous default microphone yet");
            }
        }
        let (control, rt) =
            macafon_engine::microphone(&self.config.mic.chain, self.telemetry.clone())?;
        self.control = Some(control);
        self.mixer.start_session(&self.config.mixer)?;
        self.mixer_telemetry_changed();
        Ok(rt)
    }

    fn end_session(&mut self) {
        if self.recording.is_some() {
            self.fail_calibration(
                CalibrationFailure::Disconnected,
                "PipeWire restarted during calibration",
            );
        }
        if self.ab.is_active() {
            self.ab.abort_recording(None);
            self.ab.player_lost();
            self.ab_changed();
        }
        self.control = None;
        self.clock = None;
        self.mixer.end_session();
        self.mixer_telemetry_changed();
        if self.input_connected {
            self.input_connected = false;
            self.active_device = None;
            self.changed();
        }
    }

    fn input(&self) -> Option<String> {
        self.selected_device.clone()
    }

    fn graph_clock(&mut self, rate_hz: u32, quantum: u32) {
        self.clock = (rate_hz != 0 && quantum != 0).then_some((rate_hz, quantum));
    }

    fn input_channel_detected(&mut self, channel: macafon_dsp::InputChannel) {
        self.detected_channel = channel;
    }

    fn input_channel(&self) -> macafon_dsp::InputChannel {
        self.config.mic.input_channel
    }

    fn stereo(&self) -> bool {
        self.config.mic.stereo
    }

    fn latency_samples(&self) -> u32 {
        self.config.mic.chain.latency_samples()
    }

    fn keep_processing(&self) -> bool {
        self.recording.is_some() || self.ab.is_recording() || self.hub.metering()
    }

    fn player_request(&mut self) -> Option<PlayerRequest> {
        self.ab.take_request()
    }

    fn player_failed(&mut self, error: Error) {
        tracing::warn!(%error, "Cannot start A/B playback");
        self.ab.player_lost();
        self.ab_changed();
    }

    fn input_connected(&mut self, connected: bool) {
        if connected != self.input_connected {
            self.input_connected = connected;
            self.active_device = connected.then(|| self.selected_device.clone()).flatten();
            self.changed();
            if !connected && self.recording.is_some() {
                self.fail_calibration(
                    CalibrationFailure::Disconnected,
                    "the microphone disconnected during calibration",
                );
            }
            if !connected && self.ab.is_recording() {
                self.ab.abort_recording(self.control.as_mut());
                self.ab_changed();
            }
        }
    }

    fn device_metering(&self) -> bool {
        self.hub.device_metering()
    }
    fn device_meter(&mut self, name: &str, peak_dbfs: f32) {
        self.hub.device_meter(name, peak_dbfs);
    }

    fn inputs_changed(&mut self, inputs: &[Node]) {
        self.inputs = inputs
            .iter()
            .map(|node| Device {
                name: node.name.clone(),
                description: node.description.clone(),
                stable_id: node.stable_id.clone(),
                kind: match node.kind {
                    "usb" => macafon_ipc::DeviceKind::Usb,
                    "builtin" => macafon_ipc::DeviceKind::Builtin,
                    "headset" => macafon_ipc::DeviceKind::Headset,
                    "bluetooth" => macafon_ipc::DeviceKind::Bluetooth,
                    _ => macafon_ipc::DeviceKind::Unknown,
                },
            })
            .collect();
        self.refresh_input();
    }

    fn handle(&mut self, message: ControlMessage) {
        let message = match message {
            ControlMessage::Hotkey { action, pressed } => return self.hotkey(&action, pressed),
            ControlMessage::HotkeyStatus(status) => {
                self.hotkey_status = Some(status);
                return self.hotkeys_changed();
            }
            message => message,
        };
        let ControlMessage::Request { request, reply } = message else {
            self.ab.clear(self.control.as_mut());
            self.ab_changed();
            // BG-001: the window closed, the sound keeps going; say so once.
            if !self.quit && !self.config.ui.close_hint_shown {
                self.config.ui.close_hint_shown = true;
                self.changed_at = Some(Instant::now());
                crate::notify::send(
                    "Macafon keeps running in the tray",
                    "Your microphone and mixer stay on. Open Macafon from the tray icon.",
                );
            }
            return;
        };
        let outcome = match self.respond(request) {
            Ok(value) => Outcome::Result(value),
            Err(error) => Outcome::Error(error),
        };
        // The client may have disconnected meanwhile; nothing to do then.
        let _ = reply.send(outcome);
    }

    fn quit_requested(&self) -> bool {
        self.quit
    }

    fn needs_processing_budget(&self) -> bool {
        self.config.mic.chain.noise_suppression.enabled
            && self.config.mic.chain.noise_suppression.params.engine
                == macafon_dsp::settings::NoiseEngine::Studio
    }

    fn processing_budget(&mut self, elapsed_ns: u64, budget_ns: u64) {
        if !self.needs_processing_budget() {
            return;
        }
        let fallback = self.control.as_mut().map(|control| {
            control
                .observe_processing_budget(elapsed_ns, budget_ns)
                .map(|changed| changed.then(|| *control.settings()))
        });
        match fallback {
            Some(Ok(Some(settings))) => self.noise_fallback(settings),
            Some(Err(error)) => tracing::warn!(%error, "Cannot apply processing budget fallback"),
            _ => {}
        }
    }

    fn tick(&mut self) {
        self.refresh_input();
        self.mixer.collect_garbage();
        self.mixer.poll_default_sink(&self.config.mixer);
        if let Some(control) = &mut self.control {
            control.collect_garbage();
        }
        let fallback = self.control.as_mut().map(|control| {
            control
                .poll_studio_fallback()
                .map(|changed| changed.then(|| *control.settings()))
        });
        match fallback {
            Some(Ok(Some(settings))) => self.noise_fallback(settings),
            Some(Err(error)) => tracing::warn!(%error, "Cannot apply noise suppression fallback"),
            _ => {}
        }
        self.poll_calibration();
        if let Some(control) = &mut self.control {
            if self.ab.poll(control) || self.ab.is_active() {
                self.ab_changed();
            }
        }
        if self
            .changed_at
            .is_some_and(|changed| changed.elapsed() >= SAVE_DEBOUNCE)
        {
            self.changed_at = None;
            self.save();
        }
    }
}

/// A missing primary selection explicitly disables both inputs.
fn select_input<'a>(mic: &'a macafon_config::MicConfig, inputs: &[Device]) -> Option<&'a str> {
    let primary = mic.device.as_deref()?;
    let present = |name: &str| inputs.iter().any(|input| input.name == name);
    if present(primary) {
        Some(primary)
    } else {
        mic.fallback_device.as_deref().filter(|name| present(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_budget_fallback_updates_settings_and_device_profile() {
        let (sender, _receiver) = macafon_backend_pipewire::control_channel();
        let mut config = Config::default();
        config.mic.chain = macafon_dsp::Preset::NoisyRoom.settings();
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            PathBuf::from("unused"),
            sender,
            false,
        );
        let (control, _rt) =
            macafon_engine::microphone(&host.config.mic.chain, host.telemetry.clone())
                .expect("embedded Studio initializes");
        host.control = Some(control);
        host.selected_stable_id = Some("test-device".into());
        host.processing_budget(801, 1000);
        host.processing_budget(801, 1000);
        assert!(host.needs_processing_budget());
        // A cycle at exactly 80% interrupts the streak.
        host.processing_budget(800, 1000);
        host.processing_budget(900, 1000);
        host.processing_budget(900, 1000);
        assert!(host.needs_processing_budget());
        host.processing_budget(900, 1000);
        assert!(!host.needs_processing_budget());
        assert_eq!(
            host.config.mic.chain.noise_suppression.params.engine,
            macafon_dsp::settings::NoiseEngine::Lite
        );
        assert_eq!(
            host.config.device_profiles["test-device"].chain,
            host.config.mic.chain
        );
        assert!(host.changed_at.is_some());
    }

    #[test]
    fn input_switch_invalidates_measurement_and_tracks_actual_connection() {
        let (sender, _receiver) = macafon_backend_pipewire::control_channel();
        let mut config = Config::default();
        config.mic.device = Some("primary".into());
        config.mic.fallback_device = Some("fallback".into());
        config.mic.noise_floor_dbfs = Some(-60.0);
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            PathBuf::from("unused"),
            sender,
            false,
        );
        host.inputs = vec![
            Device {
                name: "primary".into(),
                description: "Main".into(),
                stable_id: String::new(),
                kind: macafon_ipc::DeviceKind::Unknown,
            },
            Device {
                name: "fallback".into(),
                description: "Backup".into(),
                stable_id: String::new(),
                kind: macafon_ipc::DeviceKind::Unknown,
            },
        ];
        host.refresh_input();
        host.input_connected(true);
        assert_eq!(host.state().active_device.as_deref(), Some("primary"));
        assert_eq!(host.state().noise_floor_dbfs, Some(-60.0));
        host.inputs.remove(0);
        host.refresh_input();
        assert_eq!(host.input().as_deref(), Some("fallback"));
        assert_eq!(host.state().active_device, None);
        assert!(!host.state().input_connected);
        assert_eq!(host.state().noise_floor_dbfs, None);
        host.input_connected(true);
        assert_eq!(host.state().active_device.as_deref(), Some("fallback"));
        host.inputs.push(Device {
            name: "primary".into(),
            description: "Main".into(),
            stable_id: String::new(),
            kind: macafon_ipc::DeviceKind::Unknown,
        });
        host.refresh_input();
        host.input_connected(true);
        assert_eq!(host.state().active_device.as_deref(), Some("primary"));
        assert_eq!(host.config.mic.device.as_deref(), Some("primary"));
        assert_eq!(host.state().noise_floor_dbfs, Some(-60.0));
        assert_eq!(host.config.device_profiles.len(), 2);
    }

    #[test]
    fn startup_floor_belongs_to_measured_device_and_legacy_defaults_to_primary() {
        for measured in [None, Some("primary"), Some("fallback")] {
            let (sender, _receiver) = macafon_backend_pipewire::control_channel();
            let config = Config {
                mic: macafon_config::MicConfig {
                    device: Some("primary".into()),
                    fallback_device: Some("fallback".into()),
                    noise_floor_dbfs: Some(-60.0),
                    noise_floor_device: measured.map(str::to_owned),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut host = DaemonHost::new(
                config,
                None,
                PresetStore::in_memory(),
                PathBuf::from("unused"),
                sender,
                false,
            );
            host.inputs = vec![Device {
                name: "primary".into(),
                description: "Main".into(),
                stable_id: String::new(),
                kind: macafon_ipc::DeviceKind::Unknown,
            }];
            host.refresh_input();
            if measured == Some("fallback") {
                assert_eq!(host.state().noise_floor_dbfs, None);
                assert_eq!(host.config.mic.noise_floor_device, None);
                assert!(host.changed_at.is_some());
            } else {
                assert_eq!(host.state().noise_floor_dbfs, Some(-60.0));
                assert_eq!(
                    host.config.mic.noise_floor_device.as_deref(),
                    Some("primary")
                );
            }
        }
    }

    #[test]
    fn microphone_priority_restores_primary_and_none_disables_fallback() {
        let mut mic = macafon_config::MicConfig {
            device: Some("primary".into()),
            fallback_device: Some("fallback".into()),
            ..Default::default()
        };
        let fallback = Device {
            name: "fallback".into(),
            description: "Backup".into(),
            stable_id: String::new(),
            kind: macafon_ipc::DeviceKind::Unknown,
        };
        let primary = Device {
            name: "primary".into(),
            description: "Main".into(),
            stable_id: String::new(),
            kind: macafon_ipc::DeviceKind::Unknown,
        };
        assert_eq!(select_input(&mic, &[]), None);
        assert_eq!(
            select_input(&mic, std::slice::from_ref(&fallback)),
            Some("fallback")
        );
        assert_eq!(
            select_input(&mic, &[fallback.clone(), primary]),
            Some("primary")
        );
        assert_eq!(mic.device.as_deref(), Some("primary"));
        mic.device = None;
        assert_eq!(select_input(&mic, &[fallback]), None);
    }
}
