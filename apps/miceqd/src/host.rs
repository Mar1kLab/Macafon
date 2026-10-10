//! Daemon state on the PipeWire control loop: the desired microphone configuration, the live
//! engine of the current graph session and config persistence.

use crate::calibration::{Poll, Recording};
use crate::server::{self, Hub, SocketGuard};
use miceq_backend_pipewire::ControlSender;
use miceq_backend_pipewire::{Error, MicRt, MicrophoneHost, Node};
use miceq_config::{Config, Store};
use miceq_dsp::calibration::CalibrationError;
use miceq_dsp::{Preset, Telemetry};
use miceq_engine::{EngineError, MicControl};
use miceq_ipc::{
    CalibrationFailure, CalibrationState, Device, ErrorBody, ErrorCode, Event, MicState, Outcome,
    PresetInfo, Request,
};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// CFG-001: write the config 500 ms after the last change.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

pub struct ControlMessage {
    pub request: Request,
    pub reply: mpsc::Sender<Outcome>,
}

pub struct DaemonHost {
    config: Config,
    store: Option<Store>,
    changed_at: Option<Instant>,
    control: Option<MicControl>,
    telemetry: Arc<Telemetry>,
    inputs: Vec<Device>,
    input_connected: bool,
    recording: Option<Recording>,
    calibration: CalibrationState,
    hub: Arc<Hub>,
    server: Option<(PathBuf, ControlSender<ControlMessage>)>,
    _socket: Option<SocketGuard>,
}

impl DaemonHost {
    pub fn new(
        config: Config,
        store: Option<Store>,
        socket: PathBuf,
        control: ControlSender<ControlMessage>,
    ) -> Self {
        let hub = Hub::new();
        Self {
            config,
            store,
            changed_at: None,
            control: None,
            telemetry: hub.telemetry(),
            inputs: Vec::new(),
            input_connected: false,
            recording: None,
            calibration: CalibrationState::Idle,
            hub,
            server: Some((socket, control)),
            _socket: None,
        }
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

    fn state(&self) -> MicState {
        MicState {
            settings: self.config.mic.chain,
            preset: self.config.mic.preset.clone(),
            device: self.config.mic.device.clone(),
            input_connected: self.input_connected,
            latency_samples: self.config.mic.chain.latency_samples(),
            noise_floor_dbfs: self.config.mic.noise_floor_dbfs,
        }
    }

    fn set_calibration(&mut self, state: CalibrationState) {
        self.calibration = state;
        self.hub
            .broadcast(&Event::CalibrationChanged(self.calibration.clone()));
    }

    fn start_calibration(&mut self) -> Result<(), ErrorBody> {
        if self.recording.is_some() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "calibration is already running",
            ));
        }
        let control = self
            .control
            .as_mut()
            .filter(|_| self.input_connected)
            .ok_or_else(|| {
                ErrorBody::new(
                    ErrorCode::NotFound,
                    "the selected microphone is not connected",
                )
            })?;
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
            match miceq_dsp::calibration::calibrate(samples, &self.config.mic.chain) {
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
        let applied = self.update(|config| {
            config.mic.chain = settings;
            config.mic.noise_floor_dbfs = Some(report.noise_floor_dbfs);
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

    fn changed(&mut self) {
        self.hub.broadcast(&Event::MicChanged(self.state()));
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
                })?;
        }
        if config != self.config {
            self.config = config;
            self.changed_at = Some(Instant::now());
            self.changed();
        }
        Ok(self.state())
    }

    fn respond(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        let state = match request {
            Request::MicGet => self.state(),
            Request::MicSet { settings } => self.update(|config| config.mic.chain = settings)?,
            Request::MicSetPreset { preset } => {
                let preset = Preset::from_id(&preset).ok_or_else(|| {
                    ErrorBody::new(ErrorCode::NotFound, format!("unknown preset {preset:?}"))
                })?;
                self.update(|config| {
                    let mic = &mut config.mic;
                    let (muted, bypass) = (mic.chain.muted, mic.chain.bypass);
                    mic.chain = preset.settings();
                    mic.chain.muted = muted;
                    mic.chain.bypass = bypass;
                    mic.preset = Some(preset.id().to_owned());
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
                if device != self.config.mic.device && self.recording.is_some() {
                    self.fail_calibration(
                        CalibrationFailure::Disconnected,
                        "the microphone changed during calibration",
                    );
                }
                self.update(|config| {
                    // The measured floor belongs to the previous microphone.
                    if config.mic.device != device {
                        config.mic.noise_floor_dbfs = None;
                    }
                    config.mic.device = device;
                })?
            }
            Request::PresetsList => {
                let presets: Vec<PresetInfo> = Preset::ALL
                    .into_iter()
                    .map(|preset| PresetInfo {
                        id: preset.id().to_owned(),
                        name: preset.name().to_owned(),
                    })
                    .collect();
                return Ok(serde_json::json!(presets));
            }
            Request::DevicesList => return Ok(serde_json::json!(self.inputs)),
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
            Request::Hello { .. }
            | Request::MetersSubscribe { .. }
            | Request::MetersUnsubscribe => {
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

impl MicrophoneHost for DaemonHost {
    type Message = ControlMessage;

    fn started(&mut self) -> Result<(), Error> {
        if let Some((path, control)) = self.server.take() {
            let socket = server::start(&path, self.hub.clone(), control)
                .map_err(|error| Error::Message(format!("{error:#}")))?;
            self._socket = Some(socket);
        }
        Ok(())
    }

    fn start_session(&mut self) -> Result<MicRt, Error> {
        let (control, rt) =
            miceq_engine::microphone(&self.config.mic.chain, self.telemetry.clone())?;
        self.control = Some(control);
        Ok(rt)
    }

    fn end_session(&mut self) {
        if self.recording.is_some() {
            self.fail_calibration(
                CalibrationFailure::Disconnected,
                "PipeWire restarted during calibration",
            );
        }
        self.control = None;
        if self.input_connected {
            self.input_connected = false;
            self.changed();
        }
    }

    fn input(&self) -> Option<String> {
        self.config.mic.device.clone()
    }

    fn latency_samples(&self) -> u32 {
        self.config.mic.chain.latency_samples()
    }

    fn keep_processing(&self) -> bool {
        self.recording.is_some() || self.hub.metering()
    }

    fn input_connected(&mut self, connected: bool) {
        if connected != self.input_connected {
            self.input_connected = connected;
            self.changed();
            if !connected && self.recording.is_some() {
                self.fail_calibration(
                    CalibrationFailure::Disconnected,
                    "the microphone disconnected during calibration",
                );
            }
        }
    }

    fn inputs_changed(&mut self, inputs: &[Node]) {
        self.inputs = inputs
            .iter()
            .map(|node| Device {
                name: node.name.clone(),
                description: node.description.clone(),
            })
            .collect();
    }

    fn handle(&mut self, message: ControlMessage) {
        let outcome = match self.respond(message.request) {
            Ok(value) => Outcome::Result(value),
            Err(error) => Outcome::Error(error),
        };
        // The client may have disconnected meanwhile; nothing to do then.
        let _ = message.reply.send(outcome);
    }

    fn tick(&mut self) {
        if let Some(control) = &mut self.control {
            control.collect_garbage();
        }
        self.poll_calibration();
        if self
            .changed_at
            .is_some_and(|changed| changed.elapsed() >= SAVE_DEBOUNCE)
        {
            self.changed_at = None;
            self.save();
        }
    }
}
