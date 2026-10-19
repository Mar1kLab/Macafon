//! Dictation (PRD §8.4): local speech to text. The daemon owns the audio tap, the worker
//! process, text insertion and the popup; the UI only shows state and changes preferences.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationEngine {
    #[default]
    Whisper,
    Parakeet,
}

/// Validated Whisper code, serialized as a string for config compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct DictationLanguage(&'static str);
impl DictationLanguage {
    pub fn from_code(code: &str) -> Option<Self> {
        WHISPER_LANGUAGES
            .iter()
            .find(|(candidate, _)| *candidate == code)
            .map(|(code, _)| Self(code))
    }
    pub fn code(self) -> &'static str {
        self.0
    }
    pub fn is_cyrillic(self) -> bool {
        matches!(self.0, "uk" | "ru")
    }
}
impl Default for DictationLanguage {
    fn default() -> Self {
        Self("en")
    }
}
impl<'de> Deserialize<'de> for DictationLanguage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Self::from_code(&code)
            .ok_or_else(|| serde::de::Error::custom("unsupported Whisper language code"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationLanguageInfo {
    pub code: String,
    pub name: String,
}
pub fn supported_dictation_languages() -> Vec<DictationLanguageInfo> {
    WHISPER_LANGUAGES
        .iter()
        .map(|(code, name)| DictationLanguageInfo {
            code: (*code).into(),
            name: (*name).into(),
        })
        .collect()
}
/// Whisper's multilingual vocabulary, shared by validation, worker and UI metadata.
/// `jw` and `yue` are Whisper's canonical codes (legacy Javanese and Cantonese).
pub const WHISPER_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("zh", "Chinese"),
    ("de", "German"),
    ("es", "Spanish"),
    ("ru", "Russian"),
    ("ko", "Korean"),
    ("fr", "French"),
    ("ja", "Japanese"),
    ("pt", "Portuguese"),
    ("tr", "Turkish"),
    ("pl", "Polish"),
    ("ca", "Catalan"),
    ("nl", "Dutch"),
    ("ar", "Arabic"),
    ("sv", "Swedish"),
    ("it", "Italian"),
    ("id", "Indonesian"),
    ("hi", "Hindi"),
    ("fi", "Finnish"),
    ("vi", "Vietnamese"),
    ("he", "Hebrew"),
    ("uk", "Ukrainian"),
    ("el", "Greek"),
    ("ms", "Malay"),
    ("cs", "Czech"),
    ("ro", "Romanian"),
    ("da", "Danish"),
    ("hu", "Hungarian"),
    ("ta", "Tamil"),
    ("no", "Norwegian"),
    ("th", "Thai"),
    ("ur", "Urdu"),
    ("hr", "Croatian"),
    ("bg", "Bulgarian"),
    ("lt", "Lithuanian"),
    ("la", "Latin"),
    ("mi", "Maori"),
    ("ml", "Malayalam"),
    ("cy", "Welsh"),
    ("sk", "Slovak"),
    ("te", "Telugu"),
    ("fa", "Persian"),
    ("lv", "Latvian"),
    ("bn", "Bengali"),
    ("sr", "Serbian"),
    ("az", "Azerbaijani"),
    ("sl", "Slovenian"),
    ("kn", "Kannada"),
    ("et", "Estonian"),
    ("mk", "Macedonian"),
    ("br", "Breton"),
    ("eu", "Basque"),
    ("is", "Icelandic"),
    ("hy", "Armenian"),
    ("ne", "Nepali"),
    ("mn", "Mongolian"),
    ("bs", "Bosnian"),
    ("kk", "Kazakh"),
    ("sq", "Albanian"),
    ("sw", "Swahili"),
    ("gl", "Galician"),
    ("mr", "Marathi"),
    ("pa", "Punjabi"),
    ("si", "Sinhala"),
    ("km", "Khmer"),
    ("sn", "Shona"),
    ("yo", "Yoruba"),
    ("so", "Somali"),
    ("af", "Afrikaans"),
    ("oc", "Occitan"),
    ("ka", "Georgian"),
    ("be", "Belarusian"),
    ("tg", "Tajik"),
    ("sd", "Sindhi"),
    ("gu", "Gujarati"),
    ("am", "Amharic"),
    ("yi", "Yiddish"),
    ("lo", "Lao"),
    ("uz", "Uzbek"),
    ("fo", "Faroese"),
    ("ht", "Haitian Creole"),
    ("ps", "Pashto"),
    ("tk", "Turkmen"),
    ("nn", "Nynorsk"),
    ("mt", "Maltese"),
    ("sa", "Sanskrit"),
    ("lb", "Luxembourgish"),
    ("my", "Myanmar"),
    ("bo", "Tibetan"),
    ("tl", "Tagalog"),
    ("mg", "Malagasy"),
    ("as", "Assamese"),
    ("tt", "Tatar"),
    ("haw", "Hawaiian"),
    ("ln", "Lingala"),
    ("ha", "Hausa"),
    ("ba", "Bashkir"),
    ("jw", "Javanese"),
    ("su", "Sundanese"),
    ("yue", "Cantonese"),
];

/// §8.4.4: how the transcription language is chosen. Parakeet supports only `Auto`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationLanguagePolicy {
    #[default]
    Layout,
    Fixed,
    Auto,
}

/// STT-004: where the chain is tapped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationTap {
    /// After Gain.
    Raw,
    /// After noise suppression, before expander and compressor.
    #[default]
    Clean,
    /// The chain output.
    Processed,
}

/// STT-012: how text reaches the target window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationInsert {
    /// Clipboard + Ctrl+V, the clipboard restored afterwards.
    #[default]
    Paste,
    /// Unicode `SendInput`; leaves the clipboard alone.
    Type,
}

/// STT-012: what happens when the target is a full-screen game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationGamePolicy {
    Insert,
    #[default]
    Copy,
    Ignore,
}

/// STT-016 / §9.9.3.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationPopupLook {
    /// A Wispr Flow-like pill with level bars.
    #[default]
    Flow,
    /// A vintage VU meter in the app's own style.
    Analog,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationPopupPosition {
    #[default]
    Bottom,
    Top,
}

/// The `[dictation]` preferences as the UI sees them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationPrefs {
    /// STT-001: off by default; off costs nothing.
    pub enabled: bool,
    pub engine: DictationEngine,
    /// Manifest id of the chosen model of `engine`; `null` until one is downloaded.
    pub model: Option<String>,
    pub language: DictationLanguagePolicy,
    pub fixed_language: DictationLanguage,
    pub tap: DictationTap,
    /// STT-006.
    pub mute_mic_while_dictating: bool,
    pub insert: DictationInsert,
    pub trailing_space: bool,
    pub fullscreen_games: DictationGamePolicy,
    /// STT-007: seconds after the last use; 0 = immediately, -1 = never.
    pub unload_after_s: i32,
    /// STT-008: inference threads; 0 = max(2, physical cores / 2).
    pub threads: u16,
    pub game_mode: bool,
    /// STT-020: Whisper runs on the GPU (Vulkan) when one is available; Parakeet stays on the
    /// CPU. A failed GPU load falls back to the CPU.
    #[serde(default = "default_true")]
    pub gpu: bool,
    /// STT-005.
    pub pre_roll: bool,
    /// STT-015.
    pub keep_history: bool,
    /// STT-015: keep the history across restarts in a file of the user's profile; off keeps it
    /// in memory until Macafon quits (owner request, 2026-10-19).
    #[serde(default)]
    pub save_history: bool,
    /// STT-014: Whisper only.
    pub vocabulary: Vec<String>,
    pub popup_look: DictationPopupLook,
    pub popup_position: DictationPopupPosition,
    /// Toward the center, percent of the monitor height (`OSD_MAX_OFFSET_PCT` bound).
    pub popup_offset_pct: f32,
    /// The popup stays on the primary monitor instead of following the active window's one;
    /// independent of the hotkey popup's setting.
    #[serde(default)]
    pub popup_primary_monitor: bool,
    /// §9.9.2: the Dictation tile in the tray panel and the hotkey overlay.
    pub show_in_overlay: bool,
}

fn default_true() -> bool {
    true
}

/// `dictation.set_prefs`: absent fields stay. The models folder has its own request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationPrefsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<DictationEngine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<DictationLanguagePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_language: Option<DictationLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap: Option<DictationTap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute_mic_while_dictating: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert: Option<DictationInsert>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_space: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fullscreen_games: Option<DictationGamePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unload_after_s: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_roll: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_history: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_history: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocabulary: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup_look: Option<DictationPopupLook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup_position: Option<DictationPopupPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup_offset_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup_primary_monitor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_in_overlay: Option<bool>,
}

/// `dictation.state` of §8.4.3.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationPhase {
    /// The feature is off: no tap, no worker, no hotkeys.
    #[default]
    Off,
    /// On, the worker is not running.
    Unloaded,
    /// The worker is loading the model.
    Loading,
    /// The worker holds the model and waits.
    Ready,
    Listening,
    Transcribing,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationModelStatus {
    #[default]
    Missing,
    Downloading,
    /// Downloaded and verified.
    Ready,
    Error,
}

/// One manifest entry (§8.4.1) with its local state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationModel {
    /// `whisper-large-v3-turbo-q5_0`, `whisper-small-q5_1`, `parakeet-tdt-0.6b-v3-int8`.
    pub id: String,
    pub engine: DictationEngine,
    /// Short UI name: `Turbo`, `Small`, `Parakeet v3`.
    pub name: String,
    /// Total download, VAD included.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size_bytes: u64,
    /// Static manifest ratings 1…5 for the two meters of card 01.
    pub quality: u8,
    pub speed: u8,
    /// The model the UI recommends on an empty state.
    pub recommended: bool,
    pub status: DictationModelStatus,
    /// While downloading.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub downloaded_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub bytes_per_sec: Option<u64>,
    pub error: Option<String>,
}

/// Moving the models to another folder (STT-022).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationMove {
    pub to: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub done_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total_bytes: u64,
}

/// The models folder (STT-022).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationStorage {
    /// Absolute path.
    pub folder: String,
    /// `%LOCALAPPDATA%\Macafon\models\stt`, not picked by the user.
    pub default_folder: bool,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub free_bytes: Option<u64>,
    /// Bytes of downloaded models in the folder.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub used_bytes: u64,
    /// The folder is on a spinning disk: cold starts are slower.
    pub rotational: bool,
    pub moving: Option<DictationMove>,
}

/// Where a result went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DictationOutcome {
    Inserted,
    Copied,
    /// The TRY IT panel: shown in the UI only.
    Test,
}

/// One transcription (STT-015); text stays in daemon memory only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationResult {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: u64,
    pub text: String,
    /// `null` when the engine chose it itself (Parakeet).
    pub language: Option<DictationLanguage>,
    pub engine: DictationEngine,
    pub model: String,
    /// Recorded audio.
    pub audio_ms: u32,
    /// Key release → text ready.
    pub elapsed_ms: u32,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at_unix_ms: u64,
    pub outcome: DictationOutcome,
}

/// Everything the Dictation screen shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationState {
    pub prefs: DictationPrefs,
    #[serde(default)]
    pub layout_languages: Vec<DictationLanguageInfo>,
    #[serde(default)]
    pub supported_languages: Vec<DictationLanguageInfo>,
    pub phase: DictationPhase,
    /// Short reason while `phase` is `error`.
    pub error: Option<String>,
    /// The model the worker holds now.
    pub loaded_model: Option<String>,
    /// Worker memory while it runs.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub worker_ram_bytes: Option<u64>,
    /// Threads the worker uses now.
    pub worker_threads: Option<u16>,
    /// The thread count `threads = 0` resolves to, and the slider's upper bound.
    pub default_threads: u16,
    pub logical_cores: u16,
    pub models: Vec<DictationModel>,
    pub storage: DictationStorage,
    /// Newest first; empty when `keep_history` is off.
    pub history: Vec<DictationResult>,
}

/// The tap level for the popup and the INPUT card meter, about 60 Hz while listening or while
/// the UI is subscribed to meters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DictationLevel {
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
}

/// Read installed keyboard layouts without changing the user's input language.
pub fn installed_dictation_languages() -> Vec<DictationLanguageInfo> {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetKeyboardLayoutList(count: i32, layouts: *mut isize) -> i32;
        }
        // SAFETY: a null buffer queries the size; the second call is bounded by the allocation.
        let count = unsafe { GetKeyboardLayoutList(0, std::ptr::null_mut()) };
        if count <= 0 {
            return Vec::new();
        }
        let mut layouts = vec![0isize; count as usize];
        // SAFETY: layouts has count initialized writable HKL slots.
        let written = unsafe { GetKeyboardLayoutList(count, layouts.as_mut_ptr()) };
        let mut result: Vec<DictationLanguageInfo> = Vec::new();
        for layout in layouts.into_iter().take(written.max(0) as usize) {
            if let Some(language) = dictation_layout_language(layout as usize)
                && !result.iter().any(|info| info.code == language.code())
                && let Some((code, name)) = WHISPER_LANGUAGES
                    .iter()
                    .find(|(code, _)| *code == language.code())
            {
                result.push(DictationLanguageInfo {
                    code: (*code).into(),
                    name: (*name).into(),
                });
            }
        }
        result
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

pub fn dictation_layout_language(lang_id: usize) -> Option<DictationLanguage> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLocaleInfoW(locale: u32, kind: u32, data: *mut u16, count: i32) -> i32;
        }
        let mut code = [0u16; 16];
        // SAFETY: read-only locale query with a fixed-size writable UTF-16 buffer.
        let count = unsafe {
            GetLocaleInfoW(
                (lang_id & 0xffff) as u32,
                0x59,
                code.as_mut_ptr(),
                code.len() as i32,
            )
        };
        if count <= 1 {
            return None;
        }
        let code = String::from_utf16_lossy(&code[..(count as usize - 1)]);
        DictationLanguage::from_code(if code == "jv" { "jw" } else { &code })
    }
    #[cfg(not(windows))]
    {
        // Portable fallback for callers inspecting well-known LANGIDs; layout enumeration is empty.
        DictationLanguage::from_code(match lang_id & 0x3ff {
            0x09 => "en",
            0x22 => "uk",
            0x19 => "ru",
            0x07 => "de",
            0x0c => "fr",
            0x0a => "es",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_codes_preserve_old_configs_and_validate_whisper_languages() {
        for code in ["en", "uk", "ru", "de", "fr", "ja", "yue"] {
            let json = format!("\"{code}\"");
            let language: DictationLanguage = serde_json::from_str(&json).expect("supported code");
            assert_eq!(serde_json::to_string(&language).expect("encode"), json);
        }
        assert!(serde_json::from_str::<DictationLanguage>("\"xx\"").is_err());
    }
}
