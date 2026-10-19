//! Persisted dictation preferences. Audio and history are never persisted.
use macafon_ipc::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationConfig {
    pub enabled: bool,
    pub engine: DictationEngine,
    pub model: Option<String>,
    pub language: DictationLanguagePolicy,
    pub fixed_language: DictationLanguage,
    pub tap: DictationTap,
    pub mute_mic_while_dictating: bool,
    pub insert: DictationInsert,
    pub trailing_space: bool,
    pub fullscreen_games: DictationGamePolicy,
    pub unload_after_s: i32,
    pub threads: u16,
    pub game_mode: bool,
    pub pre_roll: bool,
    pub keep_history: bool,
    pub vocabulary: Vec<String>,
    pub show_in_overlay: bool,
    pub models_dir: String,
    pub popup: DictationPopupConfig,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationPopupConfig {
    pub look: DictationPopupLook,
    pub position: DictationPopupPosition,
    pub offset_pct: f32,
}
impl Default for DictationPopupConfig {
    fn default() -> Self {
        Self {
            look: DictationPopupLook::Flow,
            position: DictationPopupPosition::Bottom,
            offset_pct: 0.0,
        }
    }
}
impl Default for DictationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: DictationEngine::Whisper,
            model: Some("whisper-large-v3-turbo-q5_0".into()),
            language: DictationLanguagePolicy::Layout,
            fixed_language: DictationLanguage::Uk,
            tap: DictationTap::Clean,
            mute_mic_while_dictating: true,
            insert: DictationInsert::Paste,
            trailing_space: true,
            fullscreen_games: DictationGamePolicy::Copy,
            unload_after_s: 180,
            threads: 0,
            game_mode: true,
            pre_roll: true,
            keep_history: true,
            vocabulary: vec!["Macafon".into(), "PipeWire".into()],
            show_in_overlay: true,
            models_dir: String::new(),
            popup: DictationPopupConfig::default(),
        }
    }
}
impl DictationConfig {
    pub fn prefs(&self) -> DictationPrefs {
        DictationPrefs {
            enabled: self.enabled,
            engine: self.engine,
            model: self.model.clone(),
            language: self.language,
            fixed_language: self.fixed_language,
            tap: self.tap,
            mute_mic_while_dictating: self.mute_mic_while_dictating,
            insert: self.insert,
            trailing_space: self.trailing_space,
            fullscreen_games: self.fullscreen_games,
            unload_after_s: self.unload_after_s,
            threads: self.threads,
            game_mode: self.game_mode,
            pre_roll: self.pre_roll,
            keep_history: self.keep_history,
            vocabulary: self.vocabulary.clone(),
            show_in_overlay: self.show_in_overlay,
            popup_look: self.popup.look,
            popup_position: self.popup.position,
            popup_offset_pct: self.popup.offset_pct,
        }
    }
    pub fn set_prefs(&mut self, prefs: DictationPrefs) {
        self.enabled = prefs.enabled;
        self.engine = prefs.engine;
        self.model = prefs.model;
        self.language = prefs.language;
        self.fixed_language = prefs.fixed_language;
        self.tap = prefs.tap;
        self.mute_mic_while_dictating = prefs.mute_mic_while_dictating;
        self.insert = prefs.insert;
        self.trailing_space = prefs.trailing_space;
        self.fullscreen_games = prefs.fullscreen_games;
        self.unload_after_s = prefs.unload_after_s;
        self.threads = prefs.threads;
        self.game_mode = prefs.game_mode;
        self.pre_roll = prefs.pre_roll;
        self.keep_history = prefs.keep_history;
        self.vocabulary = prefs.vocabulary;
        self.show_in_overlay = prefs.show_in_overlay;
        self.popup = DictationPopupConfig {
            look: prefs.popup_look,
            position: prefs.popup_position,
            offset_pct: prefs.popup_offset_pct,
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dictation_round_trip_and_ipc_mapping() {
        let mut config = crate::Config::default();
        config.dictation.models_dir = "D:/Speech models".into();
        config.dictation.popup.look = DictationPopupLook::Analog;
        let serialized = toml::to_string(&config).expect("serialize");
        assert!(serialized.contains("[dictation.popup]"));
        let decoded: crate::Config = toml::from_str(&serialized).expect("round trip");
        assert_eq!(decoded.dictation, config.dictation);
        let prefs = decoded.dictation.prefs();
        config.dictation.set_prefs(prefs.clone());
        assert_eq!(config.dictation.prefs(), prefs);
        assert_eq!(config.dictation.models_dir, "D:/Speech models");
        assert!(!prefs.enabled);
        assert_eq!(prefs.unload_after_s, 180);
    }
}
