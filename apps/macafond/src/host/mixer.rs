//! Mixer control on the daemon control loop.

use super::{DaemonHost, engine_error};
use macafon_config::Config;
use macafon_ipc::{ErrorBody, ErrorCode, Event, Request};
use std::time::Instant;

impl DaemonHost {
    pub(super) fn mixer_state(&self) -> macafon_ipc::MixerState {
        self.mixer.state(&self.config)
    }

    pub(super) fn mixer_changed(&mut self) {
        self.hub.broadcast(&Event::MixerChanged(self.mixer_state()));
        self.update_tray();
    }

    pub(super) fn mixer_telemetry_changed(&self) {
        self.hub
            .set_mixer_telemetry(self.mixer.telemetry(&self.config.mixer));
    }

    /// Apply a mixer change to the live channels first; keep it only if accepted.
    pub(super) fn update_mixer(
        &mut self,
        update: impl FnOnce(&mut Config) -> Result<(), ErrorBody>,
    ) -> Result<serde_json::Value, ErrorBody> {
        let mut config = self.config.clone();
        update(&mut config)?;
        config
            .mixer
            .validate()
            .map_err(|message| ErrorBody::new(ErrorCode::InvalidParams, message))?;
        self.mixer.sync(&config.mixer).map_err(engine_error)?;
        let was_enabled = self.config.mixer.enabled;
        for channel in &config.mixer.channels {
            if self
                .config
                .mixer
                .channel(&channel.id)
                .is_some_and(|old| old.name != channel.name)
            {
                self.mixer.rename(channel);
            }
        }
        if config != self.config {
            self.config = config;
            self.changed_at = Some(Instant::now());
        }
        if was_enabled && !self.config.mixer.enabled {
            // Give the default output back while the channels still exist (OUT-005).
            self.mixer.restore_default_sink();
        }
        self.mixer_telemetry_changed();
        self.update_tray();
        self.rebind_hotkeys();
        self.hotkeys_changed();
        let state = self.mixer_state();
        self.hub.broadcast(&Event::MixerChanged(state.clone()));
        serde_json::to_value(state)
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }

    pub(super) fn respond_mixer(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        let missing = |id: &str| ErrorBody::new(ErrorCode::NotFound, format!("no channel {id:?}"));
        match request {
            Request::MixerGet => serde_json::to_value(self.mixer_state())
                .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string())),
            Request::MixerSetEnabled { enabled } => {
                tracing::info!(enabled, "Mixer switched");
                self.update_mixer(|config| {
                    config.mixer.enabled = enabled;
                    Ok(())
                })
            }
            Request::MixerSetMaster { volume_pct, muted } => self.update_mixer(|config| {
                if let Some(volume) = volume_pct {
                    config.mixer.master_volume_pct = volume;
                }
                if let Some(muted) = muted {
                    config.mixer.master_muted = muted;
                }
                Ok(())
            }),
            Request::MixerSetOutput { device } => self.update_mixer(|config| {
                let priority = &mut config.mixer.output_priority;
                match device {
                    Some(device) => {
                        priority.retain(|name| name != &device);
                        priority.insert(0, device);
                    }
                    None => priority.clear(),
                }
                Ok(())
            }),
            Request::MixerSetOutputPriority { devices } => self.update_mixer(|config| {
                config.mixer.output_priority = devices;
                Ok(())
            }),
            Request::MixerSetAutoClassify { enabled } => self.update_mixer(|config| {
                config.mixer.auto_classify = enabled;
                Ok(())
            }),
            Request::ChannelCreate { name, color, icon } => self.update_mixer(|config| {
                if config.mixer.channels.len() >= macafon_config::mixer::MAX_CHANNELS {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        format!("at most {} channels", macafon_config::mixer::MAX_CHANNELS),
                    ));
                }
                let id = config.mixer.next_id();
                let icon = icon.unwrap_or_else(|| macafon_config::mixer::CHANNEL_ICONS[0].into());
                config
                    .mixer
                    .channels
                    .push(macafon_config::ChannelConfig::new(
                        &id,
                        name.trim(),
                        &color,
                        &icon,
                        macafon_dsp::channel::ChannelTemplate::Custom,
                    ));
                Ok(())
            }),
            Request::ChannelUpdate { id, patch } => self.update_mixer(|config| {
                let channel = config.mixer.channel_mut(&id).ok_or_else(|| missing(&id))?;
                if let Some(name) = patch.name {
                    channel.name = name.trim().to_owned();
                }
                if let Some(color) = patch.color {
                    channel.color = color;
                }
                if let Some(icon) = patch.icon {
                    channel.icon = icon;
                }
                Ok(())
            }),
            Request::ChannelDelete { id } => self.update_mixer(|config| {
                if config.mixer.default_channel == id {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "the default channel cannot be deleted",
                    ));
                }
                config.mixer.channel(&id).ok_or_else(|| missing(&id))?;
                config.mixer.channels.retain(|channel| channel.id != id);
                // CH-002: its apps move to the default channel.
                let default = config.mixer.default_channel.clone();
                for rule in &mut config.apps.rules {
                    if rule.channel == id {
                        rule.channel.clone_from(&default);
                    }
                }
                Ok(())
            }),
            Request::ChannelSet { id, settings } => self.update_mixer(|config| {
                config
                    .mixer
                    .channel_mut(&id)
                    .ok_or_else(|| missing(&id))?
                    .settings = settings;
                Ok(())
            }),
            Request::ChannelAdjustVolume { id, delta_pct } => self.update_mixer(|config| {
                let settings = &mut config
                    .mixer
                    .channel_mut(&id)
                    .ok_or_else(|| missing(&id))?
                    .settings;
                settings.volume_pct = (settings.volume_pct + delta_pct).clamp(0.0, 100.0);
                Ok(())
            }),
            Request::ChannelToggleMute { id } => self.update_mixer(|config| {
                let settings = &mut config
                    .mixer
                    .channel_mut(&id)
                    .ok_or_else(|| missing(&id))?
                    .settings;
                settings.muted = !settings.muted;
                Ok(())
            }),
            Request::ChannelSetDefault { id } => self.update_mixer(|config| {
                config.mixer.channel(&id).ok_or_else(|| missing(&id))?;
                config.mixer.default_channel = id;
                Ok(())
            }),
            Request::ChannelReorder { ids } => self.update_mixer(|config| {
                let channels = &mut config.mixer.channels;
                let mut sorted = ids.clone();
                sorted.sort();
                let mut current: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
                current.sort();
                if sorted != current {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "the new order must list every channel once",
                    ));
                }
                channels.sort_by_key(|channel| ids.iter().position(|id| id == &channel.id));
                Ok(())
            }),
            Request::AppsMove { app_key, channel } => self.update_mixer(|config| {
                config
                    .mixer
                    .channel(&channel)
                    .ok_or_else(|| missing(&channel))?;
                crate::mixer::assign(&mut config.apps, &app_key, &channel);
                if let Some(rule) = config
                    .apps
                    .rules
                    .iter_mut()
                    .find(|rule| rule.key == app_key)
                {
                    rule.managed = true;
                }
                Ok(())
            }),
            Request::AppsSetVolume {
                app_key,
                volume_pct,
            } => self.update_mixer(|config| {
                if !(0.0..=100.0).contains(&volume_pct) {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "volume_pct must be 0–100",
                    ));
                }
                let channel = config
                    .apps
                    .rule(&app_key)
                    .map(|rule| rule.channel.clone())
                    .unwrap_or_else(|| config.mixer.default_channel.clone());
                crate::mixer::assign(&mut config.apps, &app_key, &channel);
                if let Some(rule) = config
                    .apps
                    .rules
                    .iter_mut()
                    .find(|rule| rule.key == app_key)
                {
                    rule.volume_pct = volume_pct;
                }
                Ok(())
            }),
            Request::AppsSetManaged { app_key, managed } => self.update_mixer(|config| {
                let channel = config
                    .apps
                    .rule(&app_key)
                    .map(|rule| rule.channel.clone())
                    .unwrap_or_else(|| config.mixer.default_channel.clone());
                crate::mixer::assign(&mut config.apps, &app_key, &channel);
                if let Some(rule) = config
                    .apps
                    .rules
                    .iter_mut()
                    .find(|rule| rule.key == app_key)
                {
                    rule.managed = managed;
                }
                Ok(())
            }),
            Request::AppsForget { app_key } => self.update_mixer(|config| {
                config.apps.rules.retain(|rule| rule.key != app_key);
                Ok(())
            }),
            _ => Err(ErrorBody::new(ErrorCode::BadRequest, "not a mixer request")),
        }
    }
}
