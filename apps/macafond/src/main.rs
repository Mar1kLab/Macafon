#![cfg_attr(windows, windows_subsystem = "windows")]

mod ab;
#[cfg(target_os = "linux")]
mod autostart;
#[cfg(windows)]
#[path = "autostart_windows.rs"]
mod autostart;
mod calibration;
mod doctor;
mod host;
mod hotkeys;
mod logging;
mod mixer;
mod notify;
mod osd;
mod output_controls;
#[cfg_attr(target_os = "linux", path = "platform/linux.rs")]
#[cfg_attr(windows, path = "platform/windows.rs")]
mod platform;
mod server;
mod soundpad;
mod tray;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use host::DaemonHost;
use macafon_config::presets::PresetStore;
use macafon_config::{Config, MicConfig, Store};
use macafon_dsp::settings::NoiseEngine;
use macafon_dsp::{Preset, SAMPLE_RATE_HZ};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[derive(Parser)]
#[command(version, about = "Macafon audio daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon: virtual microphone and control socket. System defaults are untouched.
    Run(RunArgs),
    /// List physical inputs and outputs (read-only).
    Devices,
    /// Uninstall: stop a running daemon, give back system defaults and app volumes, and turn
    /// off autostart.
    #[cfg(windows)]
    Cleanup,
    /// M0 experiment: move one playback stream through WirePlumber metadata.
    #[cfg(target_os = "linux")]
    Route { stream_id: u32, sink_name: String },
}

#[derive(clap::Args, Default)]
struct RunArgs {
    /// Duplicate mono into left/right output channels; use --stereo=false for mono.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    stereo: Option<bool>,
    /// Config file [default: $XDG_CONFIG_HOME/macafon/config.toml, %APPDATA%\Macafon\config.toml].
    #[arg(long)]
    config: Option<PathBuf>,
    /// Physical microphone: PipeWire node.name or Windows endpoint ID (see `devices`);
    /// overrides the config.
    #[arg(long)]
    input: Option<String>,
    /// Second-priority physical microphone; overrides the config.
    #[arg(long)]
    fallback_input: Option<String>,
    /// Built-in preset: natural, discord-clear, noisy-room, broadcast, warm-podcast, raw.
    #[arg(long, value_parser = parse_preset)]
    preset: Option<Preset>,
    /// Override noise suppression: `raw` disables it.
    #[arg(long, value_enum)]
    engine: Option<Engine>,
    /// Noise suppression intensity override, percent.
    #[arg(long)]
    intensity: Option<f32>,
    /// Voice activity threshold override, percent.
    #[arg(long)]
    voice_threshold: Option<f32>,
    /// Input gain override, dB.
    #[arg(long, allow_hyphen_values = true)]
    gain: Option<f32>,
    #[arg(long)]
    mute: bool,
    /// Pass the microphone through unprocessed (global bypass).
    #[arg(long)]
    bypass: bool,
    /// Do not show the tray icon (tests, headless sessions).
    #[arg(long)]
    no_tray: bool,
}

fn parse_preset(id: &str) -> Result<Preset, String> {
    Preset::from_id(id).ok_or_else(|| {
        let ids: Vec<_> = Preset::ALL.iter().map(|preset| preset.id()).collect();
        format!("expected one of: {}", ids.join(", "))
    })
}

#[derive(Clone, Copy, ValueEnum)]
enum Engine {
    Raw,
    Lite,
}

impl RunArgs {
    /// Command-line options override the loaded config for this run.
    fn apply(&self, mic: &mut MicConfig) {
        if let Some(stereo) = self.stereo {
            mic.stereo = stereo;
        }
        if let Some(input) = &self.input {
            if mic.device.as_ref() != Some(input) {
                mic.noise_floor_dbfs = None;
                mic.noise_floor_device = None;
            }
            mic.device = Some(input.clone());
        }
        if let Some(input) = &self.fallback_input {
            mic.fallback_device = Some(input.clone());
        }
        if let Some(preset) = self.preset {
            mic.chain = preset.settings();
            mic.preset = Some(preset.id().to_owned());
        }
        let ns = &mut mic.chain.noise_suppression;
        match self.engine {
            Some(Engine::Raw) => ns.enabled = false,
            Some(Engine::Lite) => {
                ns.enabled = true;
                ns.params.engine = NoiseEngine::Lite;
            }
            None => {}
        }
        if let Some(intensity) = self.intensity {
            ns.params.intensity_pct = intensity;
        }
        if let Some(threshold) = self.voice_threshold {
            ns.params.voice_threshold_pct = threshold;
        }
        if let Some(gain) = self.gain {
            mic.chain.gain.params.gain_db = gain;
        }
        mic.chain.muted |= self.mute;
        mic.chain.bypass |= self.bypass;
    }
}

fn run(args: RunArgs) -> Result<()> {
    let store = match &args.config {
        Some(path) => Some(Store::new(path)),
        None => Store::default_location()
            .inspect_err(|error| tracing::warn!(%error, "Settings will not be saved"))
            .ok(),
    };
    let mut config_recovery = None;
    let mut config = match &store {
        Some(store) => {
            let loaded = store.load()?;
            if let Some(recovered) = loaded.recovered {
                config_recovery = Some(macafon_ipc::ConfigRecovery {
                    backup: recovered.backup.display().to_string(),
                    reason: recovered.reason.clone(),
                });
                tracing::warn!(
                    reason = recovered.reason,
                    backup = %recovered.backup.display(),
                    "Config was invalid; using defaults"
                );
            }
            loaded.config
        }
        None => Config::default(),
    };
    args.apply(&mut config.mic);
    let settings = &config.mic.chain;
    settings.validate()?;
    tracing::info!(
        preset = config.mic.preset.as_deref().unwrap_or("custom"),
        input = config.mic.device.as_deref().unwrap_or("none"),
        bypass = settings.bypass,
        muted = settings.muted,
        latency_ms = settings.latency_samples() as f32 * 1000.0 / SAMPLE_RATE_HZ as f32,
        "Starting microphone chain"
    );
    // User presets live next to the config file: `<config dir>/presets/mic`.
    let presets = match store
        .as_ref()
        .and_then(|store| store.path().parent())
        .map(|directory| directory.join("presets").join("mic"))
    {
        Some(directory) => {
            let (presets, skipped) = PresetStore::open(directory);
            for (path, reason) in skipped {
                tracing::warn!(path = %path.display(), %reason, "Skipping preset");
            }
            presets
        }
        None => PresetStore::in_memory(),
    };
    let (control, receiver) = platform::control_channel();
    let endpoint = platform::endpoint()?;
    let host = Rc::new(RefCell::new(DaemonHost::new(
        config,
        store,
        presets,
        endpoint,
        control,
        !args.no_tray,
    )));
    host.borrow_mut().set_config_recovery(config_recovery);
    let flushing = host.clone();
    platform::run(host, receiver, move || flushing.borrow_mut().flush())
}

fn main() -> Result<()> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // SAFETY: attaches only to an existing parent console, never creates a window.
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
    let cli = Cli::parse();
    logging::init();
    match cli.command.unwrap_or(Command::Run(RunArgs::default())) {
        Command::Run(args) => run(args)?,
        Command::Devices => platform::print_devices()?,
        #[cfg(windows)]
        Command::Cleanup => platform::cleanup()?,
        #[cfg(target_os = "linux")]
        Command::Route {
            stream_id,
            sink_name,
        } => platform::route(stream_id, &sink_name)?,
    }
    Ok(())
}
