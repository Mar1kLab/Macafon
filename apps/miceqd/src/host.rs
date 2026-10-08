//! Daemon state on the PipeWire control loop: the desired microphone configuration, the live
//! engine of the current graph session and config persistence.

use crate::server::{self, Hub, SocketGuard};
use miceq_backend_pipewire::ControlSender;
use miceq_backend_pipewire::{Error, MicRt, MicrophoneHost, Node};
use miceq_config::{Config, Store};
use miceq_dsp::{Preset, Telemetry};
use miceq_engine::{EngineError, MicControl};
use miceq_ipc::{Device, ErrorBody, ErrorCode, Event, MicState, Outcome, PresetInfo, Request};
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
            Request::MicSetDevice { device } => self.update(|config| config.mic.device = device)?,
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

    fn input_connected(&mut self, connected: bool) {
        if connected != self.input_connected {
            self.input_connected = connected;
            self.changed();
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
        if self
            .changed_at
            .is_some_and(|changed| changed.elapsed() >= SAVE_DEBOUNCE)
        {
            self.changed_at = None;
            self.save();
        }
    }
}
