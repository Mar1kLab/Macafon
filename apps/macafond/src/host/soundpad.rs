//! Soundpad requests and housekeeping on the daemon control loop (PRD §7.6).

use super::DaemonHost;
use crate::soundpad::{Soundpad, library_error, open_folder};
use macafon_config::soundpad::clamp_volume;
use macafon_ipc::{
    ErrorBody, ErrorCode, Event, HotkeyAction, Request, SoundpadPrefs, SoundpadState,
};
use macafon_soundpad::library;
use std::time::Instant;

impl DaemonHost {
    /// Create the soundpad where the platform plays it: only in the running daemon, so tests
    /// never touch the user's folder.
    pub(super) fn start_soundpad(&mut self) {
        if !macafon_ipc::Platform::current().capabilities().soundpad {
            return;
        }
        let state_dir = match macafon_config::state_dir() {
            Ok(dir) => dir,
            Err(error) => {
                tracing::warn!(%error, "No state directory; the soundpad is off");
                return;
            }
        };
        let mut soundpad = Soundpad::new(&self.config.soundpad, &state_dir);
        soundpad.rescan();
        tracing::info!(folder = %soundpad.folder().display(), sounds = soundpad.entries().len(), "Soundpad ready");
        self.soundpad = Some(soundpad);
        self.preload_sounds();
    }

    pub(super) fn soundpad_state(&self) -> Option<SoundpadState> {
        let soundpad = self.soundpad.as_ref()?;
        let config = &self.config.soundpad;
        Some(SoundpadState {
            folder: soundpad.folder().display().to_string(),
            categories: soundpad.categories().to_vec(),
            sounds: soundpad.sounds(config),
            prefs: SoundpadPrefs {
                to_microphone: config.to_microphone,
                to_headphones: config.to_headphones,
                microphone_volume_pct: config.microphone_volume_pct,
                headphones_volume_pct: config.headphones_volume_pct,
                mute_voice_while_playing: config.mute_voice_while_playing,
                monitor_device: config.monitor_device.clone(),
                show_in_overlay: config.show_in_overlay,
            },
            playback: soundpad.playback(),
            microphone_ready: self.virtual_mic_ready,
        })
    }

    fn soundpad_value(&self) -> Result<serde_json::Value, ErrorBody> {
        serde_json::to_value(self.soundpad_state())
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }

    /// Sounds, categories or preferences changed: tell the clients, and register the hotkeys
    /// of the sounds that exist now.
    pub(super) fn soundpad_changed(&mut self) {
        if let Some(state) = self.soundpad_state() {
            self.hub.broadcast(&Event::SoundpadChanged(state));
        }
        let bound = self.bound_hotkeys.clone();
        self.rebind_hotkeys();
        if self.bound_hotkeys != bound {
            self.hotkeys_changed();
        }
        self.preload_sounds();
    }

    /// Sounds with a hotkey are decoded ahead, so a press plays at once.
    pub(super) fn preload_sounds(&mut self) {
        let Some(soundpad) = &mut self.soundpad else {
            return;
        };
        let ids: Vec<String> = self
            .config
            .hotkeys
            .keys()
            .filter_map(
                |binding| match HotkeyAction::parse(super::hotkeys::base_action(binding)) {
                    Some(HotkeyAction::SoundpadPlay(id)) => Some(id),
                    _ => None,
                },
            )
            .collect();
        soundpad.preload(ids.iter().map(String::as_str));
    }

    /// Every 100 ms: decodes, the end of a sound, the folder and progress events.
    pub(super) fn tick_soundpad(&mut self, now: Instant) {
        let Some(soundpad) = &mut self.soundpad else {
            return;
        };
        let outcome = soundpad.tick(&self.config.soundpad, now);
        let playback = soundpad.playback_changed();
        if let Some(id) = &outcome.played {
            self.count_play(id);
        }
        if outcome.library_changed || outcome.played.is_some() {
            self.soundpad_changed();
        }
        if let Some(playback) = playback {
            self.hub.broadcast(&Event::SoundpadPlayback(playback));
        }
    }

    fn count_play(&mut self, id: &str) {
        self.config.soundpad.edit_sound(id, |sound| {
            sound.play_count = sound.play_count.saturating_add(1)
        });
        self.changed_at = Some(Instant::now());
    }

    /// Settings and hotkeys of a renamed or moved sound follow it.
    fn sound_renamed(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        self.config.soundpad.rename_sound(from, to);
        for suffix in ["", "#secondary"] {
            let old = format!(
                "{}{suffix}",
                HotkeyAction::SoundpadPlay(from.to_owned()).id()
            );
            if let Some(shortcut) = self.config.hotkeys.remove(&old) {
                let new = format!("{}{suffix}", HotkeyAction::SoundpadPlay(to.to_owned()).id());
                self.config.hotkeys.insert(new, shortcut);
            }
        }
        if let Some(soundpad) = &mut self.soundpad {
            soundpad.renamed(from, to);
        }
        self.changed_at = Some(Instant::now());
    }

    /// A deleted sound takes its settings and hotkeys along.
    fn sound_removed(&mut self, id: &str) {
        self.config.soundpad.sounds.remove(id);
        for suffix in ["", "#secondary"] {
            let action = format!("{}{suffix}", HotkeyAction::SoundpadPlay(id.to_owned()).id());
            self.config.hotkeys.remove(&action);
        }
        if let Some(soundpad) = &mut self.soundpad {
            soundpad.removed(id);
        }
        self.changed_at = Some(Instant::now());
    }

    pub(super) fn respond_soundpad(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        let Some(soundpad) = &mut self.soundpad else {
            return Err(ErrorBody::new(
                ErrorCode::BadRequest,
                "the soundpad is not available on this platform",
            ));
        };
        let folder = soundpad.folder().to_owned();
        let null = Ok(serde_json::Value::Null);
        match request {
            Request::SoundpadGet => {}
            Request::SoundpadRescan => {
                if soundpad.rescan() {
                    self.soundpad_changed();
                }
            }
            Request::SoundpadOpenFolder => {
                open_folder(&folder)
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))?;
                return null;
            }
            Request::SoundpadPlay { id } => {
                if soundpad.play(&id, &self.config.soundpad)? {
                    self.count_play(&id);
                    self.soundpad_changed();
                }
                self.tick_playback();
                return null;
            }
            Request::SoundpadPlayRandom { category } => {
                let candidates: Vec<String> = soundpad
                    .sounds(&self.config.soundpad)
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
                return self.respond_soundpad(Request::SoundpadPlay { id });
            }
            Request::SoundpadStop => {
                soundpad.stop();
                self.tick_playback();
                return null;
            }
            Request::SoundpadSetPaused { paused } => {
                soundpad.set_paused(paused);
                self.tick_playback();
                return null;
            }
            Request::SoundpadSeek { position_ms } => {
                soundpad.seek(position_ms);
                self.tick_playback();
                return null;
            }
            Request::SoundpadSetLooping { looping } => {
                soundpad.set_looping(looping);
                self.config.soundpad.looping = looping;
                self.changed_at = Some(Instant::now());
                self.tick_playback();
                return null;
            }
            Request::SoundpadSetPrefs { patch } => {
                let config = &mut self.config.soundpad;
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
                soundpad.apply(config);
                self.changed_at = Some(Instant::now());
                self.soundpad_changed();
            }
            Request::SoundpadSetMonitorDevice { device } => {
                self.config.soundpad.monitor_device = device;
                self.changed_at = Some(Instant::now());
                self.soundpad_changed();
            }
            Request::SoundpadSetSound { id, patch } => {
                soundpad.entry(&id)?;
                if let Some(volume) = patch.volume_pct {
                    soundpad.sound_volume_changed(&id, clamp_volume(volume));
                }
                self.config.soundpad.edit_sound(&id, |sound| {
                    if let Some(volume) = patch.volume_pct {
                        sound.volume_pct = volume;
                    }
                    if let Some(favorite) = patch.favorite {
                        sound.favorite = favorite;
                    }
                });
                self.changed_at = Some(Instant::now());
                self.soundpad_changed();
            }
            Request::SoundpadRename { id, name } => {
                soundpad.entry(&id)?;
                let entry = library::rename(&folder, &id, &name).map_err(library_error)?;
                self.sound_renamed(&id, &entry.id);
                if let Some(soundpad) = &mut self.soundpad {
                    soundpad.rescan();
                }
                self.soundpad_changed();
            }
            Request::SoundpadMove { id, category } => {
                soundpad.entry(&id)?;
                let entry = library::move_to_category(&folder, &id, category.as_deref())
                    .map_err(library_error)?;
                self.sound_renamed(&id, &entry.id);
                if let Some(soundpad) = &mut self.soundpad {
                    soundpad.rescan();
                }
                self.soundpad_changed();
            }
            Request::SoundpadDelete { id } => {
                soundpad.entry(&id)?;
                library::delete(&folder, &id).map_err(library_error)?;
                soundpad.rescan();
                self.sound_removed(&id);
                self.soundpad_changed();
            }
            Request::SoundpadCreateCategory { name } => {
                library::create_category(&folder, &name).map_err(library_error)?;
                soundpad.rescan();
                self.soundpad_changed();
            }
            Request::SoundpadRenameCategory { from, to } => {
                let moved: Vec<String> = library::scan(&folder)
                    .map_err(library_error)?
                    .iter()
                    .filter(|entry| entry.category.as_deref() == Some(from.as_str()))
                    .map(|entry| entry.id.clone())
                    .collect();
                library::rename_category(&folder, &from, &to).map_err(library_error)?;
                for id in moved {
                    let file = id.rsplit('/').next().unwrap_or(&id);
                    self.sound_renamed(&id, &format!("{to}/{file}"));
                }
                if let Some(soundpad) = &mut self.soundpad {
                    soundpad.rescan();
                }
                self.soundpad_changed();
            }
            Request::SoundpadDeleteCategory { name } => {
                let moved = library::delete_category(&folder, &name).map_err(library_error)?;
                for (from, to) in moved {
                    self.sound_renamed(&from, &to);
                }
                if let Some(soundpad) = &mut self.soundpad {
                    soundpad.rescan();
                }
                self.soundpad_changed();
            }
            Request::SoundpadUpload {
                upload_id,
                file_name,
                category,
                offset,
                data,
                done,
            } => {
                let entry = soundpad.upload(
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
                self.soundpad_changed();
            }
            _ => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "not a soundpad request",
                ));
            }
        }
        self.soundpad_value()
    }

    /// Send a playback change at once instead of on the next tick.
    fn tick_playback(&mut self) {
        if let Some(playback) = self.soundpad.as_mut().and_then(Soundpad::playback_changed) {
            self.hub.broadcast(&Event::SoundpadPlayback(playback));
        }
    }
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
