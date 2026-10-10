//! Daemon configuration: `$XDG_CONFIG_HOME/miceq/config.toml` (PRD §13).

use miceq_dsp::ChainSettings;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub mic: MicConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            mic: MicConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MicConfig {
    /// PipeWire `node.name` of the physical microphone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Preset the chain was last loaded from (`miceq_dsp::Preset::id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Noise floor of the physical microphone measured by the last calibration (CAL-001).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_floor_dbfs: Option<f32>,
    #[serde(flatten)]
    pub chain: ChainSettings,
}

impl Default for MicConfig {
    fn default() -> Self {
        let preset = miceq_dsp::Preset::Natural;
        Self {
            device: None,
            preset: Some(preset.id().to_owned()),
            noise_floor_dbfs: None,
            chain: preset.settings(),
        }
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

    /// `$XDG_CONFIG_HOME/miceq/config.toml`, falling back to `~/.config`.
    pub fn default_location() -> Result<Self, Error> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
            .ok_or(Error::NoConfigDir)?;
        Ok(Self::new(base.join("miceq").join("config.toml")))
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
            Ok(config) => Ok(Loaded {
                config,
                recovered: None,
            }),
            Err(reason) => self.recover(reason),
        }
    }

    fn recover(&self, reason: String) -> Result<Loaded, Error> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
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

    /// Atomic replace: write a temporary file in the same directory, fsync, rename (CFG-001).
    pub fn save(&self, config: &Config) -> Result<(), Error> {
        let text = toml::to_string_pretty(config)?;
        let io = |source| Error::Io {
            path: self.path.clone(),
            source,
        };
        let directory = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(directory).map_err(io)?;
        let mut name = self.path.file_name().unwrap_or_default().to_owned();
        name.push(".tmp");
        let temporary = self.path.with_file_name(name);
        let mut file = std::fs::File::create(&temporary).map_err(io)?;
        file.write_all(HEADER.as_bytes()).map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);
        std::fs::rename(&temporary, &self.path).map_err(io)
    }
}

const HEADER: &str = "# micEQ configuration. Written by miceqd; edits are picked up on restart.\n\
# Percent values (intensity, …) use 0–100; levels are dB/dBFS; times are ms.\n\n";

fn parse(text: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(text).map_err(|error| error.to_string())?;
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
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use miceq_dsp::Preset;

    fn temporary_store(name: &str) -> (PathBuf, Store) {
        let directory =
            std::env::temp_dir().join(format!("miceq-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let store = Store::new(directory.join("miceq").join("config.toml"));
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
            mic: MicConfig {
                device: Some("alsa_input.usb-Yeti".into()),
                preset: Some("broadcast".into()),
                noise_floor_dbfs: Some(-61.5),
                chain,
            },
        };
        store.save(&config).expect("save");
        let text = std::fs::read_to_string(store.path()).expect("written");
        assert!(
            text.starts_with("# micEQ") && text.contains("[mic.compressor]"),
            "{text}"
        );
        assert_eq!(store.load().expect("load").config, config);
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
