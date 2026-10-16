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

pub(crate) struct VolumeGroups {
    watcher: Option<SessionWatcher>,
    /// Stable stream ids per executable for this run.
    ids: HashMap<String, u32>,
    streams: Vec<(String, PlaybackStream)>,
    /// What was last written per executable; external changes update it too.
    applied: HashMap<String, MixSettings>,
    /// App mutes set outside Macafon (WIN-CH-003); Macafon has no app mute of its own.
    muted: HashMap<String, bool>,
    enabled: bool,
    refresh: bool,
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
            ids: HashMap::new(),
            streams: Vec::new(),
            applied: HashMap::new(),
            muted: HashMap::new(),
            enabled: false,
            refresh: true,
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
                MixerEvent::Error(message) => tracing::warn!(%message, "App volumes"),
                MixerEvent::RecoveryPending { sessions } => {
                    tracing::info!(sessions, "App volumes to restore when their apps return");
                }
            }
        }
        let enabled = host.mixer_enabled();
        if self.enabled && !enabled {
            self.release(Some(watcher));
        }
        self.enabled = enabled;
        // Every tick: the apps carry fresh peaks for the meters.
        match watcher.apps() {
            Ok(apps) => self.apps_changed(host, apps),
            Err(error) => tracing::debug!(%error, "Cannot list app sessions"),
        }
        if !enabled {
            return;
        }
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
            streams.push((app.exe.clone(), stream(id, &app)));
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
        let priority: Vec<String> = host.output_device().into_iter().collect();
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
        master_pct: mix.master_pct,
        channel_muted: mix.channel_muted,
        app_muted,
        master_muted: mix.master_muted,
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
        app_name: Some(if named {
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
