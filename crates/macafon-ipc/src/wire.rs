//! Response envelopes, errors and asynchronous events.

use crate::{
    AbState, CalibrationState, Device, HotkeysState, Meters, MicState, MixerMeters, MixerState,
    Preferences, PresetInfo, Request,
};
use serde::{Deserialize, Serialize};

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
    #[serde(rename = "output_controls.changed")]
    OutputControlsChanged(Vec<crate::OutputControl>),
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
    /// Any change of channels, apps, Master or outputs.
    #[serde(rename = "mixer.changed")]
    MixerChanged(MixerState),
    /// With `meters` while subscribed and the mixer is on.
    #[serde(rename = "mixer.meters")]
    MixerMeters(MixerMeters),
    #[serde(rename = "hotkeys.changed")]
    HotkeysChanged(HotkeysState),
    /// A microphone was plugged in or removed (MIC-002); the full list, as `devices.list`.
    #[serde(rename = "devices.changed")]
    DevicesChanged(Vec<Device>),
    #[serde(rename = "prefs.changed")]
    PrefsChanged(Preferences),
    /// Any change of the library or the soundpad preferences.
    #[serde(rename = "soundpad.changed")]
    SoundpadChanged(crate::SoundpadState),
    /// Playback progress, about 10 Hz while a sound plays; `null` when it ends.
    #[serde(rename = "soundpad.playback")]
    SoundpadPlayback(Option<crate::SoundpadPlayback>),
    /// For the UI process, not its pages: show a window or toggle a panel. The daemon sends
    /// it instead of starting a second UI process, which is what made panels slow to open.
    #[serde(rename = "ui.show")]
    UiShow { target: crate::UiTarget },
    /// Any change of dictation state, models, downloads or history (§8.4).
    #[serde(rename = "dictation.changed")]
    DictationChanged(crate::DictationState),
    /// The tap level while listening (popup) or while the UI is subscribed to meters.
    #[serde(rename = "dictation.level")]
    DictationLevel(crate::DictationLevel),
    /// A finished transcription, also for TRY IT results that are not in the history.
    #[serde(rename = "dictation.result")]
    DictationResult(crate::DictationResult),
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
