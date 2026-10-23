//! Config loading, schema migration, recovery, and atomic file replacement.

use crate::{Config, DEFAULT_QUANTUM_FRAMES, QUANTUM_CHOICES, SCHEMA_VERSION};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg_attr(
        not(windows),
        error("cannot determine the config directory: neither XDG_CONFIG_HOME nor HOME is set")
    )]
    #[cfg_attr(
        windows,
        error("cannot determine the config directory: APPDATA is not set")
    )]
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
            Ok((config, old_version)) => {
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
#[cfg(not(windows))]
pub fn config_dir() -> Result<PathBuf, Error> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
        .ok_or(Error::NoConfigDir)?;
    Ok(base.join("macafon"))
}

/// `%APPDATA%\Macafon` (PRD §13).
#[cfg(windows)]
pub fn config_dir() -> Result<PathBuf, Error> {
    known_dir("APPDATA")
}

/// Machine-local state that must not roam: logs and restore journals,
/// `%LOCALAPPDATA%\Macafon` on Windows; the config directory elsewhere.
pub fn state_dir() -> Result<PathBuf, Error> {
    #[cfg(windows)]
    return known_dir("LOCALAPPDATA");
    #[cfg(not(windows))]
    config_dir()
}

#[cfg(windows)]
fn known_dir(variable: &str) -> Result<PathBuf, Error> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(|base| Path::new(&base).join("Macafon"))
        .ok_or(Error::NoConfigDir)
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

fn parse(text: &str) -> Result<(Config, i64), String> {
    let document: toml::Table = toml::from_str(text).map_err(|error| error.to_string())?;
    let old_version = document
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1);
    let mut config: Config = toml::Value::Table(document)
        .try_into()
        .map_err(|error: toml::de::Error| error.to_string())?;
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
    // Hotkey ids before 0.9.0 start with `soundpad_`.
    config.hotkeys = std::mem::take(&mut config.hotkeys)
        .into_iter()
        .map(|(id, shortcut)| match id.strip_prefix("soundpad_") {
            Some(rest) => (format!("sounddeck_{rest}"), shortcut),
            None => (id, shortcut),
        })
        .collect();
    config.sounddeck.normalize_levels();
    config.sounddeck.microphone_volume_pct =
        crate::sounddeck::clamp_volume(config.sounddeck.microphone_volume_pct);
    config.sounddeck.headphones_volume_pct =
        crate::sounddeck::clamp_volume(config.sounddeck.headphones_volume_pct);
    for sound in config.sounddeck.sounds.values_mut() {
        sound.volume_pct = crate::sounddeck::clamp_volume(sound.volume_pct);
    }
    config.ui.volume_step_pct = config.ui.volume_step_pct.clamp(1, 20);
    if !QUANTUM_CHOICES.contains(&config.audio.quantum_frames) {
        config.audio.quantum_frames = DEFAULT_QUANTUM_FRAMES;
    }
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
    Ok((config, old_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppsConfig, AudioConfig, MicConfig, MicProfile, MixerConfig, OsdConfig, OsdEdge, OsdStyle,
        UiConfig,
    };
    #[test]
    fn sounddeck_volumes_are_clamped_on_load() {
        let (config, _) = parse(
            r#"
[sounddeck]
normalized_levels = true
microphone_volume_pct = 900.0
headphones_volume_pct = -4.0
[sounddeck.sounds."bad.wav"]
volume_pct = nan
[sounddeck.sounds."loud.wav"]
volume_pct = 300.0
"#,
        )
        .expect("config");
        assert_eq!(config.sounddeck.microphone_volume_pct, 200.0);
        assert_eq!(config.sounddeck.headphones_volume_pct, 0.0);
        assert_eq!(config.sounddeck.sound("bad.wav").volume_pct, 100.0);
        assert_eq!(config.sounddeck.sound("loud.wav").volume_pct, 200.0);
    }

    #[test]
    fn soundpad_config_before_0_9_loads() {
        let (config, _) = parse(
            r#"
[hotkeys]
soundpad_stop = "Ctrl+Alt+S"
"soundpad_play.Memes/air horn.mp3" = "Ctrl+Alt+H"
mic_mute_toggle = "Ctrl+Alt+M"
[soundpad]
show_in_overlay = false
"#,
        )
        .expect("config");
        assert!(!config.sounddeck.show_in_overlay);
        let ids: Vec<&str> = config.hotkeys.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "mic_mute_toggle",
                "sounddeck_play.Memes/air horn.mp3",
                "sounddeck_stop"
            ]
        );
    }

    use macafon_dsp::{InputChannel, Preset};
    use std::collections::BTreeMap;

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
            dictation: crate::dictation::DictationConfig::default(),
            displays: crate::DisplaysConfig {
                profiles: vec![macafon_ipc::DisplayProfile {
                    id: "game".into(),
                    name: "Game".into(),
                    icon: "gamepad".into(),
                    monitors: vec![
                        macafon_ipc::ProfileMonitor {
                            identity: macafon_ipc::DisplayIdentity {
                                edid: "GSM5B7F".into(),
                                serial: Some("123".into()),
                                path: "path-1".into(),
                            },
                            name: "LG ULTRAGEAR".into(),
                            layout: Some(macafon_ipc::DisplayLayout {
                                x: 0,
                                y: 0,
                                mode: macafon_ipc::DisplayMode {
                                    width: 2560,
                                    height: 1440,
                                    refresh_mhz: 165_000,
                                },
                                rotation: macafon_ipc::DisplayRotation::None,
                                primary: true,
                                mirror_of: None,
                                hdr: true,
                            }),
                        },
                        macafon_ipc::ProfileMonitor {
                            identity: macafon_ipc::DisplayIdentity {
                                edid: "DEL4123".into(),
                                serial: None,
                                path: "path-2".into(),
                            },
                            name: "DELL P2419H".into(),
                            layout: None,
                        },
                    ],
                    output_device: Some("Speakers".into()),
                }],
                confirmed: BTreeMap::from([("game".into(), vec!["DEL4123,GSM5B7F:123".into()])]),
            },
            device_profiles: BTreeMap::new(),
            mixer: MixerConfig::default(),
            apps: AppsConfig::default(),
            ui: UiConfig {
                close_hint_shown: false,
                quit_on_close: true,
                osd: OsdConfig {
                    look: crate::OsdLook::Sketch,
                    primary_monitor: false,
                    style: OsdStyle::Horizontal,
                    edge: OsdEdge::Top,
                    offset_pct: 12.5,
                },
                overlay: Default::default(),
                hotkey_defaults: 1,
                volume_step_pct: 5,
                main_monitor: String::new(),
            },
            audio: AudioConfig {
                quantum_frames: 256,
            },
            hotkeys: BTreeMap::new(),
            sounddeck: {
                let mut sounddeck = crate::SoundDeckConfig {
                    monitor_device: Some("headphones".into()),
                    overlay_view: crate::sounddeck::SoundDeckOverlayView::List,
                    overlay_controls: crate::sounddeck::SoundDeckOverlayControls::Hidden,
                    ..Default::default()
                };
                sounddeck.edit_sound("Memes/air horn.mp3", |sound| sound.favorite = true);
                sounddeck
            },
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
