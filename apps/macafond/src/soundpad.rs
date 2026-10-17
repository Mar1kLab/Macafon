//! Soundpad (PRD §7.6) on the daemon control loop: the library folder, background decoding,
//! uploads and the playback control whose taps the audio backend mixes into the virtual
//! microphone and the monitor output.

use macafon_config::SoundpadConfig;
use macafon_ipc::{ErrorBody, ErrorCode, SOUNDPAD_UPLOAD_CHUNK_BYTES, SoundInfo, SoundpadPlayback};
use macafon_soundpad::cache::SoundLoader;
use macafon_soundpad::library::{self, SoundEntry};
use macafon_soundpad::rt::{Output, SoundpadControl, SoundpadTap};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Decoded sounds kept in memory: about 20 minutes of 48 kHz stereo.
const CACHE_BYTES: usize = 512 * 1024 * 1024;
/// Files copied into the folder by hand show up within this time.
const RESCAN_INTERVAL: Duration = Duration::from_secs(3);
/// The monitor stream stays open this long after a sound, so the next one starts at once.
const MONITOR_LINGER: Duration = Duration::from_secs(15);
/// Durations probed per control-loop tick, so a large new library does not stall it.
const PROBES_PER_TICK: usize = 16;
const STALE_UPLOAD: Duration = Duration::from_secs(120);

struct Upload {
    path: PathBuf,
    received: u64,
    touched: Instant,
}

/// What the host learns from one control-loop tick.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickOutcome {
    /// The library changed: sounds, categories, durations or decode errors.
    pub library_changed: bool,
    /// A sound started; its play count went up.
    pub played: Option<String>,
}

pub struct Soundpad {
    control: SoundpadControl,
    loader: SoundLoader,
    folder: PathBuf,
    uploads_dir: PathBuf,
    entries: Vec<SoundEntry>,
    categories: Vec<String>,
    /// Probed duration by id, valid for the file's modification time.
    durations: HashMap<String, (u64, Option<u32>)>,
    errors: HashMap<String, String>,
    /// When each file came into the folder, by id.
    added: HashMap<String, u64>,
    /// The sound waiting for its decode to start playing.
    pending: Option<String>,
    /// The sound of the current play token.
    playing: Option<(u64, String)>,
    scanned_at: Option<Instant>,
    uploads: HashMap<String, Upload>,
    last_active: Option<Instant>,
    last_playback: Option<SoundpadPlayback>,
}

impl Soundpad {
    pub fn new(config: &SoundpadConfig, state_dir: &Path) -> Self {
        let mut soundpad = Self {
            control: SoundpadControl::new(),
            loader: SoundLoader::new(CACHE_BYTES),
            folder: config.folder(),
            uploads_dir: state_dir.join("soundpad-uploads"),
            entries: Vec::new(),
            categories: Vec::new(),
            durations: HashMap::new(),
            errors: HashMap::new(),
            added: HashMap::new(),
            pending: None,
            playing: None,
            scanned_at: None,
            uploads: HashMap::new(),
            last_active: None,
            last_playback: None,
        };
        soundpad.apply(config);
        soundpad
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Output volumes and switches into the taps.
    pub fn apply(&mut self, config: &SoundpadConfig) {
        let control = &self.control;
        control.set_output_enabled(Output::Microphone, config.to_microphone);
        control.set_output_enabled(Output::Monitor, config.to_headphones);
        control.set_output_gain(Output::Microphone, config.microphone_volume_pct / 100.0);
        control.set_output_gain(Output::Monitor, config.headphones_volume_pct / 100.0);
        control.set_duck_voice(config.mute_voice_while_playing);
    }

    pub fn tap(&mut self, output: Output) -> SoundpadTap {
        self.control.tap(output)
    }

    /// The monitor stream runs while a sound plays and a little after (§7.6).
    pub fn monitor_wanted(&self, config: &SoundpadConfig) -> Option<Option<String>> {
        let active = self.playing.is_some()
            || self.pending.is_some()
            || self
                .last_active
                .is_some_and(|at| at.elapsed() < MONITOR_LINGER);
        (config.to_headphones && active).then(|| config.monitor_device.clone())
    }

    // --- Library -----------------------------------------------------------------------------

    pub fn entries(&self) -> &[SoundEntry] {
        &self.entries
    }

    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    pub fn entry(&self, id: &str) -> Result<&SoundEntry, ErrorBody> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, format!("unknown sound {id:?}")))
    }

    /// Look at the folder now; returns whether the sounds or categories changed.
    pub fn rescan(&mut self) -> bool {
        self.scanned_at = Some(Instant::now());
        let (entries, categories) = match library::scan(&self.folder).and_then(|entries| {
            library::categories(&self.folder).map(|categories| (entries, categories))
        }) {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, folder = %self.folder.display(), "Cannot scan the soundpad folder");
                (Vec::new(), Vec::new())
            }
        };
        if entries == self.entries && categories == self.categories {
            return false;
        }
        for entry in &entries {
            let changed = self
                .entries
                .iter()
                .find(|old| old.id == entry.id)
                .is_some_and(|old| old.modified_unix_ms != entry.modified_unix_ms);
            if changed {
                self.loader.invalidate(&entry.id);
                self.errors.remove(&entry.id);
            }
        }
        self.durations
            .retain(|id, _| entries.iter().any(|entry| &entry.id == id));
        self.errors
            .retain(|id, _| entries.iter().any(|entry| &entry.id == id));
        self.added = entries
            .iter()
            .map(|entry| (entry.id.clone(), added_unix_ms(entry)))
            .collect();
        self.entries = entries;
        self.categories = categories;
        true
    }

    pub fn sounds(&self, config: &SoundpadConfig) -> Vec<SoundInfo> {
        self.entries
            .iter()
            .map(|entry| {
                let settings = config.sound(&entry.id);
                SoundInfo {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    category: entry.category.clone(),
                    duration_ms: self
                        .durations
                        .get(&entry.id)
                        .filter(|(modified, _)| *modified == entry.modified_unix_ms)
                        .and_then(|(_, duration)| *duration),
                    volume_pct: settings.volume_pct,
                    favorite: settings.favorite,
                    play_count: settings.play_count,
                    added_unix_ms: self
                        .added
                        .get(&entry.id)
                        .copied()
                        .unwrap_or(entry.modified_unix_ms),
                    error: self.errors.get(&entry.id).cloned(),
                }
            })
            .collect()
    }

    /// Decode ahead the sounds that hotkeys play, so they start at once.
    pub fn preload<'a>(&mut self, ids: impl Iterator<Item = &'a str>) {
        for id in ids {
            if let Some(entry) = self.entries.iter().find(|entry| entry.id == id)
                && !self.errors.contains_key(id)
                && self
                    .loader
                    .get(&entry.id, &entry.path, entry.modified_unix_ms)
                    .is_none()
            {
                self.loader
                    .request(&entry.id, &entry.path, entry.modified_unix_ms);
            }
        }
    }

    // --- Playback ----------------------------------------------------------------------------

    /// Start `id` from the beginning, now if decoded, otherwise once its decode finishes.
    /// Returns whether it started now.
    pub fn play(&mut self, id: &str, config: &SoundpadConfig) -> Result<bool, ErrorBody> {
        let entry = self.entry(id)?.clone();
        if let Some(error) = self.errors.get(id) {
            return Err(ErrorBody::new(
                ErrorCode::Internal,
                format!("{}: {error}", entry.name),
            ));
        }
        self.last_active = Some(Instant::now());
        match self
            .loader
            .get(&entry.id, &entry.path, entry.modified_unix_ms)
        {
            Some(sound) => {
                let gain = config.sound(id).volume_pct / 100.0;
                let token = self.control.play(sound, gain, 0, config.looping);
                self.playing = Some((token, entry.id));
                self.pending = None;
                Ok(true)
            }
            None => {
                self.control.stop();
                self.playing = None;
                self.loader
                    .request(&entry.id, &entry.path, entry.modified_unix_ms);
                self.pending = Some(entry.id);
                Ok(false)
            }
        }
    }

    pub fn stop(&mut self) {
        self.pending = None;
        self.playing = None;
        self.control.stop();
    }

    pub fn set_paused(&mut self, paused: Option<bool>) {
        let status = self.control.status();
        if status.token.is_some() {
            self.control.set_paused(paused.unwrap_or(!status.paused));
        }
    }

    pub fn seek(&mut self, position_ms: u32) {
        self.control.seek(position_ms);
    }

    pub fn set_looping(&mut self, looping: bool) {
        self.control.set_looping(looping);
    }

    /// The playing sound's own volume changed.
    pub fn sound_volume_changed(&mut self, id: &str, volume_pct: f32) {
        if self
            .playing
            .as_ref()
            .is_some_and(|(_, playing)| playing == id)
        {
            self.control.set_sound_gain(volume_pct / 100.0);
        }
    }

    /// A sound got another id (rename, move); playback and the decode follow.
    pub fn renamed(&mut self, from: &str, to: &str) {
        if let Some((_, id)) = &mut self.playing
            && id == from
        {
            *id = to.to_owned();
        }
        if self.pending.as_deref() == Some(from) {
            self.pending = Some(to.to_owned());
        }
        self.loader.invalidate(from);
        if let Some(duration) = self.durations.remove(from) {
            self.durations.insert(to.to_owned(), duration);
        }
    }

    /// A sound is gone: stop it if it plays.
    pub fn removed(&mut self, id: &str) {
        if self
            .playing
            .as_ref()
            .is_some_and(|(_, playing)| playing == id)
            || self.pending.as_deref() == Some(id)
        {
            self.stop();
        }
        self.loader.invalidate(id);
    }

    pub fn playback(&self) -> Option<SoundpadPlayback> {
        if let Some(id) = &self.pending {
            return Some(SoundpadPlayback {
                sound_id: id.clone(),
                position_ms: 0,
                duration_ms: self
                    .durations
                    .get(id)
                    .and_then(|(_, duration)| *duration)
                    .unwrap_or(0),
                paused: false,
                looping: false,
                loading: true,
            });
        }
        let status = self.control.status();
        let (token, id) = self.playing.as_ref()?;
        (status.token == Some(*token)).then(|| SoundpadPlayback {
            sound_id: id.clone(),
            position_ms: status.position_ms,
            duration_ms: status.duration_ms,
            paused: status.paused,
            looping: status.looping,
            loading: false,
        })
    }

    /// The playback to broadcast, if it changed since the last call.
    pub fn playback_changed(&mut self) -> Option<Option<SoundpadPlayback>> {
        let playback = self.playback();
        if playback == self.last_playback {
            return None;
        }
        self.last_playback.clone_from(&playback);
        Some(playback)
    }

    /// Control-loop housekeeping, every 100 ms.
    pub fn tick(&mut self, config: &SoundpadConfig, now: Instant) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        self.control.tick(now);
        if self.playing.is_some() || self.pending.is_some() {
            self.last_active = Some(now);
        }
        if self
            .playing
            .as_ref()
            .is_some_and(|(token, _)| self.control.status().token != Some(*token))
        {
            self.playing = None;
        }
        for loaded in self.loader.poll() {
            let entry = self.entries.iter().find(|entry| entry.id == loaded.id);
            match loaded.result {
                Ok(sound) => {
                    if let Some(entry) = entry {
                        let duration = Some(sound.duration_ms());
                        let known = self
                            .durations
                            .insert(entry.id.clone(), (entry.modified_unix_ms, duration));
                        outcome.library_changed |= known.and_then(|(_, d)| d) != duration;
                    }
                    if self.pending.as_deref() == Some(loaded.id.as_str()) {
                        let gain = config.sound(&loaded.id).volume_pct / 100.0;
                        let token = self.control.play(sound, gain, 0, config.looping);
                        self.pending = None;
                        self.playing = Some((token, loaded.id.clone()));
                        outcome.played = Some(loaded.id);
                    }
                }
                Err(error) => {
                    tracing::warn!(sound = %loaded.id, %error, "Cannot decode a sound");
                    if self.pending.as_deref() == Some(loaded.id.as_str()) {
                        self.pending = None;
                    }
                    self.errors.insert(loaded.id, error.to_string());
                    outcome.library_changed = true;
                }
            }
        }
        if self
            .scanned_at
            .is_none_or(|at| now.saturating_duration_since(at) >= RESCAN_INTERVAL)
        {
            outcome.library_changed |= self.rescan();
        }
        outcome.library_changed |= self.probe_durations();
        self.uploads
            .retain(|_, upload| now.saturating_duration_since(upload.touched) < STALE_UPLOAD);
        outcome
    }

    fn probe_durations(&mut self) -> bool {
        let missing: Vec<SoundEntry> = self
            .entries
            .iter()
            .filter(|entry| {
                self.durations
                    .get(&entry.id)
                    .is_none_or(|(modified, _)| *modified != entry.modified_unix_ms)
            })
            .take(PROBES_PER_TICK)
            .cloned()
            .collect();
        for entry in &missing {
            let duration = macafon_soundpad::decode::probe_duration_ms(&entry.path)
                .ok()
                .flatten();
            self.durations
                .insert(entry.id.clone(), (entry.modified_unix_ms, duration));
        }
        !missing.is_empty()
    }

    // --- Uploads -----------------------------------------------------------------------------

    /// Append one chunk; on the last one the file is added to the library. Returns the new
    /// entry once done.
    pub fn upload(
        &mut self,
        upload_id: &str,
        file_name: &str,
        category: Option<&str>,
        offset: u32,
        data: &str,
        done: bool,
    ) -> Result<Option<SoundEntry>, ErrorBody> {
        let invalid = |message: String| ErrorBody::new(ErrorCode::InvalidParams, message);
        if upload_id.is_empty()
            || upload_id.len() > 64
            || !upload_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(invalid("invalid upload id".into()));
        }
        // Only the name: never a path from the client.
        let name = Path::new(file_name)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid("invalid file name".into()))?;
        let extension = Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !library::SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
            return Err(invalid(format!("unsupported file type .{extension}")));
        }
        let bytes = decode_base64(data).ok_or_else(|| invalid("invalid base64 data".into()))?;
        if bytes.len() > SOUNDPAD_UPLOAD_CHUNK_BYTES {
            return Err(invalid("chunk too large".into()));
        }
        let io = |error: std::io::Error| ErrorBody::new(ErrorCode::Internal, error.to_string());
        if offset == 0 {
            if let Some(old) = self.uploads.remove(upload_id) {
                remove_upload(&old.path);
            }
            let directory = self.uploads_dir.join(upload_id);
            std::fs::create_dir_all(&directory).map_err(io)?;
            let path = directory.join(name);
            std::fs::write(&path, b"").map_err(io)?;
            self.uploads.insert(
                upload_id.to_owned(),
                Upload {
                    path,
                    received: 0,
                    touched: Instant::now(),
                },
            );
        }
        let upload = self
            .uploads
            .get_mut(upload_id)
            .ok_or_else(|| invalid("unknown upload; start at offset 0".into()))?;
        if u64::from(offset) != upload.received {
            return Err(invalid(format!(
                "expected offset {}, got {offset}",
                upload.received
            )));
        }
        if upload.received + bytes.len() as u64 > library::MAX_FILE_BYTES {
            let path = upload.path.clone();
            self.uploads.remove(upload_id);
            remove_upload(&path);
            return Err(invalid("the file is larger than 200 MB".into()));
        }
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&upload.path)
                .map_err(io)?;
            file.write_all(&bytes).map_err(io)?;
        }
        upload.received += bytes.len() as u64;
        upload.touched = Instant::now();
        if !done {
            return Ok(None);
        }
        let Some(upload) = self.uploads.remove(upload_id) else {
            return Ok(None);
        };
        let result = library::import(&self.folder, &upload.path, category);
        remove_upload(&upload.path);
        let entry = result.map_err(library_error)?;
        self.rescan();
        Ok(Some(entry))
    }
}

/// A copied file keeps its modification time, so the creation time tells when it was added;
/// renames and moves keep it.
fn added_unix_ms(entry: &SoundEntry) -> u64 {
    std::fs::metadata(&entry.path)
        .and_then(|metadata| metadata.created())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(entry.modified_unix_ms, |since| since.as_millis() as u64)
}

/// The upload's own directory goes with its file.
fn remove_upload(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(directory) = path.parent() {
        let _ = std::fs::remove_dir(directory);
    }
}

pub fn library_error(error: macafon_soundpad::Error) -> ErrorBody {
    use macafon_soundpad::Error;
    let code = match &error {
        Error::InvalidName(_)
        | Error::InvalidId(_)
        | Error::UnsupportedFormat
        | Error::TooLarge
        | Error::TooLong => ErrorCode::InvalidParams,
        Error::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
        Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        _ => ErrorCode::Internal,
    };
    ErrorBody::new(code, error.to_string())
}

/// Standard base64 with padding, as `btoa` writes it.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        Some(u32::from(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    }
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let padding = chunk.iter().rev().take_while(|&&byte| byte == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut word = 0;
        for &byte in &chunk[..4 - padding] {
            word = word << 6 | value(byte)?;
        }
        word <<= 6 * padding as u32;
        let decoded = [(word >> 16) as u8, (word >> 8) as u8, word as u8];
        out.extend_from_slice(&decoded[..3 - padding]);
    }
    Some(out)
}

/// Open the library folder in the file manager.
pub fn open_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    #[cfg(windows)]
    let program = "explorer.exe";
    #[cfg(not(windows))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(folder).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_btoa() {
        assert_eq!(decode_base64("").as_deref(), Some(&b""[..]));
        assert_eq!(decode_base64("TWFu").as_deref(), Some(&b"Man"[..]));
        assert_eq!(decode_base64("TWE=").as_deref(), Some(&b"Ma"[..]));
        assert_eq!(decode_base64("TQ==").as_deref(), Some(&b"M"[..]));
        assert_eq!(decode_base64("TQ=="), Some(b"M".to_vec()));
        assert_eq!(decode_base64("T==A"), None);
        assert_eq!(decode_base64("TWFu!A=="), None);
    }

    #[test]
    fn uploads_land_in_the_folder_in_order_only() {
        let root = std::env::temp_dir().join(format!("macafon-soundpad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let config = SoundpadConfig {
            folder: Some(root.join("sounds")),
            ..Default::default()
        };
        let mut soundpad = Soundpad::new(&config, &root.join("state"));
        // "RIFF" then the rest of a (not decodable) file; import only checks name and size.
        assert!(
            soundpad
                .upload("u1", "../Air horn.wav", Some("Memes"), 0, "UklGRg==", false)
                .expect("first chunk")
                .is_none()
        );
        let skipped = soundpad.upload("u1", "Air horn.wav", Some("Memes"), 2, "AAAA", true);
        assert_eq!(
            skipped.expect_err("offset gap").code,
            ErrorCode::InvalidParams
        );
        let entry = soundpad
            .upload("u1", "Air horn.wav", Some("Memes"), 4, "AAAA", true)
            .expect("last chunk")
            .expect("imported");
        assert_eq!(entry.id, "Memes/Air horn.wav");
        assert_eq!(std::fs::read(&entry.path).expect("file").len(), 7);
        assert!(soundpad.entries().iter().any(|e| e.id == entry.id));
        assert!(
            soundpad
                .upload("u2", "notes.txt", None, 0, "AAAA", true)
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
