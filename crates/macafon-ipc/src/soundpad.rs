//! Soundpad (PRD §7.6): a library of sound files the daemon plays into the virtual microphone
//! and the user's headphones at the same time.

use serde::{Deserialize, Serialize};

/// Largest decoded chunk of `soundpad.upload`; its base64 stays under `MAX_LINE_BYTES`.
pub const SOUNDPAD_UPLOAD_CHUNK_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SoundInfo {
    /// Path inside the library folder with `/` separators, `Memes/airhorn.mp3`; stable key for
    /// hotkeys (`soundpad_play.<id>`) and per-sound settings.
    pub id: String,
    /// File name without extension.
    pub name: String,
    /// First-level subfolder; `null` for files in the folder itself.
    pub category: Option<String>,
    /// Known once the file was probed or decoded.
    pub duration_ms: Option<u32>,
    /// Per-sound volume, 0…200 %.
    pub volume_pct: f32,
    pub favorite: bool,
    /// Times played, for sorting like Soundpad's play count.
    pub play_count: u32,
    /// When the file came into the library (its creation time), Unix ms; newest first is the
    /// default order.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub added_unix_ms: u64,
    /// The file could not be decoded.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SoundpadPlayback {
    pub sound_id: String,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub paused: bool,
    pub looping: bool,
    /// Decoding: playback starts as soon as the sound is ready.
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SoundpadPrefs {
    /// Play into Macafon Microphone, heard by the voice chat.
    pub to_microphone: bool,
    /// Play into the monitor device too, so the user hears the sound.
    pub to_headphones: bool,
    /// 0…200 %.
    pub microphone_volume_pct: f32,
    /// 0…200 %.
    pub headphones_volume_pct: f32,
    /// Silence the voice while a sound plays into the microphone.
    pub mute_voice_while_playing: bool,
    /// Monitor output device; `null` follows the system default output.
    pub monitor_device: Option<String>,
    /// Show the soundpad panel in the hotkey overlay.
    pub show_in_overlay: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SoundpadState {
    /// Absolute path of the library folder.
    pub folder: String,
    /// Every first-level subfolder, including empty ones.
    pub categories: Vec<String>,
    pub sounds: Vec<SoundInfo>,
    pub prefs: SoundpadPrefs,
    pub playback: Option<SoundpadPlayback>,
    /// Sounds reach Macafon Microphone (on Windows: VB-CABLE is installed).
    pub microphone_ready: bool,
}

/// `soundpad.set_prefs`: absent fields stay; `monitor_device` has its own request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SoundpadPrefsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_microphone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_headphones: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone_volume_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headphones_volume_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute_voice_while_playing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_in_overlay: Option<bool>,
}

/// `soundpad.set_sound`: absent fields stay.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SoundPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
}
