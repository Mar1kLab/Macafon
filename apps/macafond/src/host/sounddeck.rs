//! SoundDeck requests and housekeeping on the daemon control loop (PRD §7.6).

use super::{DaemonHost, reply};
use crate::sounddeck::{SoundDeck, library_error, open_folder};
use macafon_config::sounddeck::clamp_volume;
use macafon_ipc::{
    ErrorBody, ErrorCode, Event, HotkeyAction, Request, SoundDeckPrefs, SoundDeckPrefsPatch,
    SoundDeckState,
};
use macafon_sounddeck::library;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

impl DaemonHost {
    /// Create the SoundDeck where the platform plays it: only in the running daemon, so tests
    /// never touch the user's folder.
    pub(super) fn start_sounddeck(&mut self) {
        if !macafon_ipc::Platform::current().capabilities().sounddeck {
            return;
        }
        let state_dir = match macafon_config::state_dir() {
            Ok(dir) => dir,
            Err(error) => {
                tracing::warn!(%error, "No state directory; the SoundDeck is off");
                return;
            }
        };
        let mut sounddeck = SoundDeck::new(&self.config.sounddeck, &state_dir);
        sounddeck.rescan();
        tracing::info!(folder = %sounddeck.folder().display(), sounds = sounddeck.entries().len(), "SoundDeck ready");
        self.sounddeck = Some(sounddeck);
        self.preload_sounds();
    }

    pub(super) fn sounddeck_state(&self) -> Option<SoundDeckState> {
        let sounddeck = self.sounddeck.as_ref()?;
        let config = &self.config.sounddeck;
        Some(SoundDeckState {
            folder: sounddeck.folder().display().to_string(),
            default_folder: config.folder.is_none(),
            categories: sounddeck.categories().to_vec(),
            sounds: sounddeck.sounds(config),
            problem_sounds: sounddeck.problem_sounds(),
            prefs: SoundDeckPrefs {
                to_microphone: config.to_microphone,
                to_headphones: config.to_headphones,
                microphone_volume_pct: config.microphone_volume_pct,
                headphones_volume_pct: config.headphones_volume_pct,
                mute_voice_while_playing: config.mute_voice_while_playing,
                monitor_device: config.monitor_device.clone(),
                show_in_overlay: config.show_in_overlay,
                overlay_controls: match config.overlay_controls {
                    macafon_config::sounddeck::SoundDeckOverlayControls::Top => {
                        macafon_ipc::SoundDeckOverlayControls::Top
                    }
                    macafon_config::sounddeck::SoundDeckOverlayControls::Bottom => {
                        macafon_ipc::SoundDeckOverlayControls::Bottom
                    }
                    macafon_config::sounddeck::SoundDeckOverlayControls::Hidden => {
                        macafon_ipc::SoundDeckOverlayControls::Hidden
                    }
                },
                overlay_view: match config.overlay_view {
                    macafon_config::sounddeck::SoundDeckOverlayView::Grid => {
                        macafon_ipc::SoundDeckOverlayView::Grid
                    }
                    macafon_config::sounddeck::SoundDeckOverlayView::List => {
                        macafon_ipc::SoundDeckOverlayView::List
                    }
                },
            },
            playback: sounddeck.playback(),
            microphone_ready: self.virtual_mic_ready,
        })
    }

    fn sounddeck_value(&self) -> Result<serde_json::Value, ErrorBody> {
        reply(self.sounddeck_state())
    }

    /// Sounds, categories or preferences changed: tell the clients, and register the hotkeys
    /// of the sounds that exist now.
    pub(super) fn sounddeck_changed(&mut self) {
        self.sounddeck_dirty = true;
        let bound = self.bound_hotkeys.clone();
        self.rebind_hotkeys();
        if self.bound_hotkeys != bound {
            self.hotkeys_changed();
        }
        self.preload_sounds();
    }

    /// Sounds with a hotkey are decoded ahead, so a press plays at once.
    pub(super) fn preload_sounds(&mut self) {
        let Some(sounddeck) = &mut self.sounddeck else {
            return;
        };
        let ids: std::collections::BTreeSet<String> = self
            .config
            .hotkeys
            .keys()
            .filter_map(
                |binding| match HotkeyAction::parse(super::hotkeys::base_action(binding)) {
                    Some(HotkeyAction::SoundDeckPlay(id)) => Some(id),
                    _ => None,
                },
            )
            .collect();
        sounddeck.preload(ids.iter().map(String::as_str));
    }

    /// Every 100 ms: decodes, the end of a sound, the folder and progress events.
    pub(super) fn tick_sounddeck(&mut self, now: Instant) {
        let Some(sounddeck) = &mut self.sounddeck else {
            return;
        };
        let outcome = sounddeck.tick(&self.config.sounddeck, now);
        let playback = sounddeck.playback_changed();
        if let Some(id) = &outcome.played {
            self.count_play(id);
        }
        for (from, to) in outcome.renamed {
            self.sound_renamed(&from, &to);
        }
        if outcome.library_changed {
            self.sounddeck_changed();
        }
        if self.sounddeck_dirty
            && self
                .sounddeck_broadcast_at
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
            && self.hub.has_clients()
            && let Some(state) = self.sounddeck_state()
            && self.hub.broadcast_sounddeck(state)
        {
            self.sounddeck_dirty = false;
            self.sounddeck_broadcast_at = Some(now);
        }
        if outcome.preload_ready {
            self.preload_sounds();
        }
        if let Some(playback) = playback {
            self.hub.broadcast(&Event::SoundDeckPlayback(playback));
        }
    }

    fn count_play(&mut self, id: &str) {
        self.sounddeck_dirty = true;
        self.config.sounddeck.edit_sound(id, |sound| {
            sound.play_count = sound.play_count.saturating_add(1)
        });
        self.changed_at = Some(Instant::now());
    }

    /// Settings and hotkeys of a renamed or moved sound follow it.
    fn sound_renamed(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        self.config.sounddeck.rename_sound(from, to);
        self.update_sound_hotkeys(from, Some(to));
        if let Some(sounddeck) = &mut self.sounddeck {
            sounddeck.renamed(from, to);
        }
        self.changed_at = Some(Instant::now());
    }

    /// A deleted sound takes its settings and hotkeys along.
    fn sound_removed(&mut self, id: &str) {
        self.config.sounddeck.sounds.remove(id);
        self.update_sound_hotkeys(id, None);
        if let Some(sounddeck) = &mut self.sounddeck {
            sounddeck.removed(id);
        }
        self.changed_at = Some(Instant::now());
    }

    fn update_sound_hotkeys(&mut self, from: &str, to: Option<&str>) {
        let from = HotkeyAction::SoundDeckPlay(from.to_owned()).id();
        let to = to.map(|id| HotkeyAction::SoundDeckPlay(id.to_owned()).id());
        for suffix in ["", "#secondary"] {
            if let Some(shortcut) = self.config.hotkeys.remove(&format!("{from}{suffix}"))
                && let Some(to) = &to
            {
                self.config
                    .hotkeys
                    .insert(format!("{to}{suffix}"), shortcut);
            }
        }
    }

    pub(super) fn respond_sounddeck(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        let sounddeck = self.sounddeck.as_mut().ok_or_else(sounddeck_unavailable)?;
        let folder = sounddeck.folder().to_owned();
        let null = Ok(serde_json::Value::Null);
        match request {
            Request::SoundDeckGet => {}
            Request::SoundDeckRescan => {
                sounddeck.rescan();
            }
            Request::SoundDeckOpenFolder => {
                open_folder(&folder).map_err(super::backend_error)?;
                return null;
            }
            Request::SoundDeckSetFolder { folder } => {
                let chosen = library_folder(folder.as_deref())?;
                let path = chosen
                    .clone()
                    .unwrap_or_else(macafon_config::sounddeck::default_folder);
                std::fs::create_dir_all(&path).map_err(|error| {
                    ErrorBody::new(
                        ErrorCode::InvalidParams,
                        format!("cannot use {}: {error}", path.display()),
                    )
                })?;
                if !path.is_dir() {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        format!("{} is not a folder", path.display()),
                    ));
                }
                if folder_path(&path) != folder_path(sounddeck.folder()) {
                    sounddeck.set_folder(path);
                }
                self.config.sounddeck.folder = chosen;
                self.changed_at = Some(Instant::now());
                self.sounddeck_changed();
            }
            Request::SoundDeckPlay { .. }
            | Request::SoundDeckPlayRandom { .. }
            | Request::SoundDeckStop
            | Request::SoundDeckSetPaused { .. }
            | Request::SoundDeckSeek { .. }
            | Request::SoundDeckSetLooping { .. } => {
                return self.respond_sounddeck_transport(request);
            }
            Request::SoundDeckSetPrefs { patch } => {
                apply_prefs(&mut self.config.sounddeck, patch);
                sounddeck.apply(&self.config.sounddeck);
                self.changed_at = Some(Instant::now());
                self.sounddeck_changed();
            }
            Request::SoundDeckSetMonitorDevice { device } => {
                self.config.sounddeck.monitor_device = device;
                self.changed_at = Some(Instant::now());
                self.sounddeck_changed();
            }
            Request::SoundDeckSetSound { id, patch } => {
                sounddeck.entry(&id)?;
                if let Some(volume) = patch.volume_pct {
                    sounddeck.sound_volume_changed(&id, clamp_volume(volume));
                }
                self.config.sounddeck.edit_sound(&id, |sound| {
                    if let Some(volume) = patch.volume_pct {
                        sound.volume_pct = volume;
                    }
                    if let Some(favorite) = patch.favorite {
                        sound.favorite = favorite;
                    }
                });
                self.changed_at = Some(Instant::now());
                self.sounddeck_changed();
            }
            Request::SoundDeckRename { id, name } => {
                sounddeck.entry(&id)?;
                let entry = library::rename(&folder, &id, &name).map_err(library_error)?;
                self.sound_renamed(&id, &entry.id);
                if let Some(sounddeck) = &mut self.sounddeck {
                    sounddeck.rescan();
                }
                self.sounddeck_changed();
            }
            Request::SoundDeckMove { id, category } => {
                sounddeck.entry(&id)?;
                let entry = library::move_to_category(&folder, &id, category.as_deref())
                    .map_err(library_error)?;
                self.sound_renamed(&id, &entry.id);
                if let Some(sounddeck) = &mut self.sounddeck {
                    sounddeck.rescan();
                }
                self.sounddeck_changed();
            }
            Request::SoundDeckDeleteProblemSounds => {
                let deleted = library::delete_problem_sounds(&folder).map_err(library_error)?;
                sounddeck.rescan();
                for id in deleted {
                    self.sound_removed(&id);
                }
                self.sounddeck_changed();
            }
            Request::SoundDeckDelete { id } => {
                if !id.starts_with("Unplayable/") {
                    sounddeck.entry(&id)?;
                }
                library::delete(&folder, &id).map_err(library_error)?;
                sounddeck.rescan();
                self.sound_removed(&id);
                self.sounddeck_changed();
            }
            Request::SoundDeckCreateCategory { name } => {
                library::create_category(&folder, &name).map_err(library_error)?;
                sounddeck.rescan();
                self.sounddeck_changed();
            }
            Request::SoundDeckRenameCategory { from, to } => {
                self.rename_sounddeck_category(&from, &to)?;
                self.sounddeck_changed();
            }
            Request::SoundDeckDeleteCategory { name } => {
                self.delete_sounddeck_category(&name)?;
                self.sounddeck_changed();
            }
            Request::SoundDeckUpload {
                upload_id,
                file_name,
                category,
                offset,
                data,
                done,
            } => {
                let entry = sounddeck.upload(
                    &upload_id,
                    &file_name,
                    category.as_deref(),
                    offset,
                    &data,
                    done,
                )?;
                if entry.is_none() {
                    return null;
                }
                self.sounddeck_dirty = true;
                return null;
            }
            _ => {
                return Err(not_sounddeck_request());
            }
        }
        self.sounddeck_value()
    }

    fn rename_sounddeck_category(&mut self, from: &str, to: &str) -> Result<(), ErrorBody> {
        let folder = self
            .sounddeck
            .as_ref()
            .ok_or_else(sounddeck_unavailable)?
            .folder()
            .to_owned();
        // Only known sounds can have settings; the folder rename itself also moves new files.
        let moved: Vec<String> = self
            .sounddeck
            .as_ref()
            .ok_or_else(sounddeck_unavailable)?
            .entries()
            .iter()
            .filter(|entry| entry.category.as_deref() == Some(from))
            .map(|entry| entry.id.clone())
            .collect();
        library::rename_category(&folder, from, to).map_err(library_error)?;
        for id in moved {
            let file = id.rsplit('/').next().unwrap_or(&id);
            self.sound_renamed(&id, &format!("{to}/{file}"));
        }
        if let Some(sounddeck) = &mut self.sounddeck {
            sounddeck.rescan();
        }
        Ok(())
    }

    /// Sounds move to the folder itself, under the names the library actually gave them.
    fn delete_sounddeck_category(&mut self, name: &str) -> Result<(), ErrorBody> {
        let folder = self
            .sounddeck
            .as_ref()
            .ok_or_else(sounddeck_unavailable)?
            .folder()
            .to_owned();
        let result = library::delete_category(&folder, name, |from, to| {
            self.sound_renamed(from, to);
        });
        if let Some(sounddeck) = &mut self.sounddeck {
            sounddeck.rescan();
        }
        self.sounddeck_changed();
        result.map_err(library_error)
    }

    fn respond_sounddeck_transport(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        let sounddeck = self.sounddeck.as_mut().ok_or_else(sounddeck_unavailable)?;
        match request {
            Request::SoundDeckPlay { id } => {
                if sounddeck.play(&id, &self.config.sounddeck)? {
                    self.count_play(&id);
                }
                self.tick_playback();
            }
            Request::SoundDeckPlayRandom { category } => {
                let candidates: Vec<String> = sounddeck
                    .sounds(&self.config.sounddeck)
                    .into_iter()
                    .filter(|sound| {
                        sound.error.is_none()
                            && category
                                .as_ref()
                                .is_none_or(|category| sound.category.as_ref() == Some(category))
                    })
                    .map(|sound| sound.id)
                    .collect();
                let id = random_pick(&candidates)
                    .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "no sounds to play"))?
                    .clone();
                return self.respond_sounddeck(Request::SoundDeckPlay { id });
            }
            Request::SoundDeckStop => {
                sounddeck.stop();
                self.tick_playback();
            }
            Request::SoundDeckSetPaused { paused } => {
                sounddeck.set_paused(paused);
                self.tick_playback();
            }
            Request::SoundDeckSeek { position_ms } => {
                sounddeck.seek(position_ms);
                self.tick_playback();
            }
            Request::SoundDeckSetLooping { looping } => {
                sounddeck.set_looping(looping);
                self.config.sounddeck.looping = looping;
                self.changed_at = Some(Instant::now());
                self.tick_playback();
            }
            _ => return Err(not_sounddeck_request()),
        }
        Ok(serde_json::Value::Null)
    }

    /// Send a playback change at once instead of on the next tick.
    fn tick_playback(&mut self) {
        if let Some(playback) = self
            .sounddeck
            .as_mut()
            .and_then(SoundDeck::playback_changed)
        {
            self.hub.broadcast(&Event::SoundDeckPlayback(playback));
        }
    }
}

fn apply_prefs(config: &mut macafon_config::SoundDeckConfig, patch: SoundDeckPrefsPatch) {
    if let Some(value) = patch.overlay_controls {
        config.overlay_controls = match value {
            macafon_ipc::SoundDeckOverlayControls::Top => {
                macafon_config::sounddeck::SoundDeckOverlayControls::Top
            }
            macafon_ipc::SoundDeckOverlayControls::Bottom => {
                macafon_config::sounddeck::SoundDeckOverlayControls::Bottom
            }
            macafon_ipc::SoundDeckOverlayControls::Hidden => {
                macafon_config::sounddeck::SoundDeckOverlayControls::Hidden
            }
        };
    }
    if let Some(value) = patch.overlay_view {
        config.overlay_view = match value {
            macafon_ipc::SoundDeckOverlayView::Grid => {
                macafon_config::sounddeck::SoundDeckOverlayView::Grid
            }
            macafon_ipc::SoundDeckOverlayView::List => {
                macafon_config::sounddeck::SoundDeckOverlayView::List
            }
        };
    }
    if let Some(value) = patch.to_microphone {
        config.to_microphone = value;
    }
    if let Some(value) = patch.to_headphones {
        config.to_headphones = value;
    }
    if let Some(value) = patch.microphone_volume_pct {
        config.microphone_volume_pct = clamp_volume(value);
    }
    if let Some(value) = patch.headphones_volume_pct {
        config.headphones_volume_pct = clamp_volume(value);
    }
    if let Some(value) = patch.mute_voice_while_playing {
        config.mute_voice_while_playing = value;
    }
    if let Some(value) = patch.show_in_overlay {
        config.show_in_overlay = value;
    }
}

/// A picked folder must be absolute; empty or the default one means the default.
fn library_folder(folder: Option<&str>) -> Result<Option<PathBuf>, ErrorBody> {
    let Some(folder) = folder.map(str::trim).filter(|folder| !folder.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(folder);
    if !path.is_absolute() {
        return Err(ErrorBody::new(
            ErrorCode::InvalidParams,
            format!("{folder} is not a full path"),
        ));
    }
    let default = macafon_config::sounddeck::default_folder();
    Ok((folder_path(&path) != folder_path(&default)).then_some(path))
}

/// The same folder written with another case or trailing separator compares equal.
fn folder_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

fn sounddeck_unavailable() -> ErrorBody {
    ErrorBody::new(
        ErrorCode::BadRequest,
        "the SoundDeck is not available on this platform",
    )
}

fn not_sounddeck_request() -> ErrorBody {
    ErrorBody::new(ErrorCode::BadRequest, "not a SoundDeck request")
}

/// Good enough for "play a random sound": the clock's nanoseconds, no extra dependency.
fn random_pick<T>(items: &[T]) -> Option<&T> {
    if items.is_empty() {
        return None;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| {
            time.subsec_nanos() as usize ^ time.as_secs() as usize
        });
    // A multiplicative hash spreads consecutive nanosecond readings.
    items.get(seed.wrapping_mul(0x9E37_79B9) % items.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_prefs_patch_preserves_absent_fields_and_round_trips() {
        let mut config = macafon_config::SoundDeckConfig::default();
        let patch = serde_json::from_value(serde_json::json!({
            "overlay_view": "list", "overlay_controls": "bottom"
        }))
        .expect("overlay patch");
        apply_prefs(&mut config, patch);
        apply_prefs(&mut config, SoundDeckPrefsPatch::default());
        assert_eq!(
            config.overlay_view,
            macafon_config::sounddeck::SoundDeckOverlayView::List
        );
        assert_eq!(
            config.overlay_controls,
            macafon_config::sounddeck::SoundDeckOverlayControls::Bottom
        );
        assert!(config.show_in_overlay);
        let saved = serde_json::to_string(&config).expect("save preferences");
        assert_eq!(
            serde_json::from_str::<macafon_config::SoundDeckConfig>(&saved)
                .expect("load preferences"),
            config
        );
    }
}
