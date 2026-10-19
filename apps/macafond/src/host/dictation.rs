//! Dictation IPC and control-loop integration. Heavy I/O belongs to the model/audio workers.
use super::DaemonHost;
use macafon_ipc::*;
use std::time::{Duration, Instant};

/// A hold shortcut starts dictation only after it is held this long, so a brush of the keys
/// or fast typing never starts it; the pre-roll keeps the speech said meanwhile.
const HOLD_DELAY: Duration = Duration::from_millis(250);
/// Two short taps this close start hands-free dictation.
const DOUBLE_TAP: Duration = Duration::from_millis(400);

impl DaemonHost {
    pub(super) fn dictation_hotkey(&mut self, action: HotkeyAction, pressed: bool) {
        if !self.config.dictation.enabled {
            return;
        }
        let result = match action {
            HotkeyAction::DictationHold | HotkeyAction::DictationHoldCopy => {
                let copy = action == HotkeyAction::DictationHoldCopy;
                let now = Instant::now();
                if pressed
                    && self.dictation.hands_free
                    && self.dictation.phase == DictationPhase::Listening
                {
                    self.finish_dictation()
                } else if pressed {
                    let double =
                        self.dictation
                            .last_press
                            .take()
                            .is_some_and(|(last, was_copy)| {
                                was_copy == copy && now.duration_since(last) < DOUBLE_TAP
                            });
                    if double {
                        self.dictation.pending_hold = None;
                        self.begin_dictation(false, copy, true)
                    } else {
                        // Started by `tick_dictation` once held for HOLD_DELAY.
                        self.dictation.pending_hold = Some((now, copy));
                        Ok(())
                    }
                } else if self.dictation.pending_hold.take().is_some() {
                    // Released before the delay: a tap, not a dictation.
                    self.dictation.last_press = Some((now, copy));
                    Ok(())
                } else if !self.dictation.hands_free
                    && self.dictation.phase == DictationPhase::Listening
                {
                    self.finish_dictation()
                } else {
                    Ok(())
                }
            }
            HotkeyAction::DictationToggle => {
                if self.dictation.phase == DictationPhase::Listening {
                    self.finish_dictation()
                } else {
                    self.begin_dictation(false, false, true)
                }
            }
            HotkeyAction::DictationCopyLast | HotkeyAction::DictationPasteLast => self
                .dictation
                .last_action(
                    &self.config.dictation.prefs(),
                    action == HotkeyAction::DictationCopyLast,
                )
                .map_err(super::backend_error),
            HotkeyAction::DictationEnableToggle => self
                .respond_dictation(Request::DictationSetPrefs {
                    patch: DictationPrefsPatch {
                        enabled: Some(!self.config.dictation.enabled),
                        ..Default::default()
                    },
                })
                .map(|_| ()),
            HotkeyAction::DictationLanguageCycle => {
                let prefs = &mut self.config.dictation;
                if prefs.engine == DictationEngine::Parakeet {
                    prefs.language = DictationLanguagePolicy::Auto;
                } else if prefs.language != DictationLanguagePolicy::Fixed {
                    prefs.language = DictationLanguagePolicy::Fixed;
                    prefs.fixed_language = DictationLanguage::En;
                } else {
                    match prefs.fixed_language {
                        DictationLanguage::En => prefs.fixed_language = DictationLanguage::Uk,
                        DictationLanguage::Uk => prefs.fixed_language = DictationLanguage::Ru,
                        DictationLanguage::Ru => prefs.language = DictationLanguagePolicy::Layout,
                    }
                }
                self.changed_at = Some(Instant::now());
                self.dictation_changed();
                Ok(())
            }
            _ => Ok(()),
        };
        if result.is_err() {
            self.dictation.phase = DictationPhase::Error;
            self.dictation.popup_state = crate::osd::dictation::State::Error;
            self.show_dictation_popup();
            self.dictation.error = Some("dictation action failed".into());
            self.dictation_changed();
        }
    }

    pub(super) fn dictation_changed(&mut self) {
        self.hub.broadcast_dictation(&Event::DictationChanged(
            self.dictation.state(self.config.dictation.prefs()),
        ));
        self.update_tray();
    }

    fn show_dictation_popup(&self) {
        let prefs = self.config.dictation.prefs();
        let (level, bands) = self
            .dictation
            .audio
            .as_ref()
            .and_then(|a| a.levels.lock().ok().map(|v| *v))
            .unwrap_or((
                DictationLevel {
                    rms_dbfs: -120.0,
                    peak_dbfs: -120.0,
                },
                [0.0; 15],
            ));
        let view = crate::osd::dictation::View {
            look: prefs.popup_look,
            state: self.dictation.popup_state,
            rms_dbfs: level.rms_dbfs,
            bands,
            elapsed_s: self
                .dictation
                .started
                .map_or(self.dictation.audio_elapsed_s, |s| {
                    s.elapsed().as_secs() as u32
                }),
            words: self
                .dictation
                .last
                .as_ref()
                .map_or(0, |r| r.text.split_whitespace().count()),
            engine: if prefs.engine == DictationEngine::Whisper {
                "WHISPER"
            } else {
                "PARAKEET"
            }
            .into(),
            language: if prefs.engine == DictationEngine::Parakeet
                || prefs.language == DictationLanguagePolicy::Auto
            {
                "AUTO".into()
            } else if prefs.language == DictationLanguagePolicy::Fixed {
                format!("{:?}", prefs.fixed_language).to_uppercase()
            } else {
                format!(
                    "{:?}",
                    self.dictation
                        .language
                        .layout
                        .unwrap_or(self.dictation.language.fixed)
                )
                .to_uppercase()
            },
            hands_free: self.dictation.hands_free,
        };
        crate::osd::show(
            crate::osd::Osd::Dictation(view),
            macafon_config::OsdConfig {
                // Independent of the hotkey popup's monitor setting.
                primary_monitor: prefs.popup_primary_monitor,
                style: macafon_config::OsdStyle::Horizontal,
                edge: if prefs.popup_position == DictationPopupPosition::Top {
                    macafon_config::OsdEdge::Top
                } else {
                    macafon_config::OsdEdge::Bottom
                },
                offset_pct: prefs.popup_offset_pct,
                ..Default::default()
            },
        );
    }

    pub(super) fn respond_dictation(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        let invalid =
            |error: anyhow::Error| ErrorBody::new(ErrorCode::InvalidParams, error.to_string());
        if !cfg!(windows) && !matches!(request, Request::DictationGet) {
            return Err(ErrorBody::new(
                ErrorCode::BadRequest,
                "dictation is available on Windows only",
            ));
        }
        match request {
            Request::DictationGet => {
                self.dictation.models.ensure_scanned();
            }
            Request::DictationSetPrefs { patch } => {
                let previous = self.config.dictation.prefs();
                let mut value = serde_json::to_value(&previous).map_err(super::backend_error)?;
                let patch = serde_json::to_value(patch).map_err(super::backend_error)?;
                if let (Some(value), Some(patch)) = (value.as_object_mut(), patch.as_object()) {
                    for (key, field) in patch {
                        value.insert(key.clone(), field.clone());
                    }
                }
                let mut prefs: DictationPrefs =
                    serde_json::from_value(value).map_err(super::backend_error)?;
                if ![0, 60, 180, 600, -1].contains(&prefs.unload_after_s)
                    || prefs.threads > self.dictation.logical_cores
                    || prefs.vocabulary.len() > 50
                    || !prefs.popup_offset_pct.is_finite()
                    || !(0.0..=macafon_config::OSD_MAX_OFFSET_PCT).contains(&prefs.popup_offset_pct)
                {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "invalid dictation preferences",
                    ));
                }
                if prefs.engine == DictationEngine::Parakeet {
                    prefs.language = DictationLanguagePolicy::Auto;
                }
                if prefs.enabled {
                    let valid = prefs.model.as_deref().is_some_and(|id| {
                        self.dictation.models.models.iter().any(|m| {
                            m.id == id
                                && m.engine == prefs.engine
                                && m.status == DictationModelStatus::Ready
                        })
                    });
                    if !valid {
                        let ready = self.dictation.models.models.iter().find(|m| {
                            m.engine == prefs.engine && m.status == DictationModelStatus::Ready
                        });
                        prefs.model = Some(
                            ready
                                .ok_or_else(|| {
                                    ErrorBody::new(
                                        if self.dictation.models.busy() {
                                            ErrorCode::Busy
                                        } else {
                                            ErrorCode::NotFound
                                        },
                                        if self.dictation.models.busy() {
                                            "model verification is in progress"
                                        } else {
                                            "download a model for the selected engine first"
                                        },
                                    )
                                })?
                                .id
                                .clone(),
                        );
                    }
                }
                if prefs.enabled != previous.enabled
                    || prefs.engine != previous.engine
                    || prefs.model != previous.model
                    || prefs.threads != previous.threads
                    || prefs.gpu != previous.gpu
                    || !prefs.enabled
                {
                    self.dictation.cancel(prefs.enabled);
                }
                if !prefs.keep_history {
                    self.dictation.history.clear();
                }
                if !prefs.keep_history || !prefs.save_history {
                    crate::dictation::delete_saved_history();
                } else if !previous.save_history {
                    crate::dictation::save_history(&self.dictation.history);
                }
                let rebuild_tap = prefs.pre_roll != previous.pre_roll
                    || prefs.tap != previous.tap
                    || prefs.enabled != previous.enabled;
                self.config.dictation.set_prefs(prefs);
                if rebuild_tap {
                    self.dictation.cancel(self.config.dictation.enabled);
                    self.stop_dictation_audio();
                }
                self.changed_at = Some(Instant::now());
                self.ensure_dictation_audio();
                self.rebind_hotkeys();
                self.hotkeys_changed();
                self.dictation_changed();
            }
            Request::DictationDownload { model } => {
                self.dictation.models.download(&model).map_err(invalid)?;
                self.dictation_changed();
            }
            Request::DictationCancelDownload { model } => self
                .dictation
                .models
                .cancel_download(&model)
                .map_err(invalid)?,
            Request::DictationDeleteModel { model } => {
                if self.dictation.loaded_model.as_deref() == Some(&model) {
                    self.cancel_dictation();
                }
                self.dictation.models.delete(&model).map_err(invalid)?;
                if self.config.dictation.model.as_deref() == Some(&model) {
                    self.config.dictation.enabled = false;
                    self.stop_dictation_audio();
                    self.dictation.unload(false);
                    self.changed_at = Some(Instant::now());
                    self.rebind_hotkeys();
                }
                self.dictation_changed();
            }
            Request::DictationImportModel { path } => {
                self.cancel_dictation();
                self.dictation.models.import(path.into()).map_err(invalid)?
            }
            Request::DictationSetModelsDir { folder } => {
                self.cancel_dictation();
                self.dictation
                    .models
                    .move_to(
                        folder
                            .map(Into::into)
                            .unwrap_or_else(crate::dictation::models::default_folder),
                    )
                    .map_err(invalid)?;
            }
            Request::DictationCancelMove => self.dictation.models.cancel_move(),
            Request::DictationTestStart => self.begin_dictation(true, false, false)?,
            Request::DictationTestStop => self.finish_dictation()?,
            Request::DictationCancel => self.cancel_dictation(),
            Request::DictationHistoryClear => {
                self.dictation.history.clear();
                crate::dictation::delete_saved_history();
                self.dictation_changed();
            }
            Request::DictationReinsert { id } => self
                .dictation
                .reinsert(id, &self.config.dictation.prefs())
                .map_err(invalid)?,
            _ => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "not a dictation request",
                ));
            }
        }
        Ok(serde_json::json!(
            self.dictation.state(self.config.dictation.prefs())
        ))
    }

    pub(super) fn ensure_dictation_audio(&mut self) {
        if !cfg!(windows) || !self.config.dictation.enabled || self.dictation.audio.is_some() {
            return;
        }
        self.dictation.models.ensure_scanned();
        if !self
            .config
            .dictation
            .model
            .as_deref()
            .is_some_and(|id| self.dictation.models.ready(id))
        {
            return;
        }
        let Some(control) = &mut self.control else {
            return;
        };
        let point = match self.config.dictation.tap {
            DictationTap::Raw => macafon_dsp::TapPoint::Raw,
            DictationTap::Clean => macafon_dsp::TapPoint::Clean,
            DictationTap::Processed => macafon_dsp::TapPoint::Processed,
        };
        match control
            .enable_dictation(self.config.dictation.pre_roll, point)
            .map_err(anyhow::Error::from)
            .and_then(|reader| crate::dictation::audio(reader, self.hub.clone()))
        {
            Ok(audio) => self.dictation.audio = Some(audio),
            Err(_) => {
                self.dictation.phase = DictationPhase::Error;
                self.dictation.error = Some("cannot prepare dictation audio".into());
            }
        }
    }
    pub(super) fn stop_dictation_audio(&mut self) {
        self.dictation.audio = None;
        if let Some(control) = &mut self.control {
            let _ = control.disable_dictation();
        }
    }
    pub(super) fn begin_dictation(
        &mut self,
        test: bool,
        copy: bool,
        hands_free: bool,
    ) -> Result<(), ErrorBody> {
        self.ensure_dictation_audio();
        self.dictation
            .begin(&self.config.dictation.prefs(), test, copy, hands_free)
            .map_err(super::backend_error)?;
        if let Some(control) = &self.control {
            control.mute_for_dictation(self.config.dictation.mute_mic_while_dictating);
        }
        crate::hotkeys::dictation_recording(self.dictation.phase == DictationPhase::Listening);
        self.show_dictation_popup();
        self.dictation_changed();
        Ok(())
    }
    pub(super) fn finish_dictation(&mut self) -> Result<(), ErrorBody> {
        self.dictation
            .finish(&self.config.dictation.prefs())
            .map_err(super::backend_error)?;
        if let Some(control) = &self.control {
            control.mute_for_dictation(false);
        }
        crate::hotkeys::dictation_recording(false);
        self.show_dictation_popup();
        self.dictation_changed();
        Ok(())
    }
    pub(super) fn cancel_dictation(&mut self) {
        self.dictation.cancel(self.config.dictation.enabled);
        if let Some(control) = &self.control {
            control.mute_for_dictation(false);
        }
        crate::hotkeys::dictation_recording(false);
        self.dictation_changed();
    }
    pub(super) fn tick_dictation(&mut self) {
        self.ensure_dictation_audio();
        if let Some((since, copy)) = self.dictation.pending_hold {
            if since.elapsed() >= HOLD_DELAY {
                self.dictation.pending_hold = None;
                if self.begin_dictation(false, copy, false).is_err() {
                    self.dictation.phase = DictationPhase::Error;
                    self.dictation.popup_state = crate::osd::dictation::State::Error;
                    self.dictation.error = Some("dictation action failed".into());
                    self.show_dictation_popup();
                    self.dictation_changed();
                }
            }
        }
        if self
            .dictation
            .audio
            .as_ref()
            .is_some_and(|a| a.failed.load(std::sync::atomic::Ordering::Acquire))
        {
            self.cancel_dictation();
            self.stop_dictation_audio();
            self.dictation.phase = DictationPhase::Error;
            self.dictation.popup_state = crate::osd::dictation::State::Error;
            self.dictation.error = Some("dictation audio overflowed".into());
            self.show_dictation_popup();
        }
        let (models_changed, moved) = self.dictation.models.poll();
        if let Some(folder) = moved {
            let old = self.config.dictation.models_dir.clone();
            self.config.dictation.models_dir =
                if folder == crate::dictation::models::default_folder() {
                    String::new()
                } else {
                    folder.to_string_lossy().into_owned()
                };
            let saved = self
                .store
                .as_ref()
                .is_none_or(|store| store.save(&self.config).is_ok());
            if !saved {
                self.config.dictation.models_dir = old;
            }
            self.dictation.models.commit_move(saved);
        }
        if models_changed {
            if let Some(error) = &self.dictation.models.error {
                self.dictation.error = Some(error.clone());
            }
        }
        if self
            .dictation
            .started
            .is_some_and(|s| s.elapsed().as_secs() >= 300)
        {
            let _ = self.finish_dictation();
        }
        let (changed, result) = self.dictation.poll(&self.config.dictation.prefs());
        if changed
            || matches!(
                self.dictation.phase,
                DictationPhase::Listening | DictationPhase::Transcribing
            )
        {
            self.show_dictation_popup();
        }
        if let Some(result) = result {
            self.hub
                .broadcast_dictation(&Event::DictationResult(result));
        }
        if models_changed || changed {
            if self.dictation.phase != DictationPhase::Listening {
                if let Some(control) = &self.control {
                    control.mute_for_dictation(false);
                }
            }
            self.dictation_changed();
        }
    }
}
