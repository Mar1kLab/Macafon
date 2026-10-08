use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use miceq_dsp::settings::NoiseEngine;
use miceq_dsp::{ChainSettings, MicrophoneProcessor, Preset, SAMPLE_RATE_HZ};

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[derive(Parser)]
#[command(version, about = "micEQ Linux audio daemon prototype")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the virtual microphone without changing system defaults.
    Run(RunArgs),
    /// List physical inputs, sinks and playback streams (read-only).
    Devices,
    /// M0 experiment: move one playback stream through WirePlumber metadata.
    Route { stream_id: u32, sink_name: String },
}

#[derive(clap::Args, Default)]
struct RunArgs {
    /// Physical PipeWire node.name (see `devices`). Omit for manual pw-link wiring.
    #[arg(long)]
    input: Option<String>,
    /// Built-in microphone preset.
    #[arg(long, value_enum, default_value_t = PresetArg::Natural)]
    preset: PresetArg,
    /// Override the preset's noise suppression: `raw` disables it.
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

#[derive(Clone, Copy, Default, ValueEnum)]
enum PresetArg {
    #[default]
    Natural,
    DiscordClear,
    NoisyRoom,
    Broadcast,
    WarmPodcast,
    Raw,
}

impl From<PresetArg> for Preset {
    fn from(preset: PresetArg) -> Self {
        match preset {
            PresetArg::Natural => Self::Natural,
            PresetArg::DiscordClear => Self::DiscordClear,
            PresetArg::NoisyRoom => Self::NoisyRoom,
            PresetArg::Broadcast => Self::Broadcast,
            PresetArg::WarmPodcast => Self::WarmPodcast,
            PresetArg::Raw => Self::Raw,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Engine {
    Raw,
    Lite,
}

impl RunArgs {
    fn settings(&self) -> ChainSettings {
        let preset = Preset::from(self.preset);
        let mut settings = preset.settings();
        let ns = &mut settings.noise_suppression;
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
            settings.gain.params = gain;
        }
        settings.muted = self.mute;
        settings.bypass = self.bypass;
        settings
    }
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
        Command::Run(args) => {
            let settings = args.settings();
            let processor = MicrophoneProcessor::new(&settings)?;
            if settings.noise_suppression.enabled
                && !settings.bypass
                && settings.noise_suppression.params.engine == NoiseEngine::Studio
            {
                tracing::warn!("Studio noise suppression is not implemented yet; using Lite");
            }
            tracing::info!(
                preset = Preset::from(args.preset).name(),
                bypass = settings.bypass,
                muted = settings.muted,
                latency_samples = processor.latency_samples(),
                latency_ms = processor.latency_samples() as f32 * 1000.0 / SAMPLE_RATE_HZ as f32,
                "Starting microphone chain"
            );
            drop(processor);
            miceq_backend_pipewire::run_microphone(
                || MicrophoneProcessor::new(&settings),
                args.input,
            )
            .context("Cannot run microphone; check PipeWire and WirePlumber")?;
        }
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
