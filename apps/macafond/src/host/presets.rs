//! Presets control on the daemon control loop.

use super::{DaemonHost, preset_error, reply};
use macafon_config::presets::{self, PresetError};
use macafon_dsp::{ChainSettings, Preset};
use macafon_ipc::{ErrorBody, ErrorCode, Event, PresetInfo, Request};
use std::time::Instant;

impl DaemonHost {
    /// Settings of a built-in or user preset.
    pub(super) fn preset_settings(&self, id: &str) -> Option<ChainSettings> {
        Preset::from_id(id)
            .map(Preset::settings)
            .or_else(|| self.presets.get(id).map(|preset| preset.settings))
    }

    pub(super) fn preset_list(&self) -> Vec<PresetInfo> {
        let builtin = Preset::ALL.into_iter().map(|preset| PresetInfo {
            id: preset.id().to_owned(),
            name: preset.name().to_owned(),
            builtin: true,
        });
        let user = self.presets.list().iter().map(|preset| PresetInfo {
            id: preset.id.clone(),
            name: preset.name.clone(),
            builtin: false,
        });
        builtin.chain(user).collect()
    }

    pub(super) fn preset_info(&self, id: &str) -> Result<serde_json::Value, ErrorBody> {
        let info = self
            .preset_list()
            .into_iter()
            .find(|preset| preset.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::Internal, "preset vanished"))?;
        Ok(serde_json::json!(info))
    }

    /// Select `id` as the current preset without touching the chain.
    pub(super) fn mark_preset(&mut self, id: Option<String>) {
        if self.config.mic.preset != id {
            self.config.mic.preset = id;
            self.changed_at = Some(Instant::now());
        }
    }

    pub(super) fn respond_presets(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        let selected = match request {
            Request::PresetsSave { name } => {
                let id = self
                    .presets
                    .create(&name, &self.config.mic.chain)
                    .map_err(preset_error)?
                    .id
                    .clone();
                self.mark_preset(Some(id.clone()));
                id
            }
            Request::PresetsUpdate { id } => {
                self.presets
                    .update(&id, &self.config.mic.chain)
                    .map_err(preset_error)?;
                self.mark_preset(Some(id.clone()));
                id
            }
            Request::PresetsRename { id, name } => {
                self.presets.rename(&id, &name).map_err(preset_error)?;
                id
            }
            Request::PresetsDuplicate { id } => {
                let (name, settings) = match Preset::from_id(&id) {
                    Some(preset) => (preset.name().to_owned(), preset.settings()),
                    None => self
                        .presets
                        .get(&id)
                        .map(|preset| (preset.name.clone(), preset.settings))
                        .ok_or_else(|| preset_error(PresetError::NotFound(id.clone())))?,
                };
                self.presets
                    .add_unique(&name, &settings)
                    .map_err(preset_error)?
                    .id
                    .clone()
            }
            Request::PresetsDelete { id } => {
                self.presets.delete(&id).map_err(preset_error)?;
                if self.config.mic.preset.as_deref() == Some(id.as_str()) {
                    self.mark_preset(None);
                }
                self.presets_changed();
                return Ok(serde_json::Value::Null);
            }
            Request::PresetsExport { id } => {
                let document = match Preset::from_id(&id) {
                    Some(preset) => presets::export(preset.name(), &preset.settings()),
                    None => self
                        .presets
                        .get(&id)
                        .map(|preset| presets::export(&preset.name, &preset.settings))
                        .ok_or_else(|| preset_error(PresetError::NotFound(id.clone())))?,
                };
                return reply(document);
            }
            Request::PresetsImport { document } => {
                let file = presets::parse(&document.to_string()).map_err(preset_error)?;
                self.presets.import(&file).map_err(preset_error)?.id.clone()
            }
            _ => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "not a preset request",
                ));
            }
        };
        self.presets_changed();
        self.preset_info(&selected)
    }

    pub(super) fn presets_changed(&mut self) {
        self.hub
            .broadcast(&Event::PresetsChanged(self.preset_list()));
        // Saving or editing a preset can change `preset` and `preset_modified`.
        self.changed();
    }
}
