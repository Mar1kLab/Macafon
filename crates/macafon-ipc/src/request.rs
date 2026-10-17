//! Client requests; serialized method names are the public protocol.

use crate::{AbSide, ChainSettings, ChannelPatch, CloseAction, InputChannel, OsdPrefs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "method", content = "params")]
#[allow(clippy::large_enum_variant)] // transient messages; boxing buys nothing
pub enum Request {
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
    },
    /// §9.5 preferences.
    #[serde(rename = "prefs.get")]
    PrefsGet,
    /// Change the given preferences; `quantum_frames` is one of 64, 128, 256, 512.
    #[serde(rename = "prefs.set")]
    PrefsSet {
        #[serde(default)]
        quantum_frames: Option<u32>,
        #[serde(default)]
        close_action: Option<CloseAction>,
        /// The whole popup setup; the daemon shows a sample popup with it.
        #[serde(default)]
        osd: Option<OsdPrefs>,
    },
}
