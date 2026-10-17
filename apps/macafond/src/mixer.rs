//! Mixer state on the control loop (PRD §7): live channel engines, applications built from
//! playback streams, output device choice and routing decisions. The desired configuration
//! lives in `Config::mixer` / `Config::apps`; this holds what the graph session needs.

use macafon_apps::{AppIdentity, Category, StreamProps};
use macafon_backend::{AppMix, ChannelRequest, Node, PlaybackStream, Route};
use macafon_config::{AppsConfig, ChannelConfig, Config, MixerConfig};
use macafon_dsp::ChannelTelemetry;
use macafon_engine::{ChannelControl, EngineError};
use macafon_ipc::{
    AppState, ChannelMeterFrame, ChannelState, MixerMeters, MixerState, OutputDevice,
    PlaybackDeviceKind,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often to retry taking over the default output while a channel node is not up yet.
const DEFAULT_SINK_RETRY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Mixer {
    /// A graph session is running; channel nodes can exist.
    session: bool,
    controls: HashMap<String, ChannelControl>,
    telemetry: HashMap<String, Arc<ChannelTelemetry>>,
    requests: Vec<ChannelRequest>,
    outputs: Vec<Node>,
    system_default: Option<String>,
    streams: Vec<PlaybackStream>,
    identities: HashMap<u32, AppIdentity>,
    /// Macafon set the default output to a channel (OUT-005) during this run.
    owns_default: bool,
    default_attempt: Option<Instant>,
    /// The channel the system default points to, as last set.
    default_target: Option<String>,
    /// The default output from before Macafon took over, read from the journal.
    previous_default: Option<String>,
}

fn device_kind(form_factor: &str) -> PlaybackDeviceKind {
    match form_factor {
        "headset" | "headphone" => PlaybackDeviceKind::Headphones,
        "speaker" | "speakers" => PlaybackDeviceKind::Speakers,
        _ => PlaybackDeviceKind::Unknown,
    }
}

impl Mixer {
    // --- Graph session ------------------------------------------------------------------

    pub fn start_session(&mut self, config: &MixerConfig) -> Result<(), EngineError> {
        self.session = true;
        self.previous_default = read_previous_default();
        self.controls.clear();
        self.requests.clear();
        if config.enabled {
            for channel in config.channels.iter().filter(|channel| channel.enabled) {
                self.create(channel, config.master_gain())?;
            }
        }
        Ok(())
    }

    pub fn end_session(&mut self) {
        self.session = false;
        self.controls.clear();
        self.requests.clear();
        self.streams.clear();
        self.identities.clear();
        self.default_target = None;
    }

    pub fn take_requests(&mut self) -> Vec<ChannelRequest> {
        std::mem::take(&mut self.requests)
    }

    fn create(&mut self, channel: &ChannelConfig, master_gain: f32) -> Result<(), EngineError> {
        let telemetry = self
            .telemetry
            .entry(channel.id.clone())
            .or_default()
            .clone();
        let (control, processor) =
            macafon_engine::channel(&channel.settings, master_gain, telemetry)?;
        self.controls.insert(channel.id.clone(), control);
        self.requests.push(ChannelRequest::Create {
            id: channel.id.clone(),
            description: description(channel),
            processor,
        });
        Ok(())
    }

    /// Bring the live channels in line with `config` after any mixer change.
    pub fn sync(&mut self, config: &MixerConfig) -> Result<(), EngineError> {
        if !self.session {
            return Ok(());
        }
        let wanted: Vec<&ChannelConfig> = if config.enabled {
            config
                .channels
                .iter()
                .filter(|channel| channel.enabled)
                .collect()
        } else {
            Vec::new()
        };
        let stale: Vec<String> = self
            .controls
            .keys()
            .filter(|id| !wanted.iter().any(|channel| &channel.id == *id))
            .cloned()
            .collect();
        for id in stale {
            self.controls.remove(&id);
            self.telemetry.remove(&id);
            self.requests.push(ChannelRequest::Remove { id });
        }
        let master = config.master_gain();
        for channel in wanted {
            match self.controls.get_mut(&channel.id) {
                None => self.create(channel, master)?,
                Some(control) => {
                    let latency = control.latency_samples();
                    control.apply(&channel.settings, master)?;
                    if control.latency_samples() != latency {
                        self.requests.push(ChannelRequest::Latency {
                            id: channel.id.clone(),
                            samples: control.latency_samples(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn rename(&mut self, channel: &ChannelConfig) {
        if self.controls.contains_key(&channel.id) {
            self.requests.push(ChannelRequest::Describe {
                id: channel.id.clone(),
                description: description(channel),
            });
        }
    }

    pub fn collect_garbage(&mut self) {
        self.controls
            .values_mut()
            .for_each(ChannelControl::collect_garbage);
    }

    // --- Outputs (OUT-001…005) ------------------------------------------------------------

    pub fn outputs_changed(&mut self, outputs: &[Node]) {
        self.outputs = outputs.to_vec();
    }

    pub fn default_sink_changed(&mut self, name: Option<&str>) {
        self.system_default = name.map(str::to_owned);
    }

    /// OUT-002: the first present device of the priority list; else the default from before
    /// Macafon; else the current system default if it is a real device; else the first one.
    pub fn output_device(&self, config: &MixerConfig) -> Option<String> {
        let present = |name: &str| self.outputs.iter().any(|node| node.name == name);
        config
            .output_priority
            .iter()
            .find(|name| present(name))
            .cloned()
            .or_else(|| self.previous_default.clone().filter(|name| present(name)))
            .or_else(|| self.system_default.clone().filter(|name| present(name)))
            .or_else(|| self.outputs.first().map(|node| node.name.clone()))
    }

    /// OUT-005 while the mixer is on: the default output is the default channel. Retried
    /// every second until the channel node exists.
    pub fn poll_default_sink(&mut self, config: &MixerConfig) {
        if !crate::platform::CHANNEL_NODES || !self.session || !config.enabled {
            return;
        }
        let target = crate::platform::channel_node_name(&config.default_channel);
        if self.default_target.as_deref() == Some(target.as_str()) {
            return;
        }
        if self
            .default_attempt
            .is_some_and(|at| at.elapsed() < DEFAULT_SINK_RETRY)
        {
            return;
        }
        self.default_attempt = Some(Instant::now());
        let result = crate::platform::set_default_sink(&target);
        match result {
            Ok(()) => {
                tracing::info!(channel = %config.default_channel, "Default output set to the default channel");
                self.default_target = Some(target);
                self.owns_default = true;
                self.previous_default = read_previous_default();
            }
            Err(error) => tracing::debug!(%error, "Default output not taken over yet"),
        }
    }

    /// Mixer off or Quit: give the default output back (OUT-005).
    pub fn restore_default_sink(&mut self) {
        self.default_target = None;
        self.default_attempt = None;
        let result = crate::platform::restore_default_sink();
        match result {
            Ok(()) => {
                if std::mem::take(&mut self.owns_default) {
                    tracing::info!("Previous default output restored");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "Cannot restore the default output; recovery journal retained")
            }
        }
    }

    // --- Applications (APP-001…011) ---------------------------------------------------------

    pub fn streams_changed(&mut self, streams: &[PlaybackStream]) {
        self.streams = streams
            .iter()
            .filter(|stream| !stream.ours())
            .cloned()
            .collect();
        self.identities = self
            .streams
            .iter()
            .filter_map(|stream| Some((stream.id, identify(stream)?)))
            .collect();
    }

    /// The channel an app belongs to: its rule, else the built-in classification (APP-006),
    /// else the default channel; channels that no longer exist or are off fall back to the
    /// default.
    fn channel_for(&self, config: &Config, app: &AppIdentity) -> String {
        let mixer = &config.mixer;
        let exists = |id: &str| mixer.active(id);
        if let Some(rule) = config.apps.rule(&app.key) {
            if mixer.channel(&rule.channel).is_some() {
                return mixer.effective_channel(&rule.channel).to_owned();
            }
        }
        if mixer.auto_classify {
            let class = match macafon_apps::classify(app) {
                Category::Voice => Some("voice"),
                Category::Music => Some("music"),
                Category::Other => None,
            };
            if let Some(id) = class.filter(|id| exists(id)) {
                return id.to_owned();
            }
        }
        mixer.default_channel.clone()
    }

    pub fn route(&self, config: &Config, stream: &PlaybackStream) -> Route {
        let Some(app) = self.identities.get(&stream.id) else {
            return Route::Ignore;
        };
        if !config.mixer.enabled || !self.session {
            return Route::Release;
        }
        if config.apps.rule(&app.key).is_some_and(|rule| !rule.managed) {
            return Route::Release;
        }
        Route::Channel(self.channel_for(config, app))
    }

    /// CH-008: linear gain for the app's streams; 1.0 when Macafon does not manage them.
    pub fn stream_volume(&self, config: &Config, stream: &PlaybackStream) -> f32 {
        if self.route(config, stream) == Route::Release || !self.session {
            return 1.0;
        }
        self.identities
            .get(&stream.id)
            .and_then(|app| config.apps.rule(&app.key))
            .map_or(1.0, |rule| {
                macafon_dsp::channel::volume_gain(rule.volume_pct)
            })
    }

    // --- Volume groups (Windows, §12.2.2) -------------------------------------------------------

    /// WIN-CH-002: the faders behind a managed app's volume; `None` while it is not managed.
    pub fn app_mix(&self, config: &Config, stream: &PlaybackStream) -> Option<AppMix> {
        let Route::Channel(id) = self.route(config, stream) else {
            return None;
        };
        let channel = config.mixer.channel(&id)?;
        let app = self.identities.get(&stream.id)?;
        Some(AppMix {
            channel_pct: channel.settings.volume_pct,
            app_pct: config
                .apps
                .rule(&app.key)
                .map_or(100.0, |rule| rule.volume_pct),
            master_pct: config.mixer.master_volume_pct,
            channel_muted: channel.settings.muted,
            master_muted: config.mixer.master_muted,
        })
    }

    /// The app key and its channel, for remembering a volume changed outside Macafon.
    pub fn app_of(&self, config: &Config, stream: &PlaybackStream) -> Option<(String, String)> {
        let app = self.identities.get(&stream.id)?;
        Some((app.key.clone(), self.channel_for(config, app)))
    }

    /// WIN-CH-004: a channel meter shows the loudest of its apps.
    pub fn stream_peaks(&self, config: &Config, peaks: &[(u32, f32)]) {
        for (id, peak) in peaks {
            let Some(stream) = self.streams.iter().find(|stream| stream.id == *id) else {
                continue;
            };
            if let Route::Channel(channel) = self.route(config, stream) {
                if let Some(telemetry) = self.telemetry.get(&channel) {
                    telemetry.raise_peak(*peak);
                }
            }
        }
    }

    // --- State and meters ---------------------------------------------------------------------

    pub fn state(&self, config: &Config) -> MixerState {
        let mixer = &config.mixer;
        let mut apps: BTreeMap<String, AppState> = BTreeMap::new();
        for stream in &self.streams {
            let Some(app) = self.identities.get(&stream.id) else {
                continue;
            };
            let entry = apps.entry(app.key.clone()).or_insert_with(|| AppState {
                key: app.key.clone(),
                name: app.name.clone(),
                icon: app.icon.clone(),
                channel: self.channel_for(config, app),
                active: false,
                streams: 0,
                managed: config.apps.rule(&app.key).is_none_or(|rule| rule.managed),
                movable: true,
                volume_pct: config
                    .apps
                    .rule(&app.key)
                    .map_or(100.0, |rule| rule.volume_pct),
            });
            entry.streams += 1;
            entry.active |= stream.running;
            entry.movable &= stream.movable;
        }
        // Remembered apps that are not running (APP-007).
        for rule in &config.apps.rules {
            if !apps.contains_key(&rule.key) {
                apps.insert(
                    rule.key.clone(),
                    AppState {
                        key: rule.key.clone(),
                        name: rule_name(&rule.key),
                        icon: None,
                        channel: mixer.effective_channel(&rule.channel).to_owned(),
                        active: false,
                        streams: 0,
                        managed: rule.managed,
                        movable: true,
                        volume_pct: rule.volume_pct,
                    },
                );
            }
        }
        let mut apps: Vec<AppState> = apps.into_values().collect();
        apps.sort_by(|a, b| {
            (b.streams > 0, a.name.to_lowercase()).cmp(&(a.streams > 0, b.name.to_lowercase()))
        });
        MixerState {
            enabled: mixer.enabled,
            default_channel: mixer.default_channel.clone(),
            master_volume_pct: mixer.master_volume_pct,
            master_muted: mixer.master_muted,
            auto_classify: mixer.auto_classify,
            channels: mixer
                .channels
                .iter()
                .map(|channel| ChannelState {
                    id: channel.id.clone(),
                    name: channel.name.clone(),
                    color: channel.color.clone(),
                    icon: channel.icon.clone(),
                    enabled: channel.enabled,
                    settings: channel.settings,
                    latency_samples: channel.settings.latency_samples(),
                })
                .collect(),
            apps,
            outputs: self
                .outputs
                .iter()
                .map(|node| OutputDevice {
                    name: node.name.clone(),
                    description: node.description.clone(),
                    kind: device_kind(&node.form_factor),
                })
                .collect(),
            output_priority: mixer.output_priority.clone(),
            output: mixer.enabled.then(|| self.output_device(mixer)).flatten(),
        }
    }

    /// Meter telemetry of the live channels, in display order, for the meter pump.
    pub fn telemetry(&self, config: &MixerConfig) -> Vec<(String, Arc<ChannelTelemetry>)> {
        config
            .channels
            .iter()
            .filter_map(|channel| {
                self.controls
                    .contains_key(&channel.id)
                    .then(|| Some((channel.id.clone(), self.telemetry.get(&channel.id)?.clone())))
                    .flatten()
            })
            .collect()
    }
}

/// Meters of every channel; the Master shows the loudest post-fader channel per side.
pub fn take_meters(channels: &[(String, Arc<ChannelTelemetry>)]) -> MixerMeters {
    let frames: Vec<ChannelMeterFrame> = channels
        .iter()
        .map(|(id, telemetry)| ChannelMeterFrame {
            id: id.clone(),
            meters: telemetry.take(),
        })
        .collect();
    let max =
        |side: fn(&ChannelMeterFrame) -> f32| frames.iter().map(side).fold(-120.0f32, f32::max);
    MixerMeters {
        master_left_dbfs: max(|frame| frame.meters.peak_left_dbfs),
        master_right_dbfs: max(|frame| frame.meters.peak_right_dbfs),
        channels: frames,
    }
}

fn read_previous_default() -> Option<String> {
    crate::platform::previous_default_sink()
}

fn identify(stream: &PlaybackStream) -> Option<AppIdentity> {
    macafon_apps::identify(&StreamProps {
        binary: stream.binary.as_deref(),
        flatpak_id: stream.flatpak_id.as_deref(),
        app_id: stream.app_id.as_deref(),
        name: stream.app_name.as_deref(),
        icon_name: stream.icon_name.as_deref(),
        media_role: stream.media_role.as_deref(),
    })
}

/// A readable name for a remembered app without live streams.
fn rule_name(key: &str) -> String {
    if key == macafon_apps::SYSTEM_SOUNDS_KEY {
        return "System sounds".into();
    }
    let raw = key.split_once(':').map_or(key, |(_, rest)| rest);
    let last = raw.rsplit('.').next().unwrap_or(raw);
    let mut chars = last.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// What apps and other mixers see: "Macafon Voice" (§7.1).
fn description(channel: &ChannelConfig) -> String {
    format!("Macafon {}", channel.name)
}

/// Move `app` and remember it (APP-004/005); apps are created on first move.
pub fn assign(apps: &mut AppsConfig, key: &str, channel: &str) {
    apps.set(key, channel);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: u32, binary: &str) -> PlaybackStream {
        PlaybackStream {
            id,
            serial: id.to_string(),
            node_name: format!("{binary}-stream"),
            binary: Some(binary.into()),
            movable: true,
            running: true,
            ..PlaybackStream::default()
        }
    }

    #[test]
    fn rules_beat_classification_and_vanished_channels_fall_back_to_default() {
        let mut config = Config::default();
        config.mixer.enabled = true;
        let mut mixer = Mixer::default();
        mixer.start_session(&config.mixer).expect("channels build");
        assert_eq!(mixer.take_requests().len(), 3);
        mixer.streams_changed(&[
            stream(1, "Discord"),
            stream(2, "firefox"),
            stream(3, "Discord"),
        ]);
        assert_eq!(
            mixer.route(&config, &mixer.streams[0]),
            Route::Channel("voice".into())
        );
        assert_eq!(
            mixer.route(&config, &mixer.streams[1]),
            Route::Channel("main".into())
        );
        assign(&mut config.apps, "bin:firefox", "music");
        assert_eq!(
            mixer.route(&config, &mixer.streams[1]),
            Route::Channel("music".into())
        );
        let state = mixer.state(&config);
        let discord = state
            .apps
            .iter()
            .find(|app| app.key == "bin:discord")
            .expect("grouped");
        assert_eq!((discord.streams, discord.channel.as_str()), (2, "voice"));
        config
            .mixer
            .channels
            .retain(|channel| channel.id != "music");
        assert_eq!(
            mixer.route(&config, &mixer.streams[1]),
            Route::Channel("main".into())
        );
        mixer.sync(&config.mixer).expect("sync");
        assert!(matches!(
            mixer.take_requests().as_slice(),
            [ChannelRequest::Remove { id }] if id == "music"
        ));
        // CH-008: the app's fader follows the channel curve and only while Macafon manages it.
        config.apps.rules[0].volume_pct = 50.0;
        assert_eq!(mixer.stream_volume(&config, &mixer.streams[1]), 0.125);
        assert_eq!(mixer.stream_volume(&config, &mixer.streams[0]), 1.0);
        config.apps.rules[0].managed = false;
        assert_eq!(mixer.route(&config, &mixer.streams[1]), Route::Release);
        assert_eq!(mixer.stream_volume(&config, &mixer.streams[1]), 1.0);
        config.mixer.enabled = false;
        assert_eq!(mixer.route(&config, &mixer.streams[0]), Route::Release);
    }

    #[test]
    fn a_switched_off_channel_loses_its_node_and_lends_its_apps_to_the_default() {
        let mut config = Config::default();
        config.mixer.enabled = true;
        let mut mixer = Mixer::default();
        mixer.start_session(&config.mixer).expect("channels build");
        mixer.take_requests();
        mixer.streams_changed(&[stream(1, "Discord"), stream(2, "firefox")]);
        assign(&mut config.apps, "bin:firefox", "voice");
        let voice = Route::Channel("voice".into());
        assert_eq!(mixer.route(&config, &mixer.streams[0]), voice);
        config.mixer.channel_mut("voice").expect("voice").enabled = false;
        let main = Route::Channel("main".into());
        assert_eq!(mixer.route(&config, &mixer.streams[0]), main);
        assert_eq!(mixer.route(&config, &mixer.streams[1]), main);
        assert!(
            mixer
                .state(&config)
                .apps
                .iter()
                .all(|app| app.channel == "main")
        );
        mixer.sync(&config.mixer).expect("sync");
        assert!(matches!(
            mixer.take_requests().as_slice(),
            [ChannelRequest::Remove { id }] if id == "voice"
        ));
        assert_eq!(
            config.apps.rule("bin:firefox").expect("kept").channel,
            "voice"
        );
        config.mixer.channel_mut("voice").expect("voice").enabled = true;
        assert_eq!(mixer.route(&config, &mixer.streams[1]), voice);
        mixer.sync(&config.mixer).expect("sync");
        assert!(matches!(
            mixer.take_requests().as_slice(),
            [ChannelRequest::Create { id, .. }] if id == "voice"
        ));
    }

    #[test]
    fn volume_groups_combine_channel_app_and_master_faders_of_managed_apps() {
        let mut config = Config::default();
        config.mixer.enabled = true;
        config.mixer.master_volume_pct = 80.0;
        let mut mixer = Mixer::default();
        mixer.start_session(&config.mixer).expect("channels build");
        mixer.streams_changed(&[stream(1, "discord"), stream(2, "firefox")]);
        let voice = config.mixer.channel_mut("voice").expect("voice");
        voice.settings.volume_pct = 50.0;
        voice.settings.muted = true;
        assign(&mut config.apps, "bin:discord", "voice");
        config.apps.rules[0].volume_pct = 40.0;
        assert_eq!(
            mixer.app_mix(&config, &mixer.streams[0]),
            Some(AppMix {
                channel_pct: 50.0,
                app_pct: 40.0,
                master_pct: 80.0,
                channel_muted: true,
                master_muted: false,
            })
        );
        config.apps.rules[0].managed = false;
        assert_eq!(mixer.app_mix(&config, &mixer.streams[0]), None);
        config.mixer.enabled = false;
        assert_eq!(mixer.app_mix(&config, &mixer.streams[1]), None);
    }
}
