//! Channels as volume groups (PRD §12.2.2) on the daemon's control loop: apps from the
//! [`SessionWatcher`] become the host's playback streams, the host's faders become session
//! volumes, and the output priority becomes the system default render device.

use crate::devices::{DataFlow, Endpoint};
use crate::journal::{self, SystemDefaults};
use crate::mixer::{App, MixSettings, MixerEvent, SYSTEM_SOUNDS, SessionWatcher};
use crate::output::{OutputChange, OutputPolicy};
use macafon_backend::{AppMix, MixerHost, PlaybackStream};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often sessions and the system volume are read while no window shows mixer levels;
/// session events still refresh the list at once (NFR-R7).
const UNWATCHED_POLL: Duration = Duration::from_secs(1);

pub(crate) struct VolumeGroups {
    watcher: Option<SessionWatcher>,
    master: Option<(String, f32, bool)>,
    /// Stable stream ids per executable for this run.
    ids: HashMap<String, u32>,
    streams: Vec<(String, PlaybackStream)>,
    /// What was last written per executable; external changes update it too.
    applied: HashMap<String, MixSettings>,
    /// The new-app faders last given to the watcher.
    new_app: Option<MixSettings>,
    /// App mutes set outside Macafon (WIN-CH-003); Macafon has no app mute of its own.
    muted: HashMap<String, bool>,
    enabled: bool,
    refresh: bool,
    /// When sessions and the system volume were last read.
    polled_at: Option<Instant>,
    output: OutputPolicy,
    output_journal: PathBuf,
    /// No output chosen: the system default is the user's (OUT-001 "Follow system default").
    following_system: bool,
}

impl VolumeGroups {
    /// Start watching sessions; this also finishes a restore left by a crash (WIN-CH-008).
    pub fn start(state_dir: &Path) -> Self {
        let output_journal = journal::journal_path(state_dir, DataFlow::Render);
        if let Err(error) = journal::restore(&mut SystemDefaults, &output_journal) {
            tracing::warn!(%error, "Cannot restore the previous default output yet");
        }
        let watcher = SessionWatcher::start(state_dir)
            .inspect_err(|error| tracing::warn!(%error, "App volumes unavailable"))
            .ok();
        Self {
            watcher,
            master: None,
            ids: HashMap::new(),
            streams: Vec::new(),
            applied: HashMap::new(),
            new_app: None,
            muted: HashMap::new(),
            enabled: false,
            refresh: true,
            polled_at: None,
            output: OutputPolicy::default(),
            output_journal,
            following_system: false,
        }
    }

    pub fn tick<H: MixerHost>(&mut self, host: &mut H) {
        // Out of `self` while it is used, so the rest of the state stays mutable.
        if let Some(watcher) = self.watcher.take() {
            self.tick_with(&watcher, host);
            self.watcher = Some(watcher);
        }
    }

    fn tick_with<H: MixerHost>(&mut self, watcher: &SessionWatcher, host: &mut H) {
        while let Some(event) = watcher.try_event() {
            match event {
                MixerEvent::AppsChanged => self.refresh = true,
                MixerEvent::ExternalVolume {
                    exe,
                    app_pct,
                    muted,
                } => {
                    self.muted.insert(exe.clone(), muted);
                    if let Some(applied) = self.applied.get_mut(&exe) {
                        applied.app_pct = app_pct;
                        applied.app_muted = muted;
                    }
                    if let Some((_, stream)) = self.streams.iter().find(|(name, _)| *name == exe) {
                        host.app_volume_changed(stream, app_pct);
                    }
                }
                MixerEvent::NewAppDefaulted { exe } => {
                    // Written by the watcher: the loop below corrects it to the app's real
                    // channel, or restores it if Macafon does not manage the app.
                    if let Some(settings) = self.new_app {
                        self.applied.entry(exe).or_insert(settings);
                    }
                }
                MixerEvent::Error(message) => tracing::warn!(%message, "App volumes"),
                MixerEvent::SetFailed { exe, message } => {
                    tracing::warn!(%message, app = %exe, "Cannot set app volume");
                    // Sent again on the next tick.
                    self.applied.remove(&exe);
                }
                MixerEvent::RecoveryPending { sessions } => {
                    tracing::info!(sessions, "App volumes to restore when their apps return");
                }
            }
        }
        // Live levels need fresh peaks every tick; otherwise reading every session over COM
        // ten times a second would be work nobody sees.
        let poll = host.mixer_watched()
            || self.refresh
            || self
                .polled_at
                .is_none_or(|at| at.elapsed() >= UNWATCHED_POLL);
        if poll {
            self.polled_at = Some(Instant::now());
            // Read before applying: startup and an endpoint change adopt Windows, never a saved fader.
            if let Ok(mut observed) = watcher.master(None) {
                if let Some(previous) = &self.master {
                    let desired = host.master_volume();
                    if previous.0 == observed.0
                        && ((desired.0 - previous.1).abs() > 0.001 || desired.1 != previous.2)
                    {
                        match watcher.master(Some(desired)) {
                            Ok(actual) => observed = actual,
                            Err(error) => tracing::warn!(%error, "Cannot set system volume"),
                        }
                    }
                }
                if self.master.as_ref() != Some(&observed)
                    || host.master_volume() != (observed.1, observed.2)
                {
                    host.master_volume_changed(observed.1, observed.2);
                }
                self.master = Some(observed);
            }
        }
        let enabled = host.mixer_enabled();
        if self.enabled && !enabled {
            self.release(Some(watcher));
        }
        self.enabled = enabled;
        let new_app = enabled
            .then(|| host.new_app_mix())
            .flatten()
            .map(|mix| settings(mix, false));
        if new_app != self.new_app {
            match watcher.set_new_app(new_app) {
                Ok(()) => self.new_app = new_app,
                Err(error) => tracing::warn!(%error, "Cannot set new app volume"),
            }
        }
        // The apps carry fresh peaks for the meters.
        if poll {
            match watcher.apps() {
                Ok(apps) => self.apps_changed(host, apps),
                Err(error) => tracing::debug!(%error, "Cannot list app sessions"),
            }
        }
        if enabled {
            self.apply_apps(watcher, host);
        }
    }

    /// The faders just changed (a held volume key): write them now, not on the next tick.
    pub fn apply_mix<H: MixerHost>(&mut self, host: &mut H) {
        let Some(watcher) = self.watcher.take() else {
            return;
        };
        if let Some(previous) = &self.master {
            let desired = host.master_volume();
            if (desired.0 - previous.1).abs() > 0.001 || desired.1 != previous.2 {
                match watcher.master(Some(desired)) {
                    Ok(actual) => self.master = Some(actual),
                    Err(error) => tracing::warn!(%error, "Cannot set system volume"),
                }
            }
        }
        if self.enabled && host.mixer_enabled() {
            self.apply_apps(&watcher, host);
        }
        self.watcher = Some(watcher);
    }

    fn apply_apps<H: MixerHost>(&mut self, watcher: &SessionWatcher, host: &H) {
        for (exe, stream) in &self.streams {
            let Some(mix) = host.app_mix(stream) else {
                if self.applied.contains_key(exe) {
                    match watcher.restore_app(exe) {
                        Ok(()) => {
                            self.applied.remove(exe);
                            self.muted.remove(exe);
                        }
                        Err(error) => {
                            tracing::warn!(%error, app = %exe, "Cannot restore app volume")
                        }
                    }
                }
                continue;
            };
            let settings = settings(mix, self.muted.get(exe).copied().unwrap_or(false));
            if self.applied.get(exe) == Some(&settings) {
                continue;
            }
            match watcher.set_app(exe, settings) {
                Ok(()) => {
                    self.applied.insert(exe.clone(), settings);
                }
                Err(error) => tracing::warn!(%error, app = %exe, "Cannot set app volume"),
            }
        }
    }

    fn apps_changed<H: MixerHost>(&mut self, host: &mut H, apps: Vec<App>) {
        let mut peaks = Vec::with_capacity(apps.len());
        let mut streams = Vec::with_capacity(apps.len());
        for app in apps {
            let next = self.ids.len() as u32 + 1;
            let id = *self.ids.entry(app.exe.clone()).or_insert(next);
            peaks.push((id, app.peak));
            let stream = stream(id, &app);
            // An app already muted in Windows before Macafon wrote anything keeps that mute;
            // a mute that Macafon's own channel or Master explains is not the app's.
            if app.muted
                && !self.applied.contains_key(&app.exe)
                && !self.muted.contains_key(&app.exe)
                && !host
                    .app_mix(&stream)
                    .is_some_and(|mix| mix.channel_muted || mix.master_muted)
            {
                self.muted.insert(app.exe.clone(), true);
            }
            streams.push((app.exe.clone(), stream));
        }
        if self.refresh || streams != self.streams {
            self.refresh = false;
            self.streams = streams;
            let list: Vec<PlaybackStream> = self.streams.iter().map(|(_, s)| s.clone()).collect();
            host.streams_changed(&list);
        }
        host.stream_peaks(&peaks);
    }

    /// WIN-CH-007: while channels are on, the chosen output becomes the system default.
    pub fn scan_outputs<H: MixerHost>(&mut self, host: &H, endpoints: &[Endpoint]) {
        if !self.enabled {
            return;
        }
        let priority = host.output_priority();
        if priority.is_empty() {
            // Switching to Follow system default gives back the default Macafon set.
            if !self.following_system {
                match self
                    .output
                    .follow_system_default(&mut SystemDefaults, &self.output_journal)
                {
                    Ok(()) => self.following_system = true,
                    Err(error) => tracing::warn!(%error, "Cannot restore the default output"),
                }
            }
            return;
        }
        self.following_system = false;
        match self.output.scan(
            &priority,
            endpoints,
            &mut SystemDefaults,
            &self.output_journal,
        ) {
            Ok(OutputChange::Applied(id)) => tracing::info!(output = %id, "Default output set"),
            Ok(OutputChange::ManualChange(_)) => {
                tracing::info!("Default output changed in Windows; leaving it")
            }
            Ok(OutputChange::Unchanged) => {}
            Err(error) => tracing::warn!(%error, "Cannot set the default output"),
        }
    }

    /// Mixer off: app volumes and the default output go back (WIN-CH-008, OUT-005).
    fn release(&mut self, watcher: Option<&SessionWatcher>) {
        if let Some(watcher) = watcher {
            if let Err(error) = watcher.restore() {
                tracing::warn!(%error, "Cannot restore app volumes; journal retained");
            }
        }
        self.applied.clear();
        // The watcher's restore dropped the new-app faders too.
        self.new_app = None;
        self.following_system = false;
        if let Err(error) = self
            .output
            .follow_system_default(&mut SystemDefaults, &self.output_journal)
        {
            tracing::warn!(%error, "Cannot restore the default output; journal retained");
        }
    }

    /// Quit: give everything back while the process is still allowed to run.
    pub fn stop(&mut self) {
        let watcher = self.watcher.take();
        self.release(watcher.as_ref());
    }
}

fn settings(mix: AppMix, app_muted: bool) -> MixSettings {
    MixSettings {
        channel_pct: mix.channel_pct,
        app_pct: mix.app_pct,
        master_pct: 100.0,
        channel_muted: mix.channel_muted,
        app_muted,
        master_muted: false,
    }
}

/// The host sees an app group as one stream, keyed by its executable (APP-002).
fn stream(id: u32, app: &App) -> PlaybackStream {
    let system = app.exe == SYSTEM_SOUNDS;
    let stem = app.exe.strip_suffix(".exe").unwrap_or(&app.exe);
    let named = !system && !app.exe.starts_with('<');
    PlaybackStream {
        id,
        serial: id.to_string(),
        node_name: app.exe.clone(),
        binary: named.then(|| stem.to_owned()),
        app_name: Some(if app.exe == crate::mixer::SOUNDDECK {
            "SoundDeck".to_owned()
        } else if named {
            stem.to_owned()
        } else {
            app.exe.trim_matches(['<', '>']).to_owned()
        }),
        // macafon-apps maps event streams to the System sounds pseudo-app (APP-011).
        icon_name: app.executable.clone(),
        media_role: system.then(|| "event".to_owned()),
        running: app.active,
        movable: true,
        ..PlaybackStream::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sounddeck_has_a_separate_movable_app_identity() {
        let sounddeck = stream(
            7,
            &App {
                exe: crate::mixer::SOUNDDECK.into(),
                executable: Some("C:\\Macafon\\macafond.exe".into()),
                active: true,
                session_count: 1,
                app_pct: 100.0,
                muted: false,
                peak: 0.1,
            },
        );
        assert_eq!(sounddeck.app_name.as_deref(), Some("SoundDeck"));
        assert!(sounddeck.binary.is_none());
        assert_eq!(sounddeck.node_name, crate::mixer::SOUNDDECK);
        assert!(sounddeck.movable && sounddeck.running);
        assert!(sounddeck.media_role.is_none());
    }
    #[test]
    fn an_app_muted_in_windows_stays_muted_unless_its_channel_explains_it() {
        struct Host {
            channel_muted: bool,
        }
        impl MixerHost for Host {
            fn app_mix(&self, _stream: &PlaybackStream) -> Option<AppMix> {
                Some(AppMix {
                    channel_pct: 100.0,
                    app_pct: 100.0,
                    master_pct: 100.0,
                    channel_muted: self.channel_muted,
                    master_muted: false,
                })
            }
        }
        let app = |exe: &str| App {
            exe: exe.into(),
            executable: None,
            active: true,
            session_count: 1,
            app_pct: 100.0,
            muted: true,
            peak: 0.0,
        };
        for channel_muted in [false, true] {
            let mut groups = VolumeGroups {
                watcher: None,
                master: None,
                ids: HashMap::new(),
                streams: Vec::new(),
                applied: HashMap::new(),
                new_app: None,
                muted: HashMap::new(),
                enabled: true,
                refresh: true,
                polled_at: None,
                output: OutputPolicy::default(),
                output_journal: PathBuf::new(),
                following_system: false,
            };
            groups.apps_changed(&mut Host { channel_muted }, vec![app("player.exe")]);
            assert_eq!(
                groups.muted.get("player.exe").copied(),
                (!channel_muted).then_some(true)
            );
        }
    }
    #[test]
    fn system_master_is_not_applied_again_to_sessions() {
        let settings = settings(
            AppMix {
                channel_pct: 70.0,
                app_pct: 80.0,
                master_pct: 20.0,
                channel_muted: false,
                master_muted: true,
            },
            false,
        );
        assert_eq!(settings.master_pct, 100.0);
        assert!(!settings.master_muted);
        assert_eq!(settings.channel_pct, 70.0);
        assert_eq!(settings.app_pct, 80.0);
    }
}
