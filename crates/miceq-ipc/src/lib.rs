//! Daemon control protocol (PRD §11.5): newline-delimited JSON over a per-user Unix socket.
//!
//! Requests: `{"id":1,"method":"mic.set_preset","params":{"preset":"broadcast"}}`.
//! Replies: `{"id":1,"result":…}` or `{"id":1,"error":{"code":…,"message":…}}`.
//! Events: `{"event":"meters","data":…}`. The first request must be `hello`.

mod client;

pub use client::{Client, ClientError};
pub use miceq_dsp::{ChainSettings, Meters};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;
/// Lines longer than this are rejected instead of buffered without bound.
pub const MAX_LINE_BYTES: usize = 1 << 20;
pub const MAX_METER_HZ: u32 = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum Request {
    #[serde(rename = "hello")]
    Hello { protocol: u32, client: String },
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
    #[serde(rename = "presets.list")]
    PresetsList,
    #[serde(rename = "devices.list")]
    DevicesList,
    /// Stream `meters` events at up to 60 Hz until unsubscribed (MTR-004, NFR-R7).
    #[serde(rename = "meters.subscribe")]
    MetersSubscribe { hz: u32 },
    #[serde(rename = "meters.unsubscribe")]
    MetersUnsubscribe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Malformed JSON, unknown method or missing `hello`.
    BadRequest,
    ProtocolMismatch,
    InvalidParams,
    NotFound,
    Busy,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
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
pub struct Response {
    /// `None` only when the request id could not be parsed.
    pub id: Option<u64>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Result(serde_json::Value),
    Error(ErrorBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum Event {
    #[serde(rename = "meters")]
    Meters(Meters),
    /// Any change of microphone state, from any client or the daemon itself.
    #[serde(rename = "mic.changed")]
    MicChanged(MicState),
}

/// One line from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum ServerMessage {
    Response(Response),
    Event(Event),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub daemon_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicState {
    pub settings: ChainSettings,
    /// Preset the chain was last loaded from.
    pub preset: Option<String>,
    pub device: Option<String>,
    /// The selected microphone is present and linked (MIC-005).
    pub input_connected: bool,
    pub latency_samples: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// PipeWire `node.name`, stable across reconnects.
    pub name: String,
    pub description: String,
}

/// `$XDG_RUNTIME_DIR/miceq/miceqd.sock` (PRD §10.4, §13).
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty())?;
    Some(Path::new(&runtime).join("miceq").join("miceqd.sock"))
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
                settings,
                preset: Some("natural".into()),
                device: None,
                input_connected: false,
                latency_samples: 960,
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
