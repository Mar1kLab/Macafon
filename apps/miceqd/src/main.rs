mod ab;
mod calibration;
mod host;
mod server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use host::DaemonHost;
use miceq_config::{Config, MicConfig, Store};
use miceq_dsp::settings::NoiseEngine;
use miceq_dsp::{Preset, SAMPLE_RATE_HZ};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[derive(Parser)]
#[command(version, about = "micEQ audio daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon: virtual microphone and control socket. System defaults are untouched.
    Run(RunArgs),
    /// List physical inputs, sinks and playback streams (read-only).
    Devices,
    /// M0 experiment: move one playback stream through WirePlumber metadata.
    Route { stream_id: u32, sink_name: String },
}

#[derive(clap::Args, Default)]
struct RunArgs {
    /// Config file [default: $XDG_CONFIG_HOME/miceq/config.toml].
    #[arg(long)]
    config: Option<PathBuf>,
    /// Physical PipeWire node.name (see `devices`); overrides the config.
    #[arg(long)]
    input: Option<String>,
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
        if let Some(input) = &self.input {
            mic.device = Some(input.clone());
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
    let mut config = match &store {
        Some(store) => {
            let loaded = store.load()?;
            if let Some(recovered) = loaded.recovered {
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
    if settings.noise_suppression.enabled
        && !settings.bypass
        && settings.noise_suppression.params.engine == NoiseEngine::Studio
    {
        tracing::warn!("Studio noise suppression is not implemented yet; using Lite");
    }
    tracing::info!(
        preset = config.mic.preset.as_deref().unwrap_or("custom"),
        input = config.mic.device.as_deref().unwrap_or("none"),
        bypass = settings.bypass,
        muted = settings.muted,
        latency_ms = settings.latency_samples() as f32 * 1000.0 / SAMPLE_RATE_HZ as f32,
        "Starting microphone chain"
    );
    let (control, receiver) = miceq_backend_pipewire::control_channel();
    let socket = miceq_ipc::socket_path().context("XDG_RUNTIME_DIR is not set")?;
    let host = Rc::new(RefCell::new(DaemonHost::new(
        config, store, socket, control,
    )));
    let result = miceq_backend_pipewire::run_microphone(host.clone(), receiver)
        .context("Cannot run microphone; check PipeWire and WirePlumber");
    host.borrow_mut().flush();
    result
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run(RunArgs::default())) {
        Command::Run(args) => run(args)?,
        Command::Devices => {
            for node in miceq_backend_pipewire::list_nodes()? {
                if matches!(
                    node.media_class.as_str(),
                    "Audio/Source" | "Audio/Sink" | "Stream/Output/Audio"
                ) && !node.name.starts_with("miceq_")
                    && !node.name.ends_with(".monitor")
                {
                    println!(
                        "{}\t{}\t{}\t{}",
                        node.id, node.media_class, node.name, node.description
                    );
                }
            }
        }
        Command::Route {
            stream_id,
            sink_name,
        } => {
            miceq_backend_pipewire::route_stream(stream_id, &sink_name)?;
            tracing::info!(stream_id, sink = %sink_name, "Stream moved; PipeWire links confirmed");
        }
    }
    Ok(())
}
