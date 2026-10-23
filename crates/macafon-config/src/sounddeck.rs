//! `[sounddeck]`: the sound library and how it plays (PRD §7.6).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-sound and output volumes go up to twice the file's level.
pub const SOUNDDECK_MAX_VOLUME_PCT: f32 = 200.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundDeckOverlayView {
    #[default]
    Grid,
    List,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundDeckOverlayControls {
    #[default]
    Top,
    Bottom,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundDeckConfig {
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
    #[serde(default)]
    pub overlay_view: SoundDeckOverlayView,
    #[serde(default)]
    pub overlay_controls: SoundDeckOverlayControls,
    pub looping: bool,
    /// Output volumes are trims around the normalized level (owner request, 2026-10-18).
    /// Missing in older configs, whose volumes compensated for unnormalized sounds.
    #[serde(default)]
    pub normalized_levels: bool,
    /// `[sounddeck.sounds."Memes/airhorn.mp3"]`: only sounds with non-default settings.
    pub sounds: BTreeMap<String, SoundConfig>,
}

impl Default for SoundDeckConfig {
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
            overlay_view: SoundDeckOverlayView::default(),
            overlay_controls: SoundDeckOverlayControls::default(),
            looping: false,
            normalized_levels: true,
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

impl SoundDeckConfig {
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

    /// Once: volumes set before loudness normalization (a headphones level of 1 % that tamed a
    /// full-scale stream) go back to the defaults, which now mean "as loud as the reference".
    pub fn normalize_levels(&mut self) -> bool {
        if self.normalized_levels {
            return false;
        }
        let defaults = Self::default();
        self.microphone_volume_pct = defaults.microphone_volume_pct;
        self.headphones_volume_pct = defaults.headphones_volume_pct;
        self.normalized_levels = true;
        true
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
        volume_pct.clamp(0.0, SOUNDDECK_MAX_VOLUME_PCT)
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
        let mut config = SoundDeckConfig::default();
        config.edit_sound("a.mp3", |sound| sound.volume_pct = 500.0);
        assert_eq!(config.sound("a.mp3").volume_pct, SOUNDDECK_MAX_VOLUME_PCT);
        config.rename_sound("a.mp3", "Memes/b.mp3");
        assert_eq!(config.sound("Memes/b.mp3").volume_pct, 200.0);
        config.edit_sound("Memes/b.mp3", |sound| sound.volume_pct = 100.0);
        assert!(config.sounds.is_empty());
    }

    #[test]
    fn levels_from_before_normalization_reset_once() {
        let mut old: SoundDeckConfig = toml::from_str(
            "headphones_volume_pct = 1.0
microphone_volume_pct = 40.0
",
        )
        .expect("old sounddeck section");
        assert!(old.normalize_levels());
        assert_eq!(old.headphones_volume_pct, 70.0);
        assert_eq!(old.microphone_volume_pct, 100.0);
        old.headphones_volume_pct = 5.0;
        assert!(!old.normalize_levels());
        assert_eq!(old.headphones_volume_pct, 5.0);
        assert!(!SoundDeckConfig::default().normalize_levels());
    }
}
