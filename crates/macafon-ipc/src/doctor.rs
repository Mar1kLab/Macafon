//! Doctor (PRD §9.6): self-checks with plain explanations; the UI words them by `id`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Ok,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckId {
    /// PipeWire runs and is at least 1.0.
    Pipewire,
    /// WirePlumber runs and is at least 0.5 (routing, defaults).
    SessionManager,
    /// The physical microphone is selected and linked.
    MicrophoneInput,
    /// Some app records Macafon Microphone.
    MicrophoneUsed,
    /// Another noise suppressor works on the microphone too.
    DoubleProcessing,
    /// Channels exist and play into an output device.
    Mixer,
    /// The system output is the default channel while the mixer is on (OUT-005).
    DefaultOutput,
    /// Apps that cannot be moved (`node.dont-move`).
    UnmovableApps,
    /// Graph quantum and sample rate; overloads in the last 10 minutes.
    Processing,
    /// Global shortcuts: portal or X11; otherwise CLI commands (HK-004).
    Hotkeys,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DoctorCheck {
    pub id: DoctorCheckId,
    pub status: DoctorStatus,
    /// Values for the explanation: versions, counts, device or app names. Names stay out of
    /// the copied report.
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DoctorReport {
    pub daemon_version: String,
    pub pipewire_version: Option<String>,
    pub session_manager: Option<String>,
    pub quantum: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub checks: Vec<DoctorCheck>,
}
