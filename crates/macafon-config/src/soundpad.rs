//! `[soundpad]`: the sound library and how it plays (PRD §7.6).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-sound and output volumes go up to twice the file's level.
pub const SOUNDPAD_MAX_VOLUME_PCT: f32 = 200.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundpadConfig {
    /// Library folder; unset uses [`default_folder`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    pub to_microphone: bool,
    pub to_headphones: bool,
    pub microphone_volume_pct: f32,
    pub headphones_volume_pct: f32,
    pub mute_voice_while_playing: bool,
    /// Monitor output device; unset follows the default output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor_device: Option<String>,
    pub show_in_overlay: bool,
    pub looping: bool,
    /// `[soundpad.sounds."Memes/airhorn.mp3"]`: only sounds with non-default settings.
    pub sounds: BTreeMap<String, SoundConfig>,
}

impl Default for SoundpadConfig {
    fn default() -> Self {
        Self {
            folder: None,
            to_microphone: true,
            to_headphones: true,
            microphone_volume_pct: 100.0,
            headphones_volume_pct: 70.0,
            mute_voice_while_playing: false,
            monitor_device: None,
            show_in_overlay: true,
            looping: false,
            sounds: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundConfig {
    pub volume_pct: f32,
    pub favorite: bool,
    pub play_count: u32,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            volume_pct: 100.0,
            favorite: false,
            play_count: 0,
        }
    }
}

impl SoundConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl SoundpadConfig {
    pub fn folder(&self) -> PathBuf {
        self.folder.clone().unwrap_or_else(default_folder)
    }

    pub fn sound(&self, id: &str) -> SoundConfig {
        self.sounds.get(id).cloned().unwrap_or_default()
    }

    /// Change one sound's settings, dropping the entry once it is back to the defaults.
    pub fn edit_sound(&mut self, id: &str, edit: impl FnOnce(&mut SoundConfig)) {
        let mut sound = self.sound(id);
        edit(&mut sound);
        sound.volume_pct = clamp_volume(sound.volume_pct);
        if sound.is_default() {
            self.sounds.remove(id);
        } else {
            self.sounds.insert(id.to_owned(), sound);
        }
    }

    /// A sound was renamed or moved: its settings and hotkeys follow it.
    pub fn rename_sound(&mut self, from: &str, to: &str) {
        if let Some(sound) = self.sounds.remove(from) {
            self.sounds.insert(to.to_owned(), sound);
        }
    }
}

/// Out-of-range and non-finite volumes become the nearest valid one (100 % for NaN).
pub fn clamp_volume(volume_pct: f32) -> f32 {
    if volume_pct.is_finite() {
        volume_pct.clamp(0.0, SOUNDPAD_MAX_VOLUME_PCT)
    } else {
        100.0
    }
}

/// `Music\Macafon Sounds` in the user's profile: easy to find and to fill from the file
/// manager.
pub fn default_folder() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    Path::new(&home).join("Music").join("Macafon Sounds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_settings_drop_defaults_and_follow_renames() {
        let mut config = SoundpadConfig::default();
        config.edit_sound("a.mp3", |sound| sound.volume_pct = 500.0);
        assert_eq!(config.sound("a.mp3").volume_pct, SOUNDPAD_MAX_VOLUME_PCT);
        config.rename_sound("a.mp3", "Memes/b.mp3");
        assert_eq!(config.sound("Memes/b.mp3").volume_pct, 200.0);
        config.edit_sound("Memes/b.mp3", |sound| sound.volume_pct = 100.0);
        assert!(config.sounds.is_empty());
    }
}
