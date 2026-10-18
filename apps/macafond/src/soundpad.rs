//! Soundpad (PRD §7.6) on the daemon control loop: the library folder, background decoding,
//! uploads and the playback control whose taps the audio backend mixes into the virtual
//! microphone and the monitor output.

use macafon_config::SoundpadConfig;
use macafon_ipc::{ErrorBody, ErrorCode, SOUNDPAD_UPLOAD_CHUNK_BYTES, SoundInfo, SoundpadPlayback};
use macafon_soundpad::cache::SoundLoader;
use macafon_soundpad::library::{self, SoundEntry};
use macafon_soundpad::rt::{Output, SoundpadControl, SoundpadTap};
use std::collections::{HashMap, HashSet};
#[path = "soundpad_library.rs"]
mod library_worker;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Decoded sounds kept in memory: about 20 minutes of 48 kHz stereo.
const CACHE_BYTES: usize = 512 * 1024 * 1024;
/// Files copied into the folder by hand show up within this time.
const RESCAN_INTERVAL: Duration = Duration::from_secs(3);
/// The monitor stream stays open this long after a sound, so the next one starts at once.
const MONITOR_LINGER: Duration = Duration::from_secs(15);
/// A bounded working set, attempted once per file revision to avoid eviction churn.
const MAX_PRELOADS: usize = 4;
const PRELOAD_BYTES_PER_SOUND: u64 = 8 * 1024 * 1024;
const STALE_UPLOAD: Duration = Duration::from_secs(120);

struct Upload {
    path: PathBuf,
    received: u64,
    touched: Instant,
}

impl Drop for Upload {
    fn drop(&mut self) {
        remove_upload(&self.path);
    }
}

/// What the host learns from one control-loop tick.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickOutcome {
    /// The library changed: sounds, categories, durations or decode errors.
    pub library_changed: bool,
    /// A sound started; its play count went up.
    pub played: Option<String>,
    pub renamed: Vec<(String, String)>,
    pub preload_ready: bool,
}

pub struct Soundpad {
    control: SoundpadControl,
    loader: SoundLoader,
    folder: PathBuf,
    uploads_dir: PathBuf,
    entries: Vec<SoundEntry>,
    entry_index: HashMap<String, usize>,
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
    worker: library_worker::Worker,
    scan_revision: u64,
    scan_busy: bool,
    scan_requested: bool,
    renamed_files: Vec<(String, String)>,
    preloaded: HashSet<(String, u64)>,
    preload_pending: Option<String>,
    uploads: HashMap<String, Upload>,
    last_active: Option<Instant>,
    last_playback: Option<SoundpadPlayback>,
}

impl Soundpad {
    pub fn new(config: &SoundpadConfig, _state_dir: &Path) -> Self {
        let mut soundpad = Self {
            control: SoundpadControl::new(),
            loader: SoundLoader::new(CACHE_BYTES),
            folder: config.folder(),
            uploads_dir: config.folder().join(".macafon-uploads"),
            entries: Vec::new(),
            entry_index: HashMap::new(),
            categories: Vec::new(),
            durations: HashMap::new(),
            errors: HashMap::new(),
            added: HashMap::new(),
            pending: None,
            playing: None,
            scanned_at: None,
            worker: library_worker::Worker::new(),
            scan_revision: 0,
            scan_busy: false,
            scan_requested: false,
            renamed_files: Vec::new(),
            preloaded: HashSet::new(),
            preload_pending: None,
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

    /// Switch to another library folder: what plays stops, and everything known about the old
    /// folder (sounds, decodes, unfinished uploads) goes.
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.stop();
        self.loader.clear();
        self.uploads.clear();
        self.entries.clear();
        self.entry_index.clear();
        self.categories.clear();
        self.durations.clear();
        self.errors.clear();
        self.added.clear();
        self.renamed_files.clear();
        self.preloaded.clear();
        self.preload_pending = None;
        self.uploads_dir = folder.join(".macafon-uploads");
        self.folder = folder;
        self.rescan();
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
        self.entry_index
            .get(id)
            .and_then(|&index| self.entries.get(index))
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, format!("unknown sound {id:?}")))
    }

    /// Queue an inventory; filesystem I/O and duration probes run off the control loop.
    pub fn rescan(&mut self) {
        self.scan_revision = self.scan_revision.wrapping_add(1);
        self.worker.cancel_before(self.scan_revision);
        self.scan_requested = true;
        self.start_scan();
    }
    fn start_scan(&mut self) {
        if self.scan_requested && !self.scan_busy {
            self.scan_busy =
                self.worker
                    .request(self.scan_revision, self.folder.clone(), self.scan_requested);
            self.scan_requested = !self.scan_busy;
            self.scanned_at = Some(Instant::now());
        }
    }
    fn poll_library(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..3 {
            let Ok((revision, update)) = self.worker.results.try_recv() else {
                break;
            };
            if matches!(update, library_worker::Update::Complete) {
                self.scan_busy = false;
            }
            if revision != self.scan_revision {
                continue;
            }
            match update {
                library_worker::Update::Inventory(inventory) => {
                    changed |= self.apply_inventory(inventory);
                }
                library_worker::Update::Durations(mut durations) => {
                    for (id, (modified, duration)) in &mut durations {
                        if duration.is_none()
                            && let Some((known_modified, known)) = self.durations.get(id)
                            && known_modified == modified
                        {
                            *duration = *known;
                        }
                    }
                    if durations != self.durations {
                        self.durations = durations;
                        changed = true;
                    }
                }
                library_worker::Update::Complete => {}
            }
        }
        self.start_scan();
        changed
    }
    fn apply_inventory(&mut self, inventory: library_worker::Inventory) -> bool {
        let library_worker::Inventory {
            entries,
            categories,
        } = inventory;
        if entries == self.entries && categories == self.categories {
            return false;
        }
        let old: HashMap<_, _> = self.entries.iter().map(|e| (e.id.as_str(), e)).collect();
        let new: HashMap<_, _> = entries.iter().map(|e| (e.id.as_str(), e)).collect();
        // Infer only unambiguous disappeared/arrived pairs; never guess among duplicates.
        let signature = |e: &SoundEntry| {
            (
                e.path.file_name().map(|n| n.to_owned()),
                e.size_bytes,
                e.modified_unix_ms,
                e.added_unix_ms,
            )
        };
        let mut gone: HashMap<_, Vec<String>> = HashMap::new();
        let mut arrived: HashMap<_, Vec<String>> = HashMap::new();
        for entry in &self.entries {
            if !new.contains_key(entry.id.as_str()) {
                self.loader.invalidate(&entry.id);
                gone.entry(signature(entry))
                    .or_default()
                    .push(entry.id.clone());
            }
        }
        for entry in &entries {
            if let Some(previous) = old.get(entry.id.as_str()) {
                if previous.modified_unix_ms != entry.modified_unix_ms
                    || previous.size_bytes != entry.size_bytes
                {
                    self.loader.invalidate(&entry.id);
                    self.errors.remove(&entry.id);
                }
            } else {
                arrived
                    .entry(signature(entry))
                    .or_default()
                    .push(entry.id.clone());
            }
        }
        self.durations.retain(|id, _| new.contains_key(id.as_str()));
        self.errors.retain(|id, _| new.contains_key(id.as_str()));
        self.preloaded.retain(|(id, modified)| {
            new.get(id.as_str())
                .is_some_and(|e| e.modified_unix_ms == *modified)
        });
        self.added = entries
            .iter()
            .map(|e| (e.id.clone(), e.added_unix_ms))
            .collect();
        for (signature, from) in gone {
            if from.len() == 1
                && let Some(to) = arrived.get(&signature)
                && to.len() == 1
            {
                self.renamed_files.push((from[0].clone(), to[0].clone()));
            }
        }
        if self
            .preload_pending
            .as_ref()
            .is_some_and(|id| !new.contains_key(id.as_str()))
        {
            self.preload_pending = None;
        }
        self.entry_index = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id.clone(), index))
            .collect();
        self.entries = entries;
        self.categories = categories;
        for (from, to) in self.renamed_files.clone() {
            self.renamed(&from, &to);
        }
        if let Some(id) = self.pending.clone() {
            if let Some(entry) = self.entries.iter().find(|entry| entry.id == id) {
                self.loader
                    .request(&entry.id, &entry.path, entry.modified_unix_ms);
            } else {
                self.stop();
            }
        } else if self
            .playing
            .as_ref()
            .is_some_and(|(_, id)| !self.entries.iter().any(|e| &e.id == id))
        {
            self.stop();
        }
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
        if self.preload_pending.is_some() {
            return;
        }
        for id in ids.take(MAX_PRELOADS) {
            if let Some(entry) = self.entries.iter().find(|entry| entry.id == id)
                && !self.errors.contains_key(id)
                && self
                    .durations
                    .get(id)
                    .and_then(|(_, duration)| *duration)
                    .is_some_and(|duration_ms| {
                        u64::from(duration_ms) * 48 * 8 <= PRELOAD_BYTES_PER_SOUND
                    })
                && self
                    .preloaded
                    .insert((entry.id.clone(), entry.modified_unix_ms))
                && self
                    .loader
                    .get(&entry.id, &entry.path, entry.modified_unix_ms)
                    .is_none()
            {
                self.loader
                    .request(&entry.id, &entry.path, entry.modified_unix_ms);
                self.preload_pending = Some(entry.id.clone());
                break;
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
                let gain_lin = config.sound(id).volume_pct / 100.0;
                let token = self.control.play(sound, gain_lin, 0, config.looping);
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
        if let Some(index) = self.entry_index.remove(from)
            && let Some(entry) = self.entries.get_mut(index)
        {
            self.entry_index.insert(to.to_owned(), index);
            entry.id = to.to_owned();
            entry.path = self.folder.join(to);
            entry.category = to.split_once('/').map(|(category, _)| category.to_owned());
            entry.name = entry
                .path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_owned();
        }
        if self.preload_pending.as_deref() == Some(from) {
            self.preload_pending = None;
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
        // Rescan first: a file removed meanwhile must not accept its finished decode.
        let mut outcome = TickOutcome::default();
        if !self.scan_busy
            && self
                .scanned_at
                .is_none_or(|at| now.saturating_duration_since(at) >= RESCAN_INTERVAL)
        {
            self.scan_busy = self
                .worker
                .request(self.scan_revision, self.folder.clone(), false);
            self.scanned_at = Some(now);
        }
        outcome.library_changed |= self.poll_library();
        let loaded = self.poll_loaded_sounds(config);
        outcome.library_changed |= loaded.library_changed;
        outcome.played = loaded.played;
        outcome.preload_ready = loaded.preload_ready;
        outcome.renamed = std::mem::take(&mut self.renamed_files);
        self.uploads
            .retain(|_, upload| now.saturating_duration_since(upload.touched) < STALE_UPLOAD);
        outcome
    }

    fn poll_loaded_sounds(&mut self, config: &SoundpadConfig) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        for loaded in self.loader.poll() {
            if self.preload_pending.as_deref() == Some(&loaded.id) {
                self.preload_pending = None;
                outcome.preload_ready = true;
            }
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
                        let gain_lin = config.sound(&loaded.id).volume_pct / 100.0;
                        let token = self.control.play(sound, gain_lin, 0, config.looping);
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
        outcome
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
        library::validate_name(name).map_err(library_error)?;
        if let Some(category) = category {
            library::validate_name(category).map_err(library_error)?;
        }
        let extension = Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !library::SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
            return Err(invalid(format!("unsupported file type .{extension}")));
        }
        if data.len() > SOUNDPAD_UPLOAD_CHUNK_BYTES.div_ceil(3) * 4 {
            return Err(invalid("chunk too large".into()));
        }
        let bytes = decode_base64(data).ok_or_else(|| invalid("invalid base64 data".into()))?;
        if bytes.len() > SOUNDPAD_UPLOAD_CHUNK_BYTES {
            return Err(invalid("chunk too large".into()));
        }
        let io = |error: std::io::Error| ErrorBody::new(ErrorCode::Internal, error.to_string());
        if offset == 0 {
            self.uploads.remove(upload_id);
            let directory = library::upload_directory(&self.folder)
                .map_err(library_error)?
                .join(upload_id);
            std::fs::create_dir(&directory).map_err(io)?;
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
            self.uploads.remove(upload_id);
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
        let result = library::commit_upload(&self.folder, &upload.path, category);
        drop(upload);
        let entry = result.map_err(library_error)?;
        self.entry_index
            .insert(entry.id.clone(), self.entries.len());
        self.entries.push(entry.clone());
        if let Some(category) = &entry.category
            && !self.categories.contains(category)
        {
            self.categories.push(category.clone());
            self.categories
                .sort_by_cached_key(|name| name.to_lowercase());
        }
        self.added.insert(entry.id.clone(), entry.added_unix_ms);
        self.rescan();
        Ok(Some(entry))
    }
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
        Error::Io(io) => match io.kind() {
            std::io::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            _ => ErrorCode::Internal,
        },
        Error::Decode(_) | Error::WorkerUnavailable => ErrorCode::Internal,
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

/// Acknowledge dispatch immediately; neither shell activation nor folder creation blocks IPC.
pub fn open_folder(folder: &Path) -> std::io::Result<()> {
    let folder = folder.to_owned();
    std::thread::Builder::new()
        .name("macafon-open-folder".into())
        .spawn(move || {
            let start = Instant::now();
            if let Err(error) = open_folder_blocking(&folder) {
                tracing::warn!(%error, folder = %folder.display(), "Cannot open soundpad folder");
            }
            tracing::debug!(
                elapsed_ms = start.elapsed().as_millis(),
                "Soundpad shell activation finished"
            );
        })?;
    Ok(())
}

fn open_folder_blocking(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            Win32::{
                System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
                UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            },
            core::PCWSTR,
        };
        let folder: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: a dedicated shell thread owns the COM apartment and terminated path buffer.
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        // SAFETY: all optional pointers are null, and the path buffer outlives the call.
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR::null(),
                PCWSTR(folder.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if initialized {
            unsafe {
                // SAFETY: balances this thread's successful CoInitializeEx.
                CoUninitialize();
            }
        }
        if result.0 as isize <= 32 {
            return Err(std::io::Error::other(format!(
                "ShellExecuteW failed: {}",
                result.0 as isize
            )));
        }
    }
    #[cfg(not(windows))]
    std::process::Command::new("xdg-open").arg(folder).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_scan(soundpad: &mut Soundpad) -> bool {
        soundpad.rescan();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut changed = false;
        while soundpad.scan_busy || soundpad.scan_requested {
            changed |= soundpad.poll_library();
            assert!(Instant::now() < deadline, "scan timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        changed
    }

    fn test_soundpad(label: &str) -> (PathBuf, SoundpadConfig, Soundpad) {
        let root = std::env::temp_dir().join(format!("macafon-{label}-{}", std::process::id()));
        let config = SoundpadConfig {
            folder: Some(root.join("sounds")),
            ..Default::default()
        };
        let mut soundpad = Soundpad::new(&config, &root.join("state"));
        wait_scan(&mut soundpad);
        (root, config, soundpad)
    }

    fn write_wav(path: &Path) {
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend(996_u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(48_000_u32.to_le_bytes());
        bytes.extend(96_000_u32.to_le_bytes());
        bytes.extend(2_u16.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(960_u32.to_le_bytes());
        bytes.resize(1004, 0);
        std::fs::write(path, bytes).expect("write wav");
    }

    #[test]
    fn pending_decode_follows_rename() {
        let (root, config, mut soundpad) = test_soundpad("pending-rename");
        write_wav(&soundpad.folder.join("old.wav"));
        wait_scan(&mut soundpad);
        assert!(!soundpad.play("old.wav", &config).expect("play"));
        let renamed = library::rename(&soundpad.folder, "old.wav", "new").expect("rename");
        soundpad.renamed("old.wav", &renamed.id);
        wait_scan(&mut soundpad);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(id) = soundpad.tick(&config, Instant::now()).played {
                assert_eq!(id, "new.wav");
                break;
            }
            assert!(Instant::now() < deadline, "renamed sound stayed loading");
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn another_folder_replaces_the_library_and_stops_playback() {
        let (root, config, mut soundpad) = test_soundpad("set-folder");
        write_wav(&soundpad.folder.join("old.wav"));
        wait_scan(&mut soundpad);
        soundpad.play("old.wav", &config).expect("play");
        let other = root.join("D drive").join("Sounds");
        std::fs::create_dir_all(other.join("Memes")).expect("other folder");
        write_wav(&other.join("Memes").join("new.wav"));
        soundpad.set_folder(other.clone());
        wait_scan(&mut soundpad);
        assert_eq!(soundpad.folder(), other);
        assert_eq!(soundpad.playback(), None);
        let ids: Vec<&str> = soundpad.entries().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["Memes/new.wav"]);
        assert_eq!(soundpad.categories(), ["Memes"]);
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn failed_rescan_preserves_the_known_library() {
        let (root, _, mut soundpad) = test_soundpad("failed-rescan");
        write_wav(&soundpad.folder.join("known.wav"));
        wait_scan(&mut soundpad);
        std::fs::rename(&soundpad.folder, root.join("backup")).expect("move folder");
        std::fs::write(&soundpad.folder, b"temporarily unavailable").expect("block scan");
        assert!(!wait_scan(&mut soundpad));
        assert_eq!(soundpad.entries()[0].id, "known.wav");
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rescan_cancels_deleted_pending_decode() {
        let (root, config, mut soundpad) = test_soundpad("pending-delete");
        write_wav(&soundpad.folder.join("old.wav"));
        wait_scan(&mut soundpad);
        soundpad.play("old.wav", &config).expect("play");
        std::fs::remove_file(soundpad.folder.join("old.wav")).expect("delete");
        wait_scan(&mut soundpad);
        assert!(soundpad.playback().is_none());
        assert!(soundpad.tick(&config, Instant::now()).played.is_none());
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn uploads_expire_and_reject_invalid_names_before_writing() {
        let (root, config, mut soundpad) = test_soundpad("upload-expiry");
        for name in ["NUL.wav", "audio:stream.wav", "trailing .wav:"] {
            assert!(
                soundpad
                    .upload("bad", name, None, 0, "AAAA", false)
                    .is_err()
            );
        }
        assert!(
            soundpad
                .upload(
                    "bad",
                    "large.wav",
                    None,
                    0,
                    &"A".repeat(SOUNDPAD_UPLOAD_CHUNK_BYTES.div_ceil(3) * 4 + 4),
                    false
                )
                .is_err()
        );
        assert!(!soundpad.uploads_dir.join("bad").exists());
        soundpad
            .upload("expired", "tone.wav", None, 0, "AAAA", false)
            .expect("chunk");
        let path = soundpad.uploads["expired"].path.clone();
        assert!(path.exists());
        soundpad.tick(&config, Instant::now() + STALE_UPLOAD);
        assert!(!path.exists());
        assert!(!path.parent().expect("directory").exists());
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

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
    fn hotkey_preloading_is_bounded_and_does_not_restart_evicted_work() {
        let (root, _, mut soundpad) = test_soundpad("bounded-preload");
        let ids: Vec<String> = (0..8).map(|i| format!("tone{i}.wav")).collect();
        for id in &ids {
            write_wav(&soundpad.folder.join(id));
        }
        wait_scan(&mut soundpad);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            soundpad.preload(ids.iter().map(String::as_str));
            soundpad.poll_loaded_sounds(&SoundpadConfig::default());
            if soundpad.preloaded.len() == MAX_PRELOADS && soundpad.preload_pending.is_none() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        soundpad.loader.clear();
        soundpad.preload(ids.iter().map(String::as_str));
        assert!(
            soundpad.preload_pending.is_none(),
            "eviction must not cause decode churn"
        );
        let long = &ids[4];
        soundpad.durations.insert(long.clone(), (0, Some(600_000)));
        soundpad.preload(std::iter::once(long.as_str()));
        assert!(
            soundpad.preload_pending.is_none(),
            "long sounds decode only on demand"
        );
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opens Explorer for manual latency measurement"]
    fn open_folder_shell_measurement() {
        let root = std::env::temp_dir().join("macafon-shell-measurement");
        std::fs::create_dir_all(&root).expect("folder");
        let start = Instant::now();
        let mut explorer = std::process::Command::new("explorer.exe")
            .arg(&root)
            .spawn()
            .expect("old launch");
        eprintln!("old Explorer process spawn call: {:?}", start.elapsed());
        let start = Instant::now();
        open_folder(&root).expect("dispatch shell worker");
        eprintln!(
            "new open-folder IPC handler dispatch: {:?}",
            start.elapsed()
        );
        let _ = explorer.wait();
        // Shell activation is measured separately; it does not measure visible-window latency.
        let start = Instant::now();
        open_folder_blocking(&root).expect("shell activation");
        eprintln!(
            "ShellExecuteW including folder preparation + COM: {:?}",
            start.elapsed()
        );
        std::fs::remove_dir(root).expect("cleanup");
    }

    #[test]
    #[ignore = "generated 5000-file performance measurement"]
    fn large_library_measurement() {
        let (root, config, mut soundpad) = test_soundpad("large-library");
        for category in 0..50 {
            let dir = soundpad.folder.join(format!("Category {category}"));
            std::fs::create_dir_all(&dir).expect("category");
            for sound in 0..100 {
                write_wav(&dir.join(format!("Sound {sound}.wav")));
            }
        }
        let start = Instant::now();
        assert_eq!(library::scan(&soundpad.folder).expect("scan").len(), 5000);
        eprintln!("5000 files scan: {:?}", start.elapsed());
        let start = Instant::now();
        soundpad.rescan();
        eprintln!("rescan dispatch: {:?}", start.elapsed());
        let mut max_tick = Duration::ZERO;
        while soundpad.scan_busy || soundpad.scan_requested {
            let tick = Instant::now();
            soundpad.tick(&config, Instant::now());
            max_tick = max_tick.max(tick.elapsed());
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(soundpad.entries.len(), 5000);
        eprintln!(
            "background inventory + probes + application: {:?}; maximum applying tick: {:?}",
            start.elapsed(),
            max_tick
        );
        let start = Instant::now();
        for _ in 0..100 {
            soundpad.tick(&config, Instant::now());
        }
        eprintln!("100 idle ticks: {:?}", start.elapsed());
        let start = Instant::now();
        soundpad.scan_busy =
            soundpad
                .worker
                .request(soundpad.scan_revision, soundpad.folder.clone(), false);
        while soundpad.scan_busy {
            soundpad.poll_library();
            std::thread::yield_now();
        }
        eprintln!(
            "unchanged notification check + delivery: {:?}",
            start.elapsed()
        );
        for attempt in 0..3 {
            let start = Instant::now();
            soundpad.scan_busy =
                soundpad
                    .worker
                    .request(soundpad.scan_revision, soundpad.folder.clone(), false);
            while soundpad.scan_busy {
                soundpad.poll_library();
                std::thread::yield_now();
            }
            eprintln!("steady notification check {attempt}: {:?}", start.elapsed());
        }
        let start = Instant::now();
        let bytes = serde_json::to_vec(&soundpad.sounds(&config)).expect("serialize sounds");
        eprintln!(
            "5000-sound payload: {} bytes, construction + serialization: {:?}",
            bytes.len(),
            start.elapsed()
        );
        drop(soundpad);
        std::fs::remove_dir_all(root).expect("cleanup");
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
        wait_scan(&mut soundpad);
        assert!(soundpad.entries().iter().any(|e| e.id == entry.id));
        assert!(
            soundpad
                .upload("u2", "notes.txt", None, 0, "AAAA", true)
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
