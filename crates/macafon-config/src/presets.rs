//! User microphone presets: `$XDG_CONFIG_HOME/macafon/presets/mic/<slug>.json` (PRD §6.3.10, §13).
//!
//! The same JSON is the `.macafon-preset.json` export format. Presets are loaded once at
//! startup and written through on every change; mute and bypass are never stored.

use crate::{Error, write_atomically};
use macafon_dsp::{ChainSettings, ParameterError, Preset};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SCHEMA_VERSION: u32 = 1;
/// Ids of user presets start with this, so they never collide with built-in ids.
pub const USER_PREFIX: &str = "user:";
pub const MAX_NAME_CHARS: usize = 64;
const MAX_SLUG_CHARS: usize = 40;

/// One preset file, also the import/export format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    pub schema_version: u32,
    pub name: String,
    pub settings: ChainSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserPreset {
    /// `user:<slug>`; the file is `<slug>.json`. Stable across renames.
    pub id: String,
    pub name: String,
    pub settings: ChainSettings,
}

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("preset names must have 1–{MAX_NAME_CHARS} characters and no control characters")]
    InvalidName,
    #[error("a preset named {0:?} already exists")]
    DuplicateName(String),
    #[error("no user preset {0:?}")]
    NotFound(String),
    #[error("built-in presets are read-only; save as a new preset")]
    BuiltIn,
    #[error("not a Macafon preset: {0}")]
    Malformed(String),
    #[error("unsupported preset schema_version {0} (expected {SCHEMA_VERSION})")]
    UnsupportedVersion(u32),
    #[error(transparent)]
    Invalid(#[from] ParameterError),
    #[error(transparent)]
    Storage(#[from] Error),
}

/// Presets keep the sound only: mute and bypass belong to the live state.
fn sound_only(settings: &ChainSettings) -> ChainSettings {
    ChainSettings {
        muted: false,
        bypass: false,
        ..*settings
    }
}

/// True when `settings` sound the same as `preset` (ignoring mute and bypass).
pub fn same_sound(settings: &ChainSettings, preset: &ChainSettings) -> bool {
    sound_only(settings) == sound_only(preset)
}

fn check_name(name: &str) -> Result<String, PresetError> {
    let name = name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(PresetError::InvalidName);
    }
    Ok(name.to_owned())
}

fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= MAX_SLUG_CHARS {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "preset".to_owned()
    } else {
        slug.to_owned()
    }
}

pub struct PresetStore {
    /// `None` keeps presets in memory only (no config directory).
    directory: Option<PathBuf>,
    presets: Vec<UserPreset>,
}

impl PresetStore {
    pub fn in_memory() -> Self {
        Self {
            directory: None,
            presets: Vec::new(),
        }
    }

    /// Load every `*.json` in `directory`; unreadable or invalid files are skipped and
    /// reported, never deleted.
    pub fn open(directory: impl Into<PathBuf>) -> (Self, Vec<(PathBuf, String)>) {
        let directory = directory.into();
        let mut store = Self {
            directory: Some(directory.clone()),
            presets: Vec::new(),
        };
        let mut skipped = Vec::new();
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (store, skipped);
            }
            Err(error) => {
                skipped.push((directory, error.to_string()));
                return (store, skipped);
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        for path in paths {
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let loaded = std::fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|text| parse(&text).map_err(|error| error.to_string()));
            match loaded {
                Ok(file) if store.name_taken(&file.name, None) => {
                    skipped.push((path, format!("duplicate name {:?}", file.name)));
                }
                Ok(file) => store.presets.push(UserPreset {
                    id: format!("{USER_PREFIX}{stem}"),
                    name: file.name,
                    settings: file.settings,
                }),
                Err(reason) => skipped.push((path, reason)),
            }
        }
        (store, skipped)
    }

    pub fn list(&self) -> &[UserPreset] {
        &self.presets
    }

    pub fn get(&self, id: &str) -> Option<&UserPreset> {
        self.presets.iter().find(|preset| preset.id == id)
    }

    /// Save `settings` as a new preset named `name`.
    pub fn create(
        &mut self,
        name: &str,
        settings: &ChainSettings,
    ) -> Result<&UserPreset, PresetError> {
        let name = check_name(name)?;
        if self.name_taken(&name, None) {
            return Err(PresetError::DuplicateName(name));
        }
        self.insert(name, settings)
    }

    /// Overwrite a user preset's settings (Save).
    pub fn update(&mut self, id: &str, settings: &ChainSettings) -> Result<(), PresetError> {
        settings.validate()?;
        let index = self.index(id)?;
        let mut preset = self.presets[index].clone();
        preset.settings = sound_only(settings);
        self.write(&preset)?;
        self.presets[index] = preset;
        Ok(())
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), PresetError> {
        let name = check_name(name)?;
        let index = self.index(id)?;
        if self.name_taken(&name, Some(id)) {
            return Err(PresetError::DuplicateName(name));
        }
        let mut preset = self.presets[index].clone();
        preset.name = name;
        self.write(&preset)?;
        self.presets[index] = preset;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), PresetError> {
        let index = self.index(id)?;
        if let Some(path) = self.path(id) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(Error::Io { path, source }.into()),
            }
        }
        self.presets.remove(index);
        Ok(())
    }

    /// Add a copy under a free name based on `name`: "Name", "Name (2)", … (Duplicate, Import).
    pub fn add_unique(
        &mut self,
        name: &str,
        settings: &ChainSettings,
    ) -> Result<&UserPreset, PresetError> {
        let base = check_name(name)?;
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    let suffix = format!(" ({n})");
                    let kept: String = base
                        .chars()
                        .take(MAX_NAME_CHARS - suffix.chars().count())
                        .collect();
                    format!("{}{suffix}", kept.trim_end())
                }
            })
            .find(|name| !self.name_taken(name, None))
            .unwrap_or(base);
        self.insert(name, settings)
    }

    /// Import a `.macafon-preset.json` document; a taken name gets a numeric suffix.
    pub fn import(&mut self, file: &PresetFile) -> Result<&UserPreset, PresetError> {
        if file.schema_version != SCHEMA_VERSION {
            return Err(PresetError::UnsupportedVersion(file.schema_version));
        }
        self.add_unique(&file.name, &file.settings)
    }

    fn insert(
        &mut self,
        name: String,
        settings: &ChainSettings,
    ) -> Result<&UserPreset, PresetError> {
        settings.validate()?;
        let base = slug(&name);
        let slug = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|slug| {
                let id = format!("{USER_PREFIX}{slug}");
                self.get(&id).is_none() && self.path(&id).is_none_or(|path| !path.exists())
            })
            .unwrap_or(base);
        let preset = UserPreset {
            id: format!("{USER_PREFIX}{slug}"),
            name,
            settings: sound_only(settings),
        };
        self.write(&preset)?;
        self.presets.push(preset);
        Ok(self.presets.last().expect("just pushed"))
    }

    /// Names are unique among user and built-in presets, ignoring case.
    fn name_taken(&self, name: &str, except: Option<&str>) -> bool {
        let lower = name.to_lowercase();
        Preset::ALL
            .iter()
            .any(|preset| preset.name().to_lowercase() == lower)
            || self.presets.iter().any(|preset| {
                Some(preset.id.as_str()) != except && preset.name.to_lowercase() == lower
            })
    }

    fn index(&self, id: &str) -> Result<usize, PresetError> {
        if Preset::from_id(id).is_some() {
            return Err(PresetError::BuiltIn);
        }
        self.presets
            .iter()
            .position(|preset| preset.id == id)
            .ok_or_else(|| PresetError::NotFound(id.to_owned()))
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        let slug = id.strip_prefix(USER_PREFIX)?;
        Some(self.directory.as_ref()?.join(format!("{slug}.json")))
    }

    fn write(&self, preset: &UserPreset) -> Result<(), Error> {
        let Some(path) = self.path(&preset.id) else {
            return Ok(());
        };
        let mut text = serde_json::to_string_pretty(&export(&preset.name, &preset.settings))
            .map_err(|error| Error::Io {
                path: path.clone(),
                source: std::io::Error::other(error),
            })?;
        text.push('\n');
        write_atomically(&path, &[text.as_bytes()])
    }
}

/// The export document of a preset.
pub fn export(name: &str, settings: &ChainSettings) -> PresetFile {
    PresetFile {
        schema_version: SCHEMA_VERSION,
        name: name.to_owned(),
        settings: sound_only(settings),
    }
}

/// Parse and validate a preset document.
pub fn parse(text: &str) -> Result<PresetFile, PresetError> {
    let file: PresetFile =
        serde_json::from_str(text).map_err(|error| PresetError::Malformed(error.to_string()))?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(PresetError::UnsupportedVersion(file.schema_version));
    }
    check_name(&file.name)?;
    file.settings.validate()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("macafon-presets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn presets_persist_across_reloads_and_keep_their_id_when_renamed() {
        let dir = directory("persist");
        let (mut store, skipped) = PresetStore::open(&dir);
        assert!(skipped.is_empty() && store.list().is_empty());
        let mut settings = Preset::Broadcast.settings();
        settings.muted = true;
        let id = store
            .create("  My Voice!  ", &settings)
            .expect("create")
            .id
            .clone();
        assert_eq!(id, "user:my-voice");
        assert!(matches!(
            store.create("my voice!", &settings),
            Err(PresetError::DuplicateName(_))
        ));
        assert!(matches!(
            store.create("Broadcast", &settings),
            Err(PresetError::DuplicateName(_))
        ));
        assert!(matches!(
            store.rename("broadcast", "x"),
            Err(PresetError::BuiltIn)
        ));
        store.rename(&id, "Late night").expect("rename");
        settings.gain.params.gain_db = 3.0;
        store.update(&id, &settings).expect("update");
        let copy = store
            .add_unique("Late night", &settings)
            .expect("copy")
            .id
            .clone();
        assert_eq!(
            store.get(&copy).map(|p| p.name.as_str()),
            Some("Late night (2)")
        );
        // A broken file next to valid ones is reported and left alone.
        std::fs::write(dir.join("broken.json"), "{").expect("write");

        let (reloaded, skipped) = PresetStore::open(&dir);
        assert_eq!(skipped.len(), 1);
        let preset = reloaded.get(&id).expect("reloaded");
        assert_eq!(preset.name, "Late night");
        assert_eq!(preset.settings.gain.params.gain_db, 3.0);
        assert!(!preset.settings.muted, "mute is not part of a preset");
        assert!(same_sound(&settings, &preset.settings));
        assert!(dir.join("broken.json").exists());

        let mut reloaded = reloaded;
        reloaded.delete(&id).expect("delete");
        assert!(!dir.join("my-voice.json").exists());
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn exported_presets_import_under_a_free_name() {
        let mut store = PresetStore::in_memory();
        let file = export("Natural", &Preset::Natural.settings());
        let text = serde_json::to_string(&file).expect("serialize");
        let parsed = parse(&text).expect("valid");
        assert_eq!(store.import(&parsed).expect("import").name, "Natural (2)");
        let mut future = parsed.clone();
        future.schema_version = 2;
        assert!(matches!(
            store.import(&future),
            Err(PresetError::UnsupportedVersion(2))
        ));
        let mut invalid = parsed;
        invalid.settings.gain.params.gain_db = 99.0;
        assert!(matches!(
            store.import(&invalid),
            Err(PresetError::Invalid(_))
        ));
        assert_eq!(slug("Ünïcode only ✨"), "n-code-only");
        assert_eq!(slug("✨"), "preset");
    }
}
