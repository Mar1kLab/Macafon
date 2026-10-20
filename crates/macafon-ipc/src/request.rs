//! Client requests; serialized method names are the public protocol.

use crate::{AbSide, ChainSettings, ChannelPatch, CloseAction, InputChannel, OsdPrefs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "method", content = "params")]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum Request {
    #[serde(rename = "output_controls.list")]
    OutputControlsList,
    #[serde(rename = "output_controls.set")]
    OutputControlsSet {
        id: String,
        settings: crate::MacaronSettings,
    },
    #[serde(rename = "hello")]
    Hello { protocol: u32, client: String },
    /// Stop the daemon: the virtual microphone disappears (BG-005).
    #[serde(rename = "daemon.quit")]
    DaemonQuit,
    /// "Start with system" (AS-001).
    #[serde(rename = "autostart.get")]
    AutostartGet,
    #[serde(rename = "autostart.set")]
    AutostartSet { enabled: bool },
    #[serde(rename = "mic.get")]
    MicGet,
    /// Replace the whole chain; continuous changes are smoothed, structural ones crossfaded.
    /// Mute and bypass stay as they are: they have their own requests, and a chain edit sent
    /// just after a mute hotkey must not reopen the microphone.
    #[serde(rename = "mic.set")]
    MicSet { settings: ChainSettings },
    #[serde(rename = "mic.set_preset")]
    MicSetPreset { preset: String },
    #[serde(rename = "mic.set_muted")]
    MicSetMuted { muted: bool },
    #[serde(rename = "mic.toggle_mute")]
    MicToggleMute,
    #[serde(rename = "mic.set_bypass")]
    MicSetBypass { bypass: bool },
    /// Physical microphone by PipeWire `node.name`; `null` disconnects.
    #[serde(rename = "mic.set_device")]
    MicSetDevice { device: Option<String> },
    /// Optional second-priority microphone; `null` disables fallback.
    #[serde(rename = "mic.set_fallback_device")]
    MicSetFallbackDevice { device: Option<String> },
    #[serde(rename = "mic.set_input_channel")]
    MicSetInputChannel { channel: InputChannel },
    #[serde(rename = "mic.set_default_source")]
    MicSetDefaultSource { confirmed: bool },
    #[serde(rename = "mic.restore_default_source")]
    MicRestoreDefaultSource,
    #[serde(rename = "ab.export")]
    AbExport { side: AbSide, path: String },
    #[serde(rename = "ab.outputs")]
    AbOutputs,
    #[serde(rename = "presets.list")]
    PresetsList,
    /// Save the current chain as a new user preset and select it.
    #[serde(rename = "presets.save")]
    PresetsSave { name: String },
    /// Overwrite a user preset with the current chain and select it.
    #[serde(rename = "presets.update")]
    PresetsUpdate { id: String },
    #[serde(rename = "presets.rename")]
    PresetsRename { id: String, name: String },
    /// Copy any preset (built-in or user) under a free name.
    #[serde(rename = "presets.duplicate")]
    PresetsDuplicate { id: String },
    #[serde(rename = "presets.delete")]
    PresetsDelete { id: String },
    /// The `.macafon-preset.json` document of a preset.
    #[serde(rename = "presets.export")]
    PresetsExport { id: String },
    /// Add a preset from a `.macafon-preset.json` document; a taken name gets a suffix.
    #[serde(rename = "presets.import")]
    PresetsImport { document: serde_json::Value },
    #[serde(rename = "devices.list")]
    DevicesList,
    #[serde(rename = "devices.subscribe")]
    DevicesSubscribe,
    #[serde(rename = "devices.unsubscribe")]
    DevicesUnsubscribe,
    /// Stream `meters` events at up to 60 Hz until unsubscribed (MTR-004, NFR-R7).
    #[serde(rename = "meters.subscribe")]
    MetersSubscribe { hz: u32 },
    #[serde(rename = "meters.unsubscribe")]
    MetersUnsubscribe,
    /// Record silence, then speech, and calibrate the chain (PRD §6.5). Progress and the
    /// result arrive as `calibration.changed` events.
    #[serde(rename = "calibration.start")]
    CalibrationStart,
    #[serde(rename = "calibration.cancel")]
    CalibrationCancel,
    #[serde(rename = "calibration.get")]
    CalibrationGet,
    /// AB-001: record the raw microphone (before the chain) for up to `max_seconds`
    /// (3–30, default 10). Stops playback; the new clip replaces the old one.
    #[serde(rename = "ab.record")]
    AbRecord {
        #[serde(default)]
        max_seconds: Option<u32>,
    },
    /// Finish the recording early (after at least 3 s).
    #[serde(rename = "ab.stop_recording")]
    AbStopRecording,
    /// AB-002: loop the clip into a sink `node.name`, or the default output.
    #[serde(rename = "ab.play")]
    AbPlay {
        #[serde(default)]
        device: Option<String>,
    },
    #[serde(rename = "ab.stop")]
    AbStop,
    /// AB-004: A = raw, B = processed by the current chain.
    #[serde(rename = "ab.select")]
    AbSelect { side: AbSide },
    #[serde(rename = "ab.toggle")]
    AbToggle,
    #[serde(rename = "ab.seek")]
    AbSeek { position_ms: u32 },
    /// AB-005: level A to B's loudness.
    #[serde(rename = "ab.set_loudness_match")]
    AbSetLoudnessMatch { enabled: bool },
    #[serde(rename = "ab.get")]
    AbGet,
    /// AB-006: peak envelopes of A and B with `points` values each (16–4096).
    #[serde(rename = "ab.waveform")]
    AbWaveform { points: u32 },
    #[serde(rename = "mixer.get")]
    MixerGet,
    /// CH-009: create or remove the channels; OUT-005 default output follows.
    #[serde(rename = "mixer.set_enabled")]
    MixerSetEnabled { enabled: bool },
    /// CH-006: Master fader and mute; absent fields stay.
    #[serde(rename = "mixer.set_master")]
    MixerSetMaster {
        #[serde(default)]
        volume_pct: Option<f32>,
        #[serde(default)]
        muted: Option<bool>,
    },
    /// OUT-001: play into `device` now; it moves to the top of the priority list.
    #[serde(rename = "mixer.set_output")]
    MixerSetOutput { device: Option<String> },
    /// OUT-002: the whole priority list, most preferred first.
    #[serde(rename = "mixer.set_output_priority")]
    MixerSetOutputPriority { devices: Vec<String> },
    /// APP-006: classify known apps on first sight.
    #[serde(rename = "mixer.set_auto_classify")]
    MixerSetAutoClassify { enabled: bool },
    /// CH-002: a new channel with a free id; returns the mixer state.
    #[serde(rename = "channel.create")]
    ChannelCreate {
        name: String,
        color: String,
        #[serde(default)]
        icon: Option<String>,
    },
    #[serde(rename = "channel.update")]
    ChannelUpdate { id: String, patch: ChannelPatch },
    /// CH-002: apps of the deleted channel move to the default one.
    #[serde(rename = "channel.delete")]
    ChannelDelete { id: String },
    /// The whole chain and fader of one channel (§7.3, CH-004).
    #[serde(rename = "channel.set")]
    ChannelSet {
        id: String,
        settings: macafon_dsp::ChannelSettings,
    },
    /// Relative fader change in percent points, for hotkeys and the CLI (HK-001, HK-004).
    #[serde(rename = "channel.adjust_volume")]
    ChannelAdjustVolume { id: String, delta_pct: f32 },
    #[serde(rename = "channel.toggle_mute")]
    ChannelToggleMute { id: String },
    /// CH-003.
    #[serde(rename = "channel.set_default")]
    ChannelSetDefault { id: String },
    /// CH-007: every channel id in the new order.
    #[serde(rename = "channel.reorder")]
    ChannelReorder { ids: Vec<String> },
    /// APP-004/005: move all streams of the app now and remember it.
    #[serde(rename = "apps.move")]
    AppsMove { app_key: String, channel: String },
    /// APP-009: `false` leaves the app where it plays and forgets nothing else.
    #[serde(rename = "apps.set_managed")]
    AppsSetManaged { app_key: String, managed: bool },
    /// Forget the rule of an app that is not running (APP-007).
    #[serde(rename = "apps.forget")]
    AppsForget { app_key: String },
    /// CH-008: the app's volume inside its channel; remembered like its channel.
    #[serde(rename = "apps.set_volume")]
    AppsSetVolume { app_key: String, volume_pct: f32 },
    /// APP-003: the image for an app's `icon` as a `data:` URL, or `null` if none is found.
    #[serde(rename = "apps.icon")]
    AppsIcon { icon: String },
    /// §9.6: run the self-checks now.
    #[serde(rename = "doctor.run")]
    DoctorRun,
    /// Apply the one-click fix of a check marked `fixable`.
    #[serde(rename = "doctor.fix")]
    DoctorFix { check: crate::DoctorCheckId },
    /// Load the next (`step = 1`) or previous (`-1`) preset (HK-001, HK-004).
    #[serde(rename = "mic.cycle_preset")]
    MicCyclePreset { step: i32 },
    #[serde(rename = "hotkeys.get")]
    HotkeysGet,
    /// HK-002: bind `shortcut` (`Ctrl+Alt+M`) to `action`, or clear it with `null`.
    #[serde(rename = "hotkeys.set")]
    HotkeysSet {
        action: String,
        shortcut: Option<String>,
        #[serde(default)]
        secondary: bool,
    },
    /// The main window closed while the UI process stays warm in the background: the daemon
    /// reacts as it did to the UI disconnecting (BG-001 hint, §9.5 quit on close).
    #[serde(rename = "ui.window_closed")]
    UiWindowClosed,
    /// The library, its sounds and the soundpad preferences (§7.6).
    #[serde(rename = "soundpad.get")]
    SoundpadGet,
    /// Look at the folder again now; it is also rescanned every few seconds.
    #[serde(rename = "soundpad.rescan")]
    SoundpadRescan,
    /// Open the library folder in the file manager.
    #[serde(rename = "soundpad.open_folder")]
    SoundpadOpenFolder,
    /// Use another library folder, on any drive; `null` goes back to the default one. A missing
    /// folder is created. Playback stops; settings and hotkeys of sounds stay by their ids.
    #[serde(rename = "soundpad.set_folder")]
    SoundpadSetFolder { folder: Option<String> },
    /// SND-002: play a sound from the start, replacing what plays.
    #[serde(rename = "soundpad.play")]
    SoundpadPlay { id: String },
    /// Play a random sound, of one category if given.
    #[serde(rename = "soundpad.play_random")]
    SoundpadPlayRandom {
        #[serde(default)]
        category: Option<String>,
    },
    #[serde(rename = "soundpad.stop")]
    SoundpadStop,
    /// `null` toggles.
    #[serde(rename = "soundpad.set_paused")]
    SoundpadSetPaused {
        #[serde(default)]
        paused: Option<bool>,
    },
    #[serde(rename = "soundpad.seek")]
    SoundpadSeek { position_ms: u32 },
    #[serde(rename = "soundpad.set_looping")]
    SoundpadSetLooping { looping: bool },
    #[serde(rename = "soundpad.set_prefs")]
    SoundpadSetPrefs { patch: crate::SoundpadPrefsPatch },
    /// Monitor output by device name; `null` follows the default output.
    #[serde(rename = "soundpad.set_monitor_device")]
    SoundpadSetMonitorDevice { device: Option<String> },
    #[serde(rename = "soundpad.set_sound")]
    SoundpadSetSound {
        id: String,
        patch: crate::SoundPatch,
    },
    /// New file name without extension.
    #[serde(rename = "soundpad.rename")]
    SoundpadRename { id: String, name: String },
    /// Move into a category; `null` is the folder itself.
    #[serde(rename = "soundpad.move")]
    SoundpadMove {
        id: String,
        category: Option<String>,
    },
    /// Delete the file from the library folder.
    #[serde(rename = "soundpad.delete")]
    SoundpadDelete { id: String },
    #[serde(rename = "soundpad.delete_problem_sounds")]
    SoundpadDeleteProblemSounds,
    #[serde(rename = "soundpad.create_category")]
    SoundpadCreateCategory { name: String },
    #[serde(rename = "soundpad.rename_category")]
    SoundpadRenameCategory { from: String, to: String },
    /// Its sounds move to the folder itself.
    #[serde(rename = "soundpad.delete_category")]
    SoundpadDeleteCategory { name: String },
    /// Add a file to the library in base64 chunks of up to `SOUNDPAD_UPLOAD_CHUNK_BYTES`
    /// decoded bytes:
    /// `offset` is the byte offset of `data`; `done` finishes the file and returns the
    /// state. A new upload starts with `offset` 0 under a client-chosen `upload_id`.
    #[serde(rename = "soundpad.upload")]
    SoundpadUpload {
        upload_id: String,
        file_name: String,
        #[serde(default)]
        category: Option<String>,
        offset: u32,
        data: String,
        done: bool,
    },
    /// §9.5 preferences.
    #[serde(rename = "prefs.get")]
    PrefsGet,
    /// Change the given preferences; `quantum_frames` is one of 64, 128, 256, 512.
    #[serde(rename = "prefs.set")]
    PrefsSet {
        #[serde(default)]
        volume_step_pct: Option<u8>,
        #[serde(default)]
        quantum_frames: Option<u32>,
        #[serde(default)]
        close_action: Option<CloseAction>,
        /// The whole popup setup; the daemon shows a sample popup with it.
        #[serde(default)]
        osd: Option<OsdPrefs>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        overlay: Option<crate::OverlayPrefs>,
        /// The monitor the main window opens on; empty: where the system puts it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        main_monitor: Option<String>,
    },
    /// Dictation (§8.4): state, models, history.
    #[serde(rename = "dictation.get")]
    DictationGet,
    #[serde(rename = "dictation.set_prefs")]
    DictationSetPrefs { patch: crate::DictationPrefsPatch },
    /// Start downloading a manifest model from Hugging Face (STT-013); progress arrives in
    /// `dictation.changed`.
    #[serde(rename = "dictation.download")]
    DictationDownload { model: String },
    #[serde(rename = "dictation.cancel_download")]
    DictationCancelDownload { model: String },
    /// Refused while the model is loaded; the worker is unloaded first.
    #[serde(rename = "dictation.delete_model")]
    DictationDeleteModel { model: String },
    /// Copy a local file in; accepted only if its SHA-256 matches a manifest file.
    #[serde(rename = "dictation.import_model")]
    DictationImportModel { path: String },
    /// STT-022: `null` resets to the default folder; downloaded models move along.
    #[serde(rename = "dictation.set_models_dir")]
    DictationSetModelsDir { folder: Option<String> },
    #[serde(rename = "dictation.cancel_move")]
    DictationCancelMove,
    /// TRY IT: record like push-to-talk, but return the text as a `test` result instead of
    /// inserting it.
    #[serde(rename = "dictation.test_start")]
    DictationTestStart,
    #[serde(rename = "dictation.test_stop")]
    DictationTestStop,
    /// Cancel a recording in progress (test or hotkey); nothing is inserted.
    #[serde(rename = "dictation.cancel")]
    DictationCancel,
    #[serde(rename = "dictation.history_clear")]
    DictationHistoryClear,
    /// Insert a history entry into the window that had focus before the UI.
    #[serde(rename = "dictation.reinsert")]
    DictationReinsert {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        id: u64,
    },
}
