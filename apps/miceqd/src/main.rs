use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use miceq_dsp::{INTENSITY, MicrophoneProcessor, SAMPLE_RATE_HZ, VOICE_THRESHOLD};

#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

#[derive(Parser)]
#[command(version, about = "micEQ Linux M0 audio prototype")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the virtual microphone without changing system defaults.
    Run {
        /// Physical PipeWire node.name (see `devices`). Omit for manual pw-link wiring.
        #[arg(long)]
        input: Option<String>,
        #[arg(long, value_enum, default_value_t = Engine::Raw)]
        engine: Engine,
        #[arg(long, default_value_t = INTENSITY.default)]
        intensity: f32,
        #[arg(long, default_value_t = VOICE_THRESHOLD.default)]
        voice_threshold: f32,
        #[arg(long)]
        mute: bool,
    },
    /// List physical inputs, sinks and playback streams (read-only).
    Devices,
    /// M0 experiment: move one playback stream through WirePlumber metadata.
    Route { stream_id: u32, sink_name: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum Engine {
    Raw,
    Lite,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run {
        input: None,
        engine: Engine::Raw,
        intensity: INTENSITY.default,
        voice_threshold: VOICE_THRESHOLD.default,
        mute: false,
    }) {
        Command::Run {
            input,
            engine,
            intensity,
            voice_threshold,
            mute,
        } => {
            let prepare = || match engine {
                Engine::Raw => Ok(MicrophoneProcessor::raw(mute)),
                Engine::Lite => MicrophoneProcessor::lite(intensity, voice_threshold, mute),
            };
            let processor = prepare()?;
            tracing::info!(
                latency_samples = processor.latency_samples(),
                latency_ms = processor.latency_samples() as f32 * 1000.0 / SAMPLE_RATE_HZ as f32,
                "Starting M0 processing path"
            );
            drop(processor);
            miceq_backend_pipewire::run_microphone(prepare, input)
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
