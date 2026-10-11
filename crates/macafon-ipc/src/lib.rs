//! Daemon control protocol (PRD §11.5): newline-delimited JSON over a per-user Unix socket.
//!
//! Requests: `{"id":1,"method":"mic.set_preset","params":{"preset":"broadcast"}}`.
//! Replies: `{"id":1,"result":…}` or `{"id":1,"error":{"code":…,"message":…}}`.
//! Events: `{"event":"meters","data":…}`. The first request must be `hello`.

mod client;
#[cfg(all(test, feature = "ts"))]
mod typescript;

pub use client::{Client, ClientError};
pub use macafon_dsp::calibration::CalibrationReport;
pub use macafon_dsp::{ChainSettings, InputChannel, Meters};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;
/// Lines longer than this are rejected instead of buffered without bound.
pub const MAX_LINE_BYTES: usize = 1 << 20;
pub const MAX_METER_HZ: u32 = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "method", content = "params")]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum Request {
    #[serde(rename = "hello")]
    Hello { protocol: u32, client: String },
    /// Stop the daemon: the virtual microphone disappears (BG-005).
    #[serde(rename = "daemon.quit")]
    DaemonQuit,
    /// "Start with system" (AS-001).
    #[serde(rename = "autostart.get")]
    AutostartGet,
    #[serde(rename = "autostart.set")]
    AutostartSet { enabled: bool },
    #[serde(rename = "mic.get")]
    MicGet,
    /// Replace the whole chain; continuous changes are smoothed, structural ones crossfaded.
    #[serde(rename = "mic.set")]
    MicSet { settings: ChainSettings },
    #[serde(rename = "mic.set_preset")]
    MicSetPreset { preset: String },
    #[serde(rename = "mic.set_muted")]
    MicSetMuted { muted: bool },
    #[serde(rename = "mic.toggle_mute")]
    MicToggleMute,
    #[serde(rename = "mic.set_bypass")]
    MicSetBypass { bypass: bool },
    /// Physical microphone by PipeWire `node.name`; `null` disconnects.
    #[serde(rename = "mic.set_device")]
    MicSetDevice { device: Option<String> },
    /// Optional second-priority microphone; `null` disables fallback.
    #[serde(rename = "mic.set_fallback_device")]
    MicSetFallbackDevice { device: Option<String> },
    #[serde(rename = "mic.set_input_channel")]
    MicSetInputChannel { channel: InputChannel },
    #[serde(rename = "mic.set_default_source")]
    MicSetDefaultSource { confirmed: bool },
    #[serde(rename = "mic.restore_default_source")]
    MicRestoreDefaultSource,
    #[serde(rename = "ab.export")]
    AbExport { side: AbSide, path: String },
    #[serde(rename = "ab.outputs")]
    AbOutputs,
    #[serde(rename = "presets.list")]
    PresetsList,
    /// Save the current chain as a new user preset and select it.
    #[serde(rename = "presets.save")]
    PresetsSave { name: String },
    /// Overwrite a user preset with the current chain and select it.
    #[serde(rename = "presets.update")]
    PresetsUpdate { id: String },
    #[serde(rename = "presets.rename")]
    PresetsRename { id: String, name: String },
    /// Copy any preset (built-in or user) under a free name.
    #[serde(rename = "presets.duplicate")]
    PresetsDuplicate { id: String },
    #[serde(rename = "presets.delete")]
    PresetsDelete { id: String },
    /// The `.macafon-preset.json` document of a preset.
    #[serde(rename = "presets.export")]
    PresetsExport { id: String },
    /// Add a preset from a `.macafon-preset.json` document; a taken name gets a suffix.
    #[serde(rename = "presets.import")]
    PresetsImport { document: serde_json::Value },
    #[serde(rename = "devices.list")]
    DevicesList,
    #[serde(rename = "devices.subscribe")]
    DevicesSubscribe,
    #[serde(rename = "devices.unsubscribe")]
    DevicesUnsubscribe,
    /// Stream `meters` events at up to 60 Hz until unsubscribed (MTR-004, NFR-R7).
    #[serde(rename = "meters.subscribe")]
    MetersSubscribe { hz: u32 },
    #[serde(rename = "meters.unsubscribe")]
    MetersUnsubscribe,
    /// Record silence, then speech, and calibrate the chain (PRD §6.5). Progress and the
    /// result arrive as `calibration.changed` events.
    #[serde(rename = "calibration.start")]
    CalibrationStart,
    #[serde(rename = "calibration.cancel")]
    CalibrationCancel,
    #[serde(rename = "calibration.get")]
    CalibrationGet,
    /// AB-001: record the raw microphone (before the chain) for up to `max_seconds`
    /// (3–30, default 10). Stops playback; the new clip replaces the old one.
    #[serde(rename = "ab.record")]
    AbRecord {
        #[serde(default)]
        max_seconds: Option<u32>,
    },
    /// Finish the recording early (after at least 3 s).
    #[serde(rename = "ab.stop_recording")]
    AbStopRecording,
    /// AB-002: loop the clip into a sink `node.name`, or the default output.
    #[serde(rename = "ab.play")]
    AbPlay {
        #[serde(default)]
        device: Option<String>,
    },
    #[serde(rename = "ab.stop")]
    AbStop,
    /// AB-004: A = raw, B = processed by the current chain.
    #[serde(rename = "ab.select")]
    AbSelect { side: AbSide },
    #[serde(rename = "ab.toggle")]
    AbToggle,
    #[serde(rename = "ab.seek")]
    AbSeek { position_ms: u32 },
    /// AB-005: level A to B's loudness.
    #[serde(rename = "ab.set_loudness_match")]
    AbSetLoudnessMatch { enabled: bool },
    #[serde(rename = "ab.get")]
    AbGet,
    /// AB-006: peak envelopes of A and B with `points` values each (16–4096).
    #[serde(rename = "ab.waveform")]
    AbWaveform { points: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestEnvelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Malformed JSON, unknown method or missing `hello`.
    BadRequest,
    ProtocolMismatch,
    InvalidParams,
    NotFound,
    /// A preset with this name already exists.
    AlreadyExists,
    Busy,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[error("{code:?}: {message}")]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Response {
    /// `None` only when the request id could not be parsed.
    pub id: Option<u64>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Result(serde_json::Value),
    Error(ErrorBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "event", content = "data")]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum Event {
    #[serde(rename = "mic.noise_fallback")]
    NoiseFallback {
        engine: macafon_dsp::settings::NoiseEngine,
    },
    #[serde(rename = "devices.meter")]
    DeviceMeter { name: String, peak_dbfs: f32 },
    #[serde(rename = "meters")]
    Meters(Meters),
    /// Any change of microphone state, from any client or the daemon itself.
    #[serde(rename = "mic.changed")]
    MicChanged(MicState),
    /// Calibration progress (about 10 Hz while measuring) and its outcome.
    #[serde(rename = "calibration.changed")]
    CalibrationChanged(CalibrationState),
    /// A/B test state; about 10 Hz while recording or playing.
    #[serde(rename = "ab.changed")]
    AbChanged(AbState),
    /// The preset list changed (saved, renamed, deleted, imported).
    #[serde(rename = "presets.changed")]
    PresetsChanged(Vec<PresetInfo>),
}

/// One line from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum ServerMessage {
    Response(Response),
    Event(Event),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Hello {
    pub protocol: u32,
    pub daemon_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MicState {
    /// Virtual microphone output uses identical FL/FR channels instead of MONO.
    #[serde(default)]
    pub stereo: bool,
    #[serde(default, skip_serializing_if = "input_channel_is_auto")]
    pub input_channel: InputChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_recovery: Option<ConfigRecovery>,
    pub settings: ChainSettings,
    /// Preset the chain was last loaded from or saved to.
    pub preset: Option<String>,
    /// The chain differs from that preset (UI: "modified", enables Save).
    pub preset_modified: bool,
    pub device: Option<String>,
    #[serde(default)]
    pub fallback_device: Option<String>,
    /// Microphone currently linked; `null` while disconnected.
    #[serde(default)]
    pub active_device: Option<String>,
    /// The selected microphone is present and linked (MIC-005).
    pub input_connected: bool,
    pub latency_samples: u32,
    /// Measured by the last calibration; the Expander Sensitivity macro is relative to it.
    pub noise_floor_dbfs: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AutostartMethod {
    /// `systemctl --user enable macafond.service`.
    Systemd,
    /// `$XDG_CONFIG_HOME/autostart/macafond.desktop`.
    XdgAutostart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AutostartState {
    pub enabled: bool,
    /// What enabling uses on this system.
    pub method: AutostartMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AbSide {
    /// Raw microphone.
    A,
    /// Through the current chain.
    B,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AbRecording {
    pub elapsed_ms: u32,
    pub max_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AbState {
    /// A recording in progress.
    pub recording: Option<AbRecording>,
    /// Length of the recorded clip, kept in memory only (AB-007).
    pub clip_ms: Option<u32>,
    pub playing: bool,
    pub side: AbSide,
    pub position_ms: u32,
    pub loudness_match: bool,
    /// Listening sink `node.name`; `None` is the default output.
    pub device: Option<String>,
}

/// Peak envelopes, 0…1 (linear), aligned in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AbWaveform {
    pub a: Vec<f32>,
    pub b: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CalibrationStep {
    /// CAL-001: "Stay silent", 4 s.
    Silence,
    /// CAL-002: "Read this sentence", 6 s.
    Speech,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CalibrationFailure {
    /// CAL-005: the input clipped; lower the system or interface level and retry.
    Clipped,
    NoSpeech,
    /// The microphone disconnected or PipeWire restarted.
    Disconnected,
    /// No audio arrived from the microphone.
    Stalled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CalibrationState {
    Idle,
    Measuring {
        step: CalibrationStep,
        /// 0…1 within the step.
        progress: f32,
        step_duration_ms: u32,
    },
    /// The calibrated settings are already applied (see `mic.changed`).
    Done {
        report: CalibrationReport,
    },
    Failed {
        reason: CalibrationFailure,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PresetInfo {
    /// Built-in: `natural`, …; user: `user:<slug>`.
    pub id: String,
    pub name: String,
    /// Built-in presets are read-only.
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Device {
    /// PipeWire `node.name`, stable across reconnects.
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stable_id: String,
    #[serde(default, skip_serializing_if = "device_kind_is_unknown")]
    pub kind: DeviceKind,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Usb,
    Builtin,
    Headset,
    Bluetooth,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PlaybackDeviceKind {
    Headphones,
    Speakers,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlaybackDevice {
    pub name: String,
    pub description: String,
    pub kind: PlaybackDeviceKind,
    pub speaker_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConfigRecovery {
    pub backup: String,
    pub reason: String,
}

fn input_channel_is_auto(channel: &InputChannel) -> bool {
    *channel == InputChannel::Auto
}
fn device_kind_is_unknown(kind: &DeviceKind) -> bool {
    *kind == DeviceKind::Unknown
}

/// `$XDG_RUNTIME_DIR/macafon/macafond.sock` (PRD §10.4, §13).
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty())?;
    Some(Path::new(&runtime).join("macafon").join("macafond.sock"))
}

pub fn encode(message: &impl Serialize) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

/// Parse a request line; on failure return the error reply to send back.
pub fn decode_request(line: &str) -> Result<RequestEnvelope, Response> {
    serde_json::from_str(line).map_err(|error| {
        let id = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| value.get("id")?.as_u64());
        Response {
            id,
            outcome: Outcome::Error(ErrorBody::new(ErrorCode::BadRequest, error.to_string())),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_matches_the_prd_sketch() {
        let set =
            decode_request(r#"{"id":2,"method":"mic.set_preset","params":{"preset":"broadcast"}}"#)
                .expect("valid request");
        assert_eq!(set.id, 2);
        assert_eq!(
            set.request,
            Request::MicSetPreset {
                preset: "broadcast".into()
            }
        );
        let get = decode_request(r#"{"id":3,"method":"mic.get"}"#).expect("no params");
        assert_eq!(get.request, Request::MicGet);
        let line = encode(&RequestEnvelope {
            id: 4,
            request: Request::MetersSubscribe { hz: 60 },
        })
        .expect("encode");
        assert_eq!(
            line,
            "{\"id\":4,\"method\":\"meters.subscribe\",\"params\":{\"hz\":60}}\n"
        );
        let unknown = decode_request(r#"{"id":5,"method":"mic.explode"}"#).expect_err("unknown");
        assert_eq!(unknown.id, Some(5));
        assert!(matches!(
            unknown.outcome,
            Outcome::Error(ErrorBody {
                code: ErrorCode::BadRequest,
                ..
            })
        ));
    }

    #[test]
    fn server_messages_round_trip() {
        let mut settings = ChainSettings::default();
        settings.expander.params.ratio = f32::INFINITY;
        let messages = [
            ServerMessage::Response(Response {
                id: Some(1),
                outcome: Outcome::Result(serde_json::json!({"ok": true})),
            }),
            ServerMessage::Response(Response {
                id: None,
                outcome: Outcome::Error(ErrorBody::new(ErrorCode::Busy, "later")),
            }),
            ServerMessage::Event(Event::MicChanged(MicState {
                stereo: false,
                input_channel: InputChannel::Auto,
                config_recovery: None,
                settings,
                preset: Some("natural".into()),
                preset_modified: false,
                device: None,
                fallback_device: None,
                active_device: None,
                input_connected: false,
                latency_samples: 960,
                noise_floor_dbfs: Some(-62.5),
            })),
            ServerMessage::Event(Event::AbChanged(AbState {
                recording: None,
                clip_ms: Some(10_000),
                playing: true,
                side: AbSide::B,
                position_ms: 1234,
                loudness_match: true,
                device: None,
            })),
            ServerMessage::Event(Event::CalibrationChanged(CalibrationState::Measuring {
                step: CalibrationStep::Speech,
                progress: 0.5,
                step_duration_ms: 6000,
            })),
        ];
        for message in messages {
            let line = encode(&message).expect("encode");
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
            let back: ServerMessage = serde_json::from_str(&line).expect("decode");
            assert_eq!(back, message);
        }
    }
}
