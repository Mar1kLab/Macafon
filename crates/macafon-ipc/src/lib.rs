//! Daemon control protocol (PRD §11.5): newline-delimited JSON over a per-user Unix socket
//! or, on Windows, a per-user named pipe (`transport`).
//!
//! Requests: `{"id":1,"method":"mic.set_preset","params":{"preset":"broadcast"}}`.
//! Replies: `{"id":1,"result":…}` or `{"id":1,"error":{"code":…,"message":…}}`.
//! Events: `{"event":"meters","data":…}`. The first request must be `hello`.

mod client;
mod dictation;
mod displays;
mod doctor;
mod framing;
mod hotkeys;
mod mixer;
mod output_control;
mod request;
mod sounddeck;
mod state;
pub mod transport;
#[cfg(all(test, feature = "ts"))]
mod typescript;
mod wire;

pub use client::{Client, ClientError};
pub use dictation::{
    DictationEngine, DictationGamePolicy, DictationInsert, DictationLanguage,
    DictationLanguageInfo, DictationLanguagePolicy, DictationLevel, DictationModel,
    DictationModelStatus, DictationMove, DictationOutcome, DictationPhase, DictationPopupLook,
    DictationPopupPosition, DictationPrefs, DictationPrefsPatch, DictationResult, DictationState,
    DictationStorage, DictationTap, WHISPER_LANGUAGES, dictation_layout_language,
    installed_dictation_languages, supported_dictation_languages,
};
pub use displays::{
    DISPLAY_PROFILE_ICONS, DisplayIdentity, DisplayLayout, DisplayMode, DisplayMonitor,
    DisplayProfile, DisplayProfileInfo, DisplayRotation, DisplaysPending, DisplaysState,
    ProfileMonitor,
};
pub use doctor::{DoctorCheck, DoctorCheckId, DoctorReport, DoctorStatus};
pub use framing::{FrameError, read_protocol_line};
pub use hotkeys::{
    BACKGROUND_ARG, HotkeyAction, HotkeyBackend, HotkeyBinding, HotkeysState, OVERLAY_ARG,
    QUICK_PANEL_ARG, UiTarget,
};
pub use macafon_dsp::calibration::CalibrationReport;
pub use macafon_dsp::{ChainSettings, InputChannel, Meters};
pub use mixer::{
    AppState, ChannelMeterFrame, ChannelPatch, ChannelState, MixerMeters, MixerState, OutputDevice,
};
pub use output_control::{MacaronSettings, OutputControl};
pub use transport::Endpoint;

use serde::Serialize;
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;
/// Lines longer than this are rejected instead of buffered without bound.
pub const MAX_LINE_BYTES: usize = 1 << 20;
pub const MAX_METER_HZ: u32 = 60;

pub use request::Request;
pub use sounddeck::{
    SOUNDDECK_UPLOAD_CHUNK_BYTES, SoundDeckOverlayControls, SoundDeckOverlayView,
    SoundDeckPlayback, SoundDeckPrefs, SoundDeckPrefsPatch, SoundDeckState, SoundInfo, SoundPatch,
};
pub use state::{
    AbRecording, AbSide, AbState, AbWaveform, AutostartMethod, AutostartState, CalibrationFailure,
    CalibrationState, CalibrationStep, Capabilities, CloseAction, ConfigRecovery, Device,
    DeviceKind, Hello, MicState, OsdEdge, OsdLook, OsdPrefs, OsdStyle, OverlayExpand, OverlayItem,
    OverlayPlacement, OverlayPrefs, Platform, PlaybackDevice, PlaybackDeviceKind, Preferences,
    PresetInfo,
};
pub use wire::{ErrorBody, ErrorCode, Event, Outcome, RequestEnvelope, Response, ServerMessage};

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
                is_default_source: false,
                virtual_mic_ready: true,
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
