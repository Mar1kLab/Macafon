//! Daemon configuration: `$XDG_CONFIG_HOME/macafon/config.toml` (PRD §13).

pub mod mixer;
pub mod presets;

pub use mixer::{AppRule, AppsConfig, ChannelConfig, MixerConfig};

use macafon_dsp::{ChainSettings, InputChannel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub mic: MicConfig,
    pub device_profiles: BTreeMap<String, MicProfile>,
    pub mixer: MixerConfig,
    pub apps: AppsConfig,
    pub ui: UiConfig,
    /// `[hotkeys]`: action id → shortcut, `mic_mute_toggle = "Ctrl+Alt+M"` (§8.3).
    pub hotkeys: BTreeMap<String, String>,
}

/// UI facts the daemon remembers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// BG-001: the "keeps running in the tray" notification was shown once.
    pub close_hint_shown: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            mic: MicConfig::default(),
            device_profiles: BTreeMap::new(),
            mixer: MixerConfig::default(),
            apps: AppsConfig::default(),
            ui: UiConfig::default(),
            hotkeys: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MicConfig {
    /// Duplicate processed mono audio into FL and FR; applied on daemon startup.
    pub stereo: bool,
    pub input_channel: InputChannel,
    /// PipeWire `node.name` of the physical microphone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Optional second-priority physical microphone by PipeWire `node.name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_device: Option<String>,
    /// Preset the chain was last loaded from (`macafon_dsp::Preset::id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Noise floor of the physical microphone measured by the last calibration (CAL-001).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_floor_dbfs: Option<f32>,
    /// Physical microphone that the persisted noise floor was measured on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_floor_device: Option<String>,
    #[serde(flatten)]
    pub chain: ChainSettings,
}

impl Default for MicConfig {
    fn default() -> Self {
        let preset = macafon_dsp::Preset::Natural;
        Self {
            stereo: false,
            input_channel: InputChannel::Auto,
            device: None,
            fallback_device: None,
            preset: Some(preset.id().to_owned()),
            noise_floor_dbfs: None,
            noise_floor_device: None,
            chain: preset.settings(),
        }
    }
}

/// The sound settings that follow one physical microphone, independent of input priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicProfile {
    pub chain: ChainSettings,
    pub preset: Option<String>,
    pub noise_floor_dbfs: Option<f32>,
    #[serde(default)]
    pub input_channel: InputChannel,
}

impl MicProfile {
    pub fn capture(mic: &MicConfig) -> Self {
        Self {
            chain: mic.chain,
            preset: mic.preset.clone(),
            noise_floor_dbfs: mic.noise_floor_dbfs,
            input_channel: mic.input_channel,
        }
    }
    pub fn apply(&self, mic: &mut MicConfig, device: &str) {
        mic.chain = self.chain;
        mic.preset = self.preset.clone();
        mic.noise_floor_dbfs = self.noise_floor_dbfs;
        mic.noise_floor_device = self.noise_floor_dbfs.map(|_| device.to_owned());
        mic.input_channel = self.input_channel;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot determine the config directory: neither XDG_CONFIG_HOME nor HOME is set")]
    NoConfigDir,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Result of loading: a usable config plus where a broken file was moved, if any (CFG-002).
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub recovered: Option<Recovered>,
}

#[derive(Debug)]
pub struct Recovered {
    pub backup: PathBuf,
    pub reason: String,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_CONFIG_HOME/macafon/config.toml`, falling back to `~/.config`.
    pub fn default_location() -> Result<Self, Error> {
        Ok(Self::new(config_dir()?.join("config.toml")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Missing file → defaults. Unreadable, invalid or newer-schema file → defaults, and the
    /// original is kept as `config.toml.broken-<unix time>` so nothing is lost.
    pub fn load(&self) -> Result<Loaded, Error> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Loaded {
                    config: Config::default(),
                    recovered: None,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                return self.recover(error.to_string());
            }
            Err(source) => {
                return Err(Error::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        match parse(&text) {
            Ok(config) => {
                let old_version = text
                    .parse::<toml::Value>()
                    .ok()
                    .and_then(|value| {
                        value
                            .get("schema_version")
                            .and_then(toml::Value::as_integer)
                    })
                    .unwrap_or(1);
                if old_version < i64::from(SCHEMA_VERSION) {
                    let mut name = self.path.file_name().unwrap_or_default().to_owned();
                    name.push(".bak");
                    let backup = self.path.with_file_name(name);
                    write_atomically(&backup, &[text.as_bytes()])?;
                    self.save(&config)?;
                }
                Ok(Loaded {
                    config,
                    recovered: None,
                })
            }
            Err(reason) => self.recover(reason),
        }
    }

    fn recover(&self, reason: String) -> Result<Loaded, Error> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let mut name = self.path.file_name().unwrap_or_default().to_owned();
        name.push(format!(".broken-{seconds}"));
        let backup = self.path.with_file_name(name);
        std::fs::rename(&self.path, &backup).map_err(|source| Error::Io {
            path: backup.clone(),
            source,
        })?;
        Ok(Loaded {
            config: Config::default(),
            recovered: Some(Recovered { backup, reason }),
        })
    }

    /// Atomic replace (CFG-001).
    pub fn save(&self, config: &Config) -> Result<(), Error> {
        let text = toml::to_string_pretty(config)?;
        write_atomically(&self.path, &[HEADER.as_bytes(), text.as_bytes()])
    }
}

/// `$XDG_CONFIG_HOME/macafon`, falling back to `~/.config/macafon`.
pub fn config_dir() -> Result<PathBuf, Error> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
        .ok_or(Error::NoConfigDir)?;
    Ok(base.join("macafon"))
}

/// Write a temporary file in the same directory, fsync, rename over `path`.
pub(crate) fn write_atomically(path: &Path, parts: &[&[u8]]) -> Result<(), Error> {
    let io = |source| Error::Io {
        path: path.to_owned(),
        source,
    };
    let directory = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory).map_err(io)?;
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".tmp");
    let temporary = path.with_file_name(name);
    let mut file = std::fs::File::create(&temporary).map_err(io)?;
    for part in parts {
        file.write_all(part).map_err(io)?;
    }
    file.sync_all().map_err(io)?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(io)
}

const HEADER: &str = "# Macafon configuration. Written by macafond; edits are picked up on restart.\n\
# Percent values (intensity, …) use 0–100; levels are dB/dBFS; times are ms.\n\n";

fn parse(text: &str) -> Result<Config, String> {
    let mut config: Config = toml::from_str(text).map_err(|error| error.to_string())?;
    if config.schema_version == 1 {
        config.schema_version = SCHEMA_VERSION;
    }
    if config.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {} (expected {SCHEMA_VERSION})",
            config.schema_version
        ));
    }
    config
        .mic
        .chain
        .validate()
        .map_err(|error| error.to_string())?;
    config.mixer.validate()?;
    for profile in config.device_profiles.values() {
        profile
            .chain
            .validate()
            .map_err(|error| error.to_string())?;
        if profile
            .noise_floor_dbfs
            .is_some_and(|floor| !floor.is_finite())
        {
            return Err("device noise floor must be finite".into());
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use macafon_dsp::Preset;

    fn temporary_store(name: &str) -> (PathBuf, Store) {
        let directory =
            std::env::temp_dir().join(format!("macafon-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let store = Store::new(directory.join("macafon").join("config.toml"));
        (directory, store)
    }

    #[test]
    fn saved_config_loads_back_and_missing_file_gives_defaults() {
        let (directory, store) = temporary_store("round-trip");
        let loaded = store.load().expect("missing file is fine");
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.recovered.is_none());
        let mut chain = Preset::Broadcast.settings();
        chain.expander.params.ratio = f32::INFINITY;
        let config = Config {
            schema_version: SCHEMA_VERSION,
            device_profiles: BTreeMap::new(),
            mixer: MixerConfig::default(),
            apps: AppsConfig::default(),
            ui: UiConfig::default(),
            hotkeys: BTreeMap::new(),
            mic: MicConfig {
                stereo: true,
                input_channel: InputChannel::Right,
                device: Some("alsa_input.usb-Yeti".into()),
                fallback_device: Some("alsa_input.internal".into()),
                preset: Some("broadcast".into()),
                noise_floor_dbfs: Some(-61.5),
                noise_floor_device: Some("alsa_input.usb-Yeti".into()),
                chain,
            },
        };
        store.save(&config).expect("save");
        let text = std::fs::read_to_string(store.path()).expect("written");
        assert!(
            text.starts_with("# Macafon") && text.contains("[mic.compressor]"),
            "{text}"
        );
        assert_eq!(store.load().expect("load").config, config);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn migration_keeps_original_and_device_profiles_round_trip() {
        let (directory, store) = temporary_store("migration");
        std::fs::create_dir_all(store.path().parent().expect("parent")).expect("mkdir");
        let old = "schema_version = 1\n[mic]\ndevice = 'usb-mic'\n";
        std::fs::write(store.path(), old).expect("write old config");
        let mut loaded = store.load().expect("migrate");
        assert_eq!(loaded.config.schema_version, SCHEMA_VERSION);
        assert_eq!(
            std::fs::read_to_string(store.path().with_extension("toml.bak")).expect("backup"),
            old
        );
        loaded.config.mic.input_channel = InputChannel::Right;
        loaded.config.device_profiles.insert(
            "serial:usb-mic".into(),
            MicProfile::capture(&loaded.config.mic),
        );
        store.save(&loaded.config).expect("save profile");
        assert_eq!(store.load().expect("reload").config, loaded.config);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn broken_or_invalid_config_is_backed_up_and_replaced_by_defaults() {
        let (directory, store) = temporary_store("broken");
        let parent = store.path().parent().expect("has parent");
        std::fs::create_dir_all(parent).expect("mkdir");
        for text in [
            "schema_version = 1\n[mic\n",
            "schema_version = 1\n[mic.gain]\nenabled = true\ngain_db = 99\n",
            "schema_version = 7\n",
        ] {
            std::fs::write(store.path(), text).expect("write");
            let loaded = store.load().expect("recovers");
            assert_eq!(loaded.config, Config::default());
            let recovered = loaded.recovered.expect("reported");
            assert_eq!(
                std::fs::read_to_string(&recovered.backup).expect("kept"),
                text
            );
            assert!(!store.path().exists());
            std::fs::remove_file(recovered.backup).expect("cleanup backup");
        }
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
