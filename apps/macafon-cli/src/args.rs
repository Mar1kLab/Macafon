//! Command-line syntax, independent of transport and output.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Open Macafon, or control the daemon with `ctl`")]
pub(super) struct Cli {
    /// Without a command, open the Macafon window.
    #[command(subcommand)]
    pub(super) command: Option<Top>,
}

#[derive(Subcommand)]
pub(super) enum Top {
    /// Run the daemon next to this binary (or on PATH), forwarding all following arguments.
    #[command(disable_help_flag = true)]
    Daemon {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
    },
    /// Restore the previous system microphone and output, including without a running daemon.
    RestoreDefault,
    /// Open the quick panel with every volume and the microphone switches, or close it.
    QuickPanel,
    /// Send a command to the running macafond.
    Ctl {
        /// Print raw JSON replies.
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        command: Ctl,
    },
}

#[derive(Subcommand)]
pub(super) enum Ctl {
    /// Show the microphone state.
    Status,
    /// Stop the daemon; the virtual microphone disappears.
    Quit,
    /// Start the daemon with the desktop session; shows the state without an argument.
    Autostart {
        #[arg(value_enum)]
        state: Option<Switch>,
    },
    /// List presets, or manage user presets.
    Presets {
        #[command(subcommand)]
        command: Option<PresetsCommand>,
    },
    /// Load a preset by id, or `next` / `previous` (keeps mute and bypass).
    Preset { id: String },
    /// Microphone commands as written in desktop shortcuts: `ctl mic mute toggle` (HK-004).
    Mic {
        #[command(subcommand)]
        command: MicCommand,
    },
    /// List hotkeys, or bind one: `ctl hotkeys set mic_mute_toggle Ctrl+Alt+M` (`none` clears).
    Hotkeys {
        #[command(subcommand)]
        command: Option<HotkeysCommand>,
    },
    /// Mute the virtual microphone (digital silence).
    Mute {
        #[arg(value_enum, default_value_t = Switch::Toggle)]
        state: Switch,
    },
    /// Bypass all processing.
    Bypass {
        #[arg(value_enum)]
        state: Switch,
    },
    /// List physical microphones.
    Devices,
    /// Make Macafon the system microphone; requires explicit --confirm.
    SetDefault {
        #[arg(long)]
        confirm: bool,
    },
    /// Restore the previous system microphone.
    RestoreDefault,
    /// Select the physical microphone by node.name; `none` disconnects.
    Device { name: String },
    /// Choose the physical input channel; mix combines stereo input.
    InputChannel {
        #[arg(value_enum)]
        channel: Channel,
    },
    /// Select the fallback microphone by node.name; `none` disables fallback.
    FallbackDevice { name: String },
    /// Set the input gain in dB.
    Gain {
        #[arg(allow_hyphen_values = true)]
        gain_db: f32,
    },
    /// Measure silence, then speech, and tune Gain, High-pass, NS and Expander.
    Calibrate,
    /// A/B test: record the raw microphone, loop it raw (A) or processed (B).
    Ab {
        #[command(subcommand)]
        command: AbCommand,
    },
    /// Show the mixer, or turn it on/off (CH-009).
    Mixer {
        #[arg(value_enum)]
        state: Option<Switch>,
    },
    /// Change one output channel by id (main, voice, music, …); for desktop hotkeys (HK-004).
    Channel {
        id: String,
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// List applications, or move one to a channel (APP-004).
    Apps {
        #[command(subcommand)]
        command: Option<AppsCommand>,
    },
    /// Play the channels into this output (node.name); `default` follows the previous system
    /// default (OUT-001, OUT-003).
    Output { name: String },
    /// Run the self-checks (§9.6) and print them.
    Doctor,
    /// Print meter frames (dBFS) as JSON lines.
    Meters {
        #[arg(long, default_value_t = 30)]
        hz: u32,
        /// Stop after this many frames.
        #[arg(long)]
        count: Option<usize>,
    },
}

#[derive(Subcommand)]
pub(super) enum MicCommand {
    Mute {
        #[arg(value_enum, default_value_t = Switch::Toggle)]
        state: Switch,
    },
    Bypass {
        #[arg(value_enum, default_value_t = Switch::Toggle)]
        state: Switch,
    },
    Preset {
        id: String,
    },
}

#[derive(Subcommand)]
pub(super) enum HotkeysCommand {
    List,
    Set { action: String, shortcut: String },
}

#[derive(Subcommand)]
pub(super) enum ChannelAction {
    /// `+5` / `-5` change by percent points; `40` sets 40%.
    Volume {
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    Mute {
        #[arg(value_enum, default_value_t = Switch::Toggle)]
        state: Switch,
    },
    /// Make it the channel new apps start in (CH-003).
    Default,
}

#[derive(Subcommand)]
pub(super) enum AppsCommand {
    List,
    /// Move every stream of the app now and remember it.
    Move {
        key: String,
        channel: String,
    },
}

#[derive(Subcommand)]
pub(super) enum PresetsCommand {
    List,
    /// Save the current chain as a new preset.
    Save {
        name: String,
    },
    /// Overwrite a user preset with the current chain.
    Update {
        id: String,
    },
    Rename {
        id: String,
        name: String,
    },
    Duplicate {
        id: String,
    },
    Delete {
        id: String,
    },
    /// Write a .macafon-preset.json document (stdout without a file).
    Export {
        id: String,
        file: Option<PathBuf>,
    },
    /// Add a preset from a .macafon-preset.json file.
    Import {
        file: PathBuf,
    },
}

#[derive(Subcommand)]
pub(super) enum AbCommand {
    /// List playback devices and speaker warnings.
    Outputs,
    /// Export raw A or processed B as a mono float WAV (never overwrites).
    Export {
        #[arg(value_enum)]
        side: Side,
        file: PathBuf,
    },
    /// Show the A/B state.
    Status,
    /// Record the raw microphone and wait until done.
    Record {
        #[arg(long, default_value_t = 10)]
        seconds: u32,
    },
    /// Loop the clip into a sink node.name, or the default output.
    Play {
        #[arg(long)]
        device: Option<String>,
    },
    Stop,
    /// Listen to A (raw) or B (processed); toggles without an argument.
    Select {
        #[arg(value_enum)]
        side: Option<Side>,
    },
    /// Jump to a position in the clip.
    Seek {
        position_ms: u32,
    },
    /// Print peak envelopes of A and B as JSON.
    Waveform {
        #[arg(long, default_value_t = 100)]
        points: u32,
    },
    /// Match A's loudness to B.
    Match {
        #[arg(value_enum)]
        state: Switch,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Channel {
    Auto,
    Left,
    Right,
    Mix,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Side {
    A,
    B,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Switch {
    On,
    Off,
    Toggle,
}
