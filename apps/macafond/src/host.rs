//! Daemon state on the PipeWire control loop: the desired microphone configuration, the live
//! engine of the current graph session and config persistence.

mod capture;
mod dictation;
mod hotkeys;
mod microphone;
mod mixer;
mod presets;
mod requests;
mod soundpad;

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
/// A failed save stays pending and is tried again after this long.
const SAVE_RETRY: Duration = Duration::from_secs(5);

#[allow(clippy::large_enum_variant)] // One transient request; settings are already bounded by the protocol.
pub enum ControlMessage {
    Request {
        request: Request,
        reply: mpsc::Sender<Outcome>,
    },
    Disconnected,
    /// Wake the control loop at the next held-volume deadline.
    HotkeyRepeat,
    /// A global shortcut went down or up (HK-001).
    Hotkey {
        action: String,
        pressed: bool,
    },
    /// The hotkey backend registered (or failed to register) the shortcuts.
    HotkeyStatus(crate::hotkeys::Status),
}

pub struct DaemonHost {
    #[cfg(windows)]
    output_controls: crate::output_controls::Manager,
    dictation: crate::dictation::Dictation,
    config: Config,
    config_recovery: Option<macafon_ipc::ConfigRecovery>,
    store: Option<Store>,
    presets: PresetStore,
    changed_at: Option<Instant>,
    /// The mixer configuration changed since the backend last asked (`take_mix_changed`).
    mix_changed: bool,
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
    pressed_hotkeys: std::collections::HashSet<String>,
    volume_repeats: hotkeys::VolumeRepeats,
    icons: macafon_apps::icons::IconResolver,
    /// Created when the daemon starts, where the platform has a soundpad (§7.6).
    soundpad: Option<crate::soundpad::Soundpad>,
    soundpad_dirty: bool,
    soundpad_broadcast_at: Option<Instant>,
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
        crate::osd::set_dictation_control(control.clone());
        if config.mic.noise_floor_dbfs.is_some() && config.mic.noise_floor_device.is_none() {
            // Older configurations only supported calibration of the primary input.
            config.mic.noise_floor_device = config.mic.device.clone();
        }
        let selected_device = config.mic.device.clone();
        Self {
            dictation: crate::dictation::Dictation::new(&config.dictation),
            selected_device,
            selected_stable_id: None,
            active_device: None,
            config,
            config_recovery: None,
            store,
            presets,
            changed_at: None,
            mix_changed: false,
            control: None,
            telemetry: hub.telemetry(),
            #[cfg(windows)]
            output_controls: crate::output_controls::Manager::new(hub.clone()),
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
            pressed_hotkeys: std::collections::HashSet::new(),
            volume_repeats: hotkeys::VolumeRepeats::new(control.clone()),
            icons: macafon_apps::icons::IconResolver::from_env(),
            soundpad: None,
            soundpad_dirty: false,
            soundpad_broadcast_at: None,
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
        if self.changed_at.is_some() && self.save() {
            self.changed_at = None;
        }
    }

    /// False when the config could not be written; it is then still unsaved.
    fn save(&self) -> bool {
        let Some(store) = &self.store else {
            return true;
        };
        match store.save(&self.config) {
            Ok(()) => {
                tracing::debug!(path = %store.path().display(), "Config saved");
                true
            }
            Err(error) => {
                tracing::error!(%error, "Cannot save config");
                false
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
            dictation: self.config.dictation.enabled,
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

    fn master_volume(&self) -> (f32, bool) {
        (
            self.config.mixer.master_volume_pct,
            self.config.mixer.master_muted,
        )
    }
    fn master_volume_changed(&mut self, volume_pct: f32, muted: bool) {
        self.config.mixer.master_volume_pct = volume_pct;
        self.config.mixer.master_muted = muted;
        self.mixer_changed();
    }

    fn mixer_enabled(&self) -> bool {
        self.config.mixer.enabled
    }

    fn app_mix(&self, stream: &PlaybackStream) -> Option<AppMix> {
        self.mixer.app_mix(&self.config, stream)
    }

    fn new_app_mix(&self) -> Option<AppMix> {
        let mixer = &self.config.mixer;
        let channel = mixer.channel(&mixer.default_channel)?;
        Some(AppMix {
            channel_pct: channel.settings.volume_pct,
            app_pct: 100.0,
            master_pct: mixer.master_volume_pct,
            channel_muted: channel.settings.muted,
            master_muted: mixer.master_muted,
        })
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

    fn take_mix_changed(&mut self) -> bool {
        std::mem::take(&mut self.mix_changed)
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
            self.start_soundpad();
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
            crate::ui::attach(self.hub.clone());
            // A warm UI opens the tray panel and the hotkey overlay at once (owner decision,
            // 2026-10-17, against NFR-R5); without a tray nobody would open them.
            #[cfg(windows)]
            if self.tray_wanted {
                crate::ui::prewarm();
            }
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
        self.ensure_dictation_audio();
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
        self.cancel_dictation();
        self.stop_dictation_audio();
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
        self.config.dictation.enabled
            || self.recording.is_some()
            || self.ab.is_recording()
            || self.hub.metering()
    }

    fn player_request(&mut self) -> Option<PlayerRequest> {
        self.ab.take_request()
    }

    fn soundpad_tap(
        &mut self,
        output: macafon_backend::SoundpadOutput,
    ) -> Option<macafon_backend::SoundpadTap> {
        Some(self.soundpad.as_mut()?.tap(output))
    }

    fn soundpad_monitor(&self) -> Option<Option<String>> {
        self.soundpad
            .as_ref()?
            .monitor_wanted(&self.config.soundpad)
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
            if self.soundpad.is_some() {
                self.soundpad_changed();
            }
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
            ControlMessage::HotkeyRepeat => return self.repeat_hotkeys(Instant::now()),
            ControlMessage::Hotkey { action, pressed } => return self.hotkey(&action, pressed),
            ControlMessage::HotkeyStatus(status) => {
                self.hotkey_status = Some(status);
                return self.hotkeys_changed();
            }
            message => message,
        };
        let ControlMessage::Request { request, reply } = message else {
            // The UI process went away (it stays warm in the background, so this is a crash
            // or the daemon going down): only stop what it was doing. Closing the window is
            // `ui.window_closed`.
            if self.dictation.test {
                self.cancel_dictation();
            }
            self.ab.clear(self.control.as_mut());
            self.ab_changed();
            return;
        };
        #[cfg(windows)]
        if matches!(
            request,
            Request::OutputControlsList | Request::OutputControlsSet { .. }
        ) {
            self.output_controls.dispatch(request, reply);
            return;
        }
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
        self.tick_dictation();
        self.tick_soundpad(Instant::now());
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
        #[cfg(windows)]
        if let Some(recording) = &mut self.recording {
            recording.input_device.clone_from(&self.active_device);
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
            // `elapsed` of a future instant is zero: the retry waits a whole SAVE_RETRY.
            self.changed_at = (!self.save()).then(|| Instant::now() + SAVE_RETRY - SAVE_DEBOUNCE);
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
    fn explorer_category_rename_keeps_settings_and_both_hotkeys() {
        let root = std::env::temp_dir().join(format!("macafon-explorer-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sounds/Short sounds")).expect("category");
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&38u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        for value in [1u16, 1] {
            wav.extend_from_slice(&value.to_le_bytes());
        }
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&96000u32.to_le_bytes());
        for value in [2u16, 16] {
            wav.extend_from_slice(&value.to_le_bytes());
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&0i16.to_le_bytes());
        std::fs::write(root.join("sounds/Short sounds/tone.wav"), wav).expect("valid sound");
        let mut config = Config::default();
        config.soundpad.folder = Some(root.join("sounds"));
        config
            .soundpad
            .edit_sound("Short sounds/tone.wav", |sound| {
                sound.favorite = true;
                sound.play_count = 9;
            });
        for (suffix, key) in [("", "Ctrl+F8"), ("#secondary", "Ctrl+F9")] {
            config.hotkeys.insert(
                format!("soundpad_play.Short sounds/tone.wav{suffix}"),
                key.into(),
            );
        }
        let (sender, _receiver) = crate::platform::control_channel();
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
            sender,
            false,
        );
        host.soundpad = Some(crate::soundpad::Soundpad::new(
            &host.config.soundpad,
            &root.join("state"),
        ));
        let wait_for = |host: &mut DaemonHost, id: &str| {
            host.respond_soundpad(macafon_ipc::Request::SoundpadRescan)
                .expect("request scan");
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            loop {
                host.tick_soundpad(Instant::now());
                if host.soundpad.as_ref().expect("soundpad").entry(id).is_ok() {
                    break;
                }
                assert!(Instant::now() < deadline, "inventory did not arrive");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        wait_for(&mut host, "Short sounds/tone.wav");
        std::fs::rename(
            root.join("sounds/Short sounds"),
            root.join("sounds/Renamed"),
        )
        .expect("Explorer rename");
        wait_for(&mut host, "Renamed/tone.wav");
        assert!(host.config.soundpad.sound("Renamed/tone.wav").favorite);
        assert_eq!(host.config.soundpad.sound("Renamed/tone.wav").play_count, 9);
        for (suffix, key) in [("", "Ctrl+F8"), ("#secondary", "Ctrl+F9")] {
            assert_eq!(
                host.config
                    .hotkeys
                    .get(&format!("soundpad_play.Renamed/tone.wav{suffix}"))
                    .map(String::as_str),
                Some(key)
            );
        }
        drop(host);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn deleting_soundpad_category_preserves_settings_and_both_hotkeys_on_collision() {
        let root =
            std::env::temp_dir().join(format!("macafon-category-settings-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sounds/Memes")).expect("category");
        std::fs::write(root.join("sounds/beep.#звук.wav"), b"root").expect("root sound");
        std::fs::write(root.join("sounds/Memes/beep.#звук.wav"), b"category")
            .expect("category sound");
        let mut config = Config::default();
        config.soundpad.folder = Some(root.join("sounds"));
        config.soundpad.edit_sound("Memes/beep.#звук.wav", |sound| {
            sound.favorite = true;
            sound.play_count = 7;
        });
        for (suffix, key) in [("", "Ctrl+F8"), ("#secondary", "Ctrl+F9")] {
            config.hotkeys.insert(
                format!("soundpad_play.Memes/beep.#звук.wav{suffix}"),
                key.into(),
            );
        }
        let (sender, _receiver) = crate::platform::control_channel();
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
            sender,
            false,
        );
        host.soundpad = Some(crate::soundpad::Soundpad::new(
            &host.config.soundpad,
            &root.join("state"),
        ));
        // Do not rescan first: the operation must also handle files added since the last scan.
        host.respond_soundpad(macafon_ipc::Request::SoundpadDeleteCategory {
            name: "Memes".into(),
        })
        .expect("delete category");
        let new_id = "beep.#звук (2).wav";
        assert!(host.config.soundpad.sound(new_id).favorite);
        assert_eq!(host.config.soundpad.sound(new_id).play_count, 7);
        assert!(!host.config.soundpad.sound("beep.#звук.wav").favorite);
        for (suffix, key) in [("", "Ctrl+F8"), ("#secondary", "Ctrl+F9")] {
            assert_eq!(
                host.config
                    .hotkeys
                    .get(&format!("soundpad_play.{new_id}{suffix}"))
                    .map(String::as_str),
                Some(key)
            );
        }
        drop(host);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn volume_hotkeys_repeat_after_delay_and_stop_on_release_or_rebind() {
        use std::time::Duration;
        let (sender, _receiver) = crate::platform::control_channel();
        let mut config = Config::default();
        config.ui.volume_step_pct = 3;
        config.mixer.channels[0].settings.volume_pct = 50.0;
        config.mixer.master_volume_pct = 50.0;
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
            sender,
            false,
        );
        let now = Instant::now();
        let at = |ms| now + Duration::from_millis(ms);
        let volume = |host: &DaemonHost| host.config.mixer.channels[0].settings.volume_pct;
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        host.hotkey_at("channel_volume_up.main", true, at(0));
        host.hotkey_at("channel_volume_up.main", true, at(100));
        host.repeat_hotkeys(at(379));
        assert_eq!(volume(&host), 53.0, "a tap is one precise step");
        host.repeat_hotkeys(at(380));
        let first = volume(&host);
        assert!(
            first > 53.0 && first < 54.0,
            "the hold starts slowly: {first}"
        );
        // Steady frames for a while: the speed eases up, never jumps.
        let mut last_delta = first - 53.0;
        for frame in 1..=100 {
            let before = volume(&host);
            host.repeat_hotkeys(at(380 + frame * 30));
            let delta = volume(&host) - before;
            if before < 100.0 && volume(&host) < 100.0 {
                assert!(delta + 1e-3 >= last_delta, "accelerates at frame {frame}");
                assert!(delta - last_delta < 0.2, "smooth at frame {frame}");
                last_delta = delta;
            }
        }
        assert_eq!(volume(&host), 100.0);
        host.hotkey_at("channel_volume_up.main", false, at(3500));
        host.repeat_hotkeys(at(4000));
        assert_eq!(volume(&host), 100.0, "release stops the motion");
        host.hotkey_at("channel_volume_down.main#secondary", true, at(5000));
        assert_eq!(volume(&host), 97.0);
        host.hotkey_at("channel_volume_down.main#secondary", false, at(5100));
        host.repeat_hotkeys(at(10000));
        assert_eq!(volume(&host), 97.0);
        host.hotkey_at("master_volume_up#secondary", true, at(11000));
        host.repeat_hotkeys(at(16000));
        let master = host.config.mixer.master_volume_pct;
        assert!(
            close(master, 53.0 + 3.0 * super::hotkeys::HOLD_FAST * 0.1),
            "no catch-up burst after a stall: {master}"
        );
        host.hotkey_at("master_volume_up#secondary", false, at(16001));
        host.hotkey_at("mic_mute_toggle", true, at(17000));
        host.repeat_hotkeys(at(20000));
        assert!(host.config.mic.chain.muted, "toggles do not repeat");
        let (service, _bindings) = crate::hotkeys::Hotkeys::fake();
        host.hotkeys = Some(service);
        host.config.mixer.channels[0].settings.volume_pct = 50.0;
        host.hotkey_at("channel_volume_up.main", true, at(21000));
        host.rebind_hotkeys();
        host.repeat_hotkeys(at(22000));
        assert_eq!(
            host.config.mixer.channels[0].settings.volume_pct, 53.0,
            "rebinding cancels a repeat"
        );
    }

    #[test]
    fn secondary_bindings_hold_until_both_are_released_and_use_configured_step() {
        let (sender, _receiver) = crate::platform::control_channel();
        let mut config = Config::default();
        config.ui.volume_step_pct = 3;
        config.mixer.channels[0].settings.volume_pct = 50.0;
        let mut host = DaemonHost::new(
            config,
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
            sender,
            false,
        );
        host.set_hotkey("push_to_talk", Some("Ctrl+F8".into()))
            .expect("primary");
        host.set_hotkey("push_to_talk#secondary", Some("Ctrl+F9".into()))
            .expect("secondary");
        assert_eq!(
            host.hotkeys_state()
                .bindings
                .iter()
                .find(|b| b.action == "push_to_talk")
                .expect("binding")
                .secondary_shortcut
                .as_deref(),
            Some("Ctrl+F9")
        );
        host.hotkey("push_to_talk", true);
        host.hotkey("push_to_talk#secondary", true);
        host.hotkey("push_to_talk", false);
        assert!(!host.config.mic.chain.muted);
        host.hotkey("push_to_talk#secondary", false);
        assert!(host.config.mic.chain.muted);
        host.hotkey("channel_volume_up.main#secondary", true);
        assert_eq!(host.config.mixer.channels[0].settings.volume_pct, 53.0);
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
    fn chain_edit_sent_after_a_mute_hotkey_keeps_the_microphone_muted() {
        let (sender, _receiver) = crate::platform::control_channel();
        let mut host = DaemonHost::new(
            Config::default(),
            None,
            PresetStore::in_memory(),
            unused_endpoint(),
            sender,
            false,
        );
        let mut stale = host.config.mic.chain;
        host.respond(Request::MicSetMuted { muted: true })
            .expect("mute");
        stale.gain.params.gain_db = 3.0;
        host.respond(Request::MicSet { settings: stale })
            .expect("edit");
        assert!(host.config.mic.chain.muted);
        assert_eq!(host.config.mic.chain.gain.params.gain_db, 3.0);
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
        assert_no_alloc::assert_no_alloc(|| host.refresh_input());
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
