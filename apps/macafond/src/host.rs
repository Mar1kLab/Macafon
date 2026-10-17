//! Daemon state on the PipeWire control loop: the desired microphone configuration, the live
//! engine of the current graph session and config persistence.

mod capture;
mod hotkeys;
mod microphone;
mod mixer;
mod presets;
mod requests;

use crate::ab::Ab;
use crate::calibration::Recording;
use crate::mixer::Mixer;
use crate::server::{self, Hub, SocketGuard};
use crate::tray::{Tray, TrayState};
use macafon_backend::ControlSender;
use macafon_backend::{
    AppMix, ChannelRequest, Error, MicRt, MicrophoneHost, MixerHost, Node, PlaybackStream,
    PlayerRequest, Route,
};
use macafon_config::presets::{PresetError, PresetStore};
use macafon_config::{Config, Store};
use macafon_dsp::Telemetry;
use macafon_engine::{EngineError, MicControl};
use macafon_ipc::{
    CalibrationFailure, CalibrationState, Device, Endpoint, ErrorBody, ErrorCode, Event, Outcome,
    Platform, Request,
};
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
    /// Processed sound reaches an app-visible microphone (WIN-MIC-001).
    virtual_mic_ready: bool,
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
    /// `default.audio.source` as PipeWire reports it (MIC-012).
    default_source: Option<String>,
    /// Noise suppression overload fallbacks, for the Doctor's 10-minute window.
    overloads: std::collections::VecDeque<Instant>,
    hub: Arc<Hub>,
    tray: Option<Tray>,
    tray_wanted: bool,
    hotkeys: Option<crate::hotkeys::Hotkeys>,
    hotkey_status: Option<crate::hotkeys::Status>,
    /// What the hotkey service registered last.
    bound_hotkeys: Option<Vec<crate::hotkeys::Binding>>,
    icons: macafon_apps::icons::IconResolver,
    quit: bool,
    server: Option<(Endpoint, ControlSender<ControlMessage>)>,
    _socket: Option<SocketGuard>,
}

impl DaemonHost {
    pub fn new(
        mut config: Config,
        store: Option<Store>,
        presets: PresetStore,
        endpoint: Endpoint,
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
            // PipeWire: the daemon creates the device itself. Windows: reported by the scan.
            virtual_mic_ready: Platform::current().capabilities().virtual_mic_device,
            recording: None,
            calibration: CalibrationState::Idle,
            detected_channel: macafon_dsp::InputChannel::Left,
            ab: Ab::default(),
            mixer: Mixer::default(),
            clock: None,
            default_source: None,
            overloads: std::collections::VecDeque::new(),
            hub,
            tray: None,
            tray_wanted: tray,
            hotkeys: None,
            hotkey_status: None,
            bound_hotkeys: None,
            icons: macafon_apps::icons::IconResolver::from_env(),
            quit: false,
            server: Some((endpoint, control)),
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

fn backend_error(error: impl std::fmt::Display) -> ErrorBody {
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

    fn quantum_frames(&self) -> u32 {
        self.config.audio.quantum_frames
    }

    fn default_source_changed(&mut self, name: Option<&str>) {
        self.default_source = name.map(str::to_owned);
        self.changed();
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

    fn stream_volume(&self, stream: &PlaybackStream) -> f32 {
        self.mixer.stream_volume(&self.config, stream)
    }

    fn mixer_enabled(&self) -> bool {
        self.config.mixer.enabled
    }

    fn app_mix(&self, stream: &PlaybackStream) -> Option<AppMix> {
        self.mixer.app_mix(&self.config, stream)
    }

    fn app_volume_changed(&mut self, stream: &PlaybackStream, volume_pct: f32) {
        let Some((key, channel)) = self.mixer.app_of(&self.config, stream) else {
            return;
        };
        let volume_pct = volume_pct.clamp(0.0, 100.0);
        if self
            .config
            .apps
            .rule(&key)
            .is_some_and(|rule| (rule.volume_pct - volume_pct).abs() < 0.5)
        {
            return;
        }
        crate::mixer::assign(&mut self.config.apps, &key, &channel);
        if let Some(rule) = self
            .config
            .apps
            .rules
            .iter_mut()
            .find(|rule| rule.key == key)
        {
            rule.volume_pct = volume_pct;
        }
        tracing::debug!(app = %key, volume_pct, "App volume changed outside Macafon");
        self.changed_at = Some(Instant::now());
        self.mixer_changed();
    }

    fn stream_peaks(&mut self, peaks: &[(u32, f32)]) {
        self.mixer.stream_peaks(&self.config, peaks);
    }
}

impl MicrophoneHost for DaemonHost {
    type Message = ControlMessage;

    fn started(&mut self) -> Result<(), Error> {
        if let Some((endpoint, control)) = self.server.take() {
            if self.tray_wanted {
                // Spawned after signals are blocked, like the IPC threads.
                self.tray = Tray::spawn(TrayState::default(), control.clone());
                self.changed();
            }
            // Spawned after signals are blocked, like the tray (HK-003).
            self.hotkeys = Some(crate::hotkeys::Hotkeys::spawn(control.clone()));
            if self
                .config
                .offer_default_hotkeys(crate::hotkeys::usable_by_default)
            {
                tracing::info!("Default hotkeys offered");
                self.changed_at = Some(Instant::now());
            }
            self.rebind_hotkeys();
            let socket = server::start(&endpoint, self.hub.clone(), control)
                .map_err(|error| Error::Message(format!("{error:#}")))?;
            self._socket = Some(socket);
        }
        Ok(())
    }

    fn start_session(&mut self) -> Result<MicRt, Error> {
        if let Err(error) = crate::platform::restore_default_source() {
            tracing::warn!(%error, "Cannot recover previous default microphone yet");
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

    fn virtual_mic_changed(&mut self, ready: bool) {
        if ready != self.virtual_mic_ready {
            self.virtual_mic_ready = ready;
            self.changed();
        }
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
        let inputs: Vec<Device> = inputs
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
        if inputs != self.inputs {
            self.hub.broadcast(&Event::DevicesChanged(inputs.clone()));
            self.inputs = inputs;
        }
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

    /// Never bound: these hosts are not started.
    fn unused_endpoint() -> Endpoint {
        #[cfg(unix)]
        return Endpoint::socket("unused");
        #[cfg(windows)]
        Endpoint::pipe("unused")
    }

    #[test]
    fn mixer_changes_do_not_rebind_hotkeys_and_release_push_to_talk() {
        let (sender, _receiver) = crate::platform::control_channel();
        let mut config = Config::default();
        config.mixer.enabled = true;
        config
            .hotkeys
            .insert("channel_volume_up.main".into(), "Ctrl+Alt+W".into());
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
            sender,
            false,
        );
        let (service, bound) = crate::hotkeys::Hotkeys::fake();
        host.hotkeys = Some(service);
        host.rebind_hotkeys();
        assert_eq!(bound.try_iter().count(), 1);
        host.hotkey("channel_volume_up.main", true);
        host.respond(Request::ChannelToggleMute { id: "voice".into() })
            .expect("mute");
        assert_eq!(bound.try_iter().count(), 0, "nothing to register again");
        // A renamed channel changes the descriptions the desktop shows.
        host.respond(Request::ChannelUpdate {
            id: "main".into(),
            patch: macafon_ipc::ChannelPatch {
                name: Some("Games".into()),
                ..Default::default()
            },
        })
        .expect("rename");
        assert_eq!(bound.try_iter().count(), 1);
    }

    #[test]
    fn graph_budget_fallback_updates_settings_and_device_profile() {
        let (sender, _receiver) = crate::platform::control_channel();
        let mut config = Config::default();
        config.mic.chain = macafon_dsp::Preset::NoisyRoom.settings();
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
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
        let (sender, _receiver) = crate::platform::control_channel();
        let mut config = Config::default();
        config.mic.device = Some("primary".into());
        config.mic.fallback_device = Some("fallback".into());
        config.mic.noise_floor_dbfs = Some(-60.0);
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
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
            let (sender, _receiver) = crate::platform::control_channel();
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
                unused_endpoint(),
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
