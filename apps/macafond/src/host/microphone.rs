//! Microphone control on the daemon control loop.

use super::{DaemonHost, engine_error, select_input};
use macafon_config::Config;
use macafon_config::presets;
use macafon_ipc::{CalibrationFailure, ErrorBody, ErrorCode, Event, MicState};
use std::time::Instant;

impl DaemonHost {
    pub(super) fn state(&self) -> MicState {
        let preset_modified = self.config.mic.preset.as_deref().is_some_and(|id| {
            self.preset_settings(id)
                .is_none_or(|preset| !presets::same_sound(&self.config.mic.chain, &preset))
        });
        MicState {
            stereo: self.config.mic.stereo,
            input_channel: self.config.mic.input_channel,
            config_recovery: self.config_recovery.clone(),
            settings: self.config.mic.chain,
            preset: self.config.mic.preset.clone(),
            preset_modified,
            device: self.config.mic.device.clone(),
            fallback_device: self.config.mic.fallback_device.clone(),
            active_device: self.active_device.clone(),
            input_connected: self.input_connected,
            is_default_source: self.default_source.as_deref()
                == Some(macafon_backend::VIRTUAL_MIC_NAME),
            virtual_mic_ready: self.virtual_mic_ready,
            latency_samples: self.config.mic.chain.latency_samples(),
            noise_floor_dbfs: self.config.mic.noise_floor_dbfs,
        }
    }

    pub(super) fn refresh_input(&mut self) {
        let selected = select_input(&self.config.mic, &self.inputs).map(str::to_owned);
        let stable_id = selected
            .as_ref()
            .and_then(|name| self.inputs.iter().find(|d| &d.name == name))
            .map(|d| {
                if d.stable_id.is_empty() {
                    d.name.clone()
                } else {
                    d.stable_id.clone()
                }
            });
        let selection_changed =
            self.selected_device != selected || self.selected_stable_id != stable_id;
        if selection_changed {
            let previous_id = self.selected_stable_id.take();
            if let Some(previous) = &previous_id {
                self.config.device_profiles.insert(
                    previous.clone(),
                    macafon_config::MicProfile::capture(&self.config.mic),
                );
            }
            if let (Some(id), Some(name)) = (&stable_id, &selected) {
                if let Some(profile) = self.config.device_profiles.get(id).cloned() {
                    let result = self.update(|config| profile.apply(&mut config.mic, name));
                    if let Err(error) = result {
                        self.selected_stable_id = previous_id;
                        tracing::warn!(%error, "Cannot load device profile yet");
                        return;
                    }
                } else if previous_id.is_some() {
                    let defaults = macafon_config::MicConfig::default();
                    let result = self.update(|config| {
                        config.mic.chain = defaults.chain;
                        config.mic.preset = defaults.preset;
                        config.mic.noise_floor_dbfs = None;
                        config.mic.noise_floor_device = None;
                        config.mic.input_channel = defaults.input_channel;
                    });
                    if let Err(error) = result {
                        self.selected_stable_id = previous_id;
                        tracing::warn!(%error, "Cannot initialize device profile yet");
                        return;
                    }
                }
            }
            self.selected_stable_id = stable_id;
        }
        let floor_invalid = self.config.mic.noise_floor_dbfs.is_some()
            && self.config.mic.noise_floor_device != selected;
        if floor_invalid {
            self.config.mic.noise_floor_dbfs = None;
            self.config.mic.noise_floor_device = None;
            self.changed_at = Some(Instant::now());
        }
        if selection_changed {
            self.selected_device = selected;
            self.active_device = None;
            self.input_connected = false;
            if self.recording.is_some() {
                self.fail_calibration(
                    CalibrationFailure::Disconnected,
                    "the microphone changed during calibration",
                );
            }
            if self.ab.is_recording() {
                self.ab.abort_recording(self.control.as_mut());
                self.ab_changed();
            }
        }
        if selection_changed || floor_invalid {
            self.changed();
        }
    }

    pub(super) fn noise_fallback(&mut self, settings: macafon_dsp::ChainSettings) {
        self.config.mic.chain = settings;
        if let Some(id) = &self.selected_stable_id {
            self.config.device_profiles.insert(
                id.clone(),
                macafon_config::MicProfile::capture(&self.config.mic),
            );
        }
        self.ab.settings_changed(&settings);
        self.changed_at = Some(Instant::now());
        self.changed();
        self.overloads.push_back(Instant::now());
        self.hub.broadcast(&Event::NoiseFallback {
            engine: settings.noise_suppression.params.engine,
        });
    }

    /// Apply a new configuration to the running engine first; keep it only if accepted.
    pub(super) fn update(
        &mut self,
        update: impl FnOnce(&mut Config),
    ) -> Result<MicState, ErrorBody> {
        let mut config = self.config.clone();
        update(&mut config);
        config
            .mic
            .chain
            .validate()
            .map_err(|error| ErrorBody::new(ErrorCode::InvalidParams, error.to_string()))?;
        if let Some(control) = &mut self.control {
            control.apply(&config.mic.chain).map_err(engine_error)?;
        }
        if config != self.config {
            if config.mic.chain != self.config.mic.chain {
                self.ab.settings_changed(&config.mic.chain);
            }
            if let Some(id) = &self.selected_stable_id {
                config
                    .device_profiles
                    .insert(id.clone(), macafon_config::MicProfile::capture(&config.mic));
            }
            self.config = config;
            self.changed_at = Some(Instant::now());
            self.changed();
        }
        Ok(self.state())
    }

    pub(super) fn cycle_preset(&mut self, step: i32) -> Result<MicState, ErrorBody> {
        let presets = self.preset_list();
        if presets.is_empty() {
            return Err(ErrorBody::new(ErrorCode::NotFound, "no presets"));
        }
        let current = self
            .config
            .mic
            .preset
            .as_deref()
            .and_then(|id| presets.iter().position(|preset| preset.id == id));
        let count = presets.len() as i32;
        let index = match current {
            Some(index) => (index as i32 + step).rem_euclid(count),
            None if step >= 0 => 0,
            None => count - 1,
        };
        let preset = presets[index as usize].id.clone();
        let settings = self
            .preset_settings(&preset)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "preset vanished"))?;
        self.update(|config| {
            let mic = &mut config.mic;
            let (muted, bypass) = (mic.chain.muted, mic.chain.bypass);
            mic.chain = settings;
            mic.chain.muted = muted;
            mic.chain.bypass = bypass;
            mic.preset = Some(preset);
        })
    }
}
