//! Daemon ↔ worker protocol over the worker's stdin/stdout (PRD §8.4.3).
//!
//! Every frame is `u32 LE length` (of what follows) + `u8 kind` + payload:
//! - [`KIND_JSON`]: a UTF-8 JSON [`WorkerRequest`] (daemon → worker) or [`WorkerEvent`]
//!   (worker → daemon);
//! - [`KIND_AUDIO`]: daemon → worker only, mono f32 LE samples at [`crate::SAMPLE_RATE_HZ`]
//!   appended to the current utterance.
//!
//! The worker exits when its stdin closes.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const KIND_JSON: u8 = 1;
pub const KIND_AUDIO: u8 = 2;
/// Frames larger than this are a protocol error (5 min of audio is sent in small chunks).
pub const MAX_FRAME_BYTES: u32 = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Whisper,
    Parakeet,
}

pub use macafon_ipc::DictationLanguage as Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguagePolicy {
    Layout,
    Fixed,
    Auto,
}

/// What the daemon knows at the key press; the worker applies §8.4.4 with the detected
/// probabilities (Whisper) and reports the language it used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageInput {
    pub policy: LanguagePolicy,
    pub fixed: Language,
    /// Keyboard layout of the target window; `None` when it is unsupported or unknown.
    pub layout: Option<Language>,
    /// The last Cyrillic language used in this daemon session.
    pub last_cyrillic: Option<Language>,
    #[serde(default)]
    pub layout_languages: Vec<Language>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerRequest {
    /// Load a model; a different model replaces the loaded one.
    Load {
        engine: Engine,
        /// The model folder `<models dir>/<engine>/<model id>/` with the manifest files.
        model_dir: PathBuf,
        model_id: String,
        threads: u16,
        gpu: bool,
        /// Game mode, sampled by the daemon with SHQueryUserNotificationState at
        /// key press. Enables EcoQoS and halves the requested thread budget
        /// (minimum one); Ready reports the effective count. Omitted means false:
        /// ordinary dictation uses BELOW_NORMAL priority without EcoQoS.
        /// Reload when this changes; game mode also requires CPU and immediate
        /// unload after the result, enforced by the daemon.
        #[serde(default)]
        efficiency: bool,
    },
    /// Start a new utterance; audio frames that follow belong to it. May arrive before the
    /// model is loaded: the worker buffers the audio.
    Begin,
    /// The utterance is complete: run VAD and inference, answer with `Result` or `NoSpeech`.
    Finish {
        language: LanguageInput,
        /// Whisper `initial_prompt` words (STT-014); ignored by Parakeet.
        vocabulary: Vec<String>,
    },
    /// Drop the current utterance.
    Cancel,
    /// Free the model and exit.
    Exit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// The language passed to the engine; `None` when the engine chose it (Parakeet).
    pub language: Option<Language>,
    /// Whisper language probabilities; absent for Parakeet or a fixed language.
    pub lang_probs: Option<Vec<(Language, f32)>>,
    pub audio_ms: u32,
    /// Speech left after VAD trimming.
    pub speech_ms: u32,
    /// `Finish` received → result sent.
    pub elapsed_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ModelMissing,
    ModelLoad,
    Inference,
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    Loading,
    Ready {
        /// Process working set after loading.
        ram_bytes: u64,
        threads: u16,
    },
    Result(Transcript),
    /// VAD found under 200 ms of speech, or the output was a known hallucination (STT-011).
    NoSpeech {
        audio_ms: u32,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_load_requests_do_not_enable_efficiency() {
        let request: WorkerRequest = serde_json::from_str(
            r#"{"type":"load","engine":"whisper","model_dir":"models","model_id":"small","threads":6,"gpu":false}"#,
        ).expect("valid legacy request");
        assert!(matches!(
            request,
            WorkerRequest::Load {
                efficiency: false,
                ..
            }
        ));
    }
}
