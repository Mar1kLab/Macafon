//! `miceq ctl …`: control the running daemon from scripts and desktop hotkeys.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use miceq_ipc::{
    AbSide, AbState, CalibrationReport, CalibrationState, CalibrationStep, Client, Device, Event,
    MicState, PresetInfo, Request,
};
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Control the micEQ daemon")]
struct Cli {
    #[command(subcommand)]
    command: Top,
}

#[derive(Subcommand)]
enum Top {
    /// Send a command to the running miceqd.
    Ctl {
        /// Print raw JSON replies.
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        command: Ctl,
    },
}

#[derive(Subcommand)]
enum Ctl {
    /// Show the microphone state.
    Status,
    /// List presets, or manage user presets.
    Presets {
        #[command(subcommand)]
        command: Option<PresetsCommand>,
    },
    /// Load a preset by id (keeps mute and bypass).
    Preset { id: String },
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
    /// Select the physical microphone by node.name; `none` disconnects.
    Device { name: String },
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
enum PresetsCommand {
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
    /// Write a .miceq-preset.json document (stdout without a file).
    Export {
        id: String,
        file: Option<PathBuf>,
    },
    /// Add a preset from a .miceq-preset.json file.
    Import {
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum AbCommand {
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
enum Side {
    A,
    B,
}

#[derive(Clone, Copy, ValueEnum)]
enum Switch {
    On,
    Off,
    Toggle,
}

fn main() -> Result<()> {
    let Top::Ctl { json, command } = Cli::parse().command;
    let path = miceq_ipc::socket_path().context("XDG_RUNTIME_DIR is not set")?;
    let (mut client, _) = Client::connect(&path, "miceq-cli")
        .with_context(|| format!("is miceqd running? ({})", path.display()))?;
    let request = match command {
        Ctl::Status => Request::MicGet,
        Ctl::Presets { command } => return presets(&mut client, command, json),
        Ctl::Devices => {
            let devices: Vec<Device> = client.call_as(Request::DevicesList)?;
            for device in devices {
                println!("{}\t{}", device.name, device.description);
            }
            return Ok(());
        }
        Ctl::Preset { id } => Request::MicSetPreset { preset: id },
        Ctl::Mute { state } => match state {
            Switch::On => Request::MicSetMuted { muted: true },
            Switch::Off => Request::MicSetMuted { muted: false },
            Switch::Toggle => Request::MicToggleMute,
        },
        Ctl::Bypass { state } => {
            let bypass = match state {
                Switch::On => true,
                Switch::Off => false,
                Switch::Toggle => {
                    let state: MicState = client.call_as(Request::MicGet)?;
                    !state.settings.bypass
                }
            };
            Request::MicSetBypass { bypass }
        }
        Ctl::Device { name } => Request::MicSetDevice {
            device: (name != "none").then_some(name),
        },
        Ctl::Gain { gain_db } => {
            let mut state: MicState = client.call_as(Request::MicGet)?;
            state.settings.gain.enabled = true;
            state.settings.gain.params.gain_db = gain_db;
            Request::MicSet {
                settings: state.settings,
            }
        }
        Ctl::Calibrate => return calibrate(&mut client, json),
        Ctl::Ab { command } => return ab(&mut client, command, json),
        Ctl::Meters { hz, count } => {
            client.call(Request::MetersSubscribe { hz })?;
            let mut printed = 0;
            while count.is_none_or(|count| printed < count) {
                if let Event::Meters(meters) = client.next_event()? {
                    println!("{}", serde_json::to_string(&meters)?);
                    printed += 1;
                }
            }
            return Ok(());
        }
    };
    let reply = client.call(request)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&reply)?);
        return Ok(());
    }
    let state: MicState = serde_json::from_value(reply)?;
    print_state(&state);
    Ok(())
}

fn calibrate(client: &mut Client, json: bool) -> Result<()> {
    let mut state: CalibrationState = client.call_as(Request::CalibrationStart)?;
    let mut step = None;
    loop {
        match &state {
            CalibrationState::Measuring {
                step: current,
                progress,
                ..
            } => {
                if !json && step != Some(*current) {
                    step = Some(*current);
                    match current {
                        CalibrationStep::Silence => println!("Stay silent for 4 seconds…"),
                        CalibrationStep::Speech => println!(
                            "\nRead aloud for 6 seconds: \"The quick brown fox jumps over the \
                             lazy dog, then naps in the warm afternoon sun.\""
                        ),
                    }
                }
                if !json {
                    print!("\r[{:<30}]", "#".repeat((progress * 30.0) as usize));
                    std::io::stdout().flush()?;
                }
            }
            CalibrationState::Done { report } => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&state)?);
                } else {
                    println!("\r[{}]\n", "#".repeat(30));
                    print_report(report);
                }
                return Ok(());
            }
            CalibrationState::Failed { message, .. } => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&state)?);
                }
                anyhow::bail!("calibration failed: {message}");
            }
            CalibrationState::Idle => anyhow::bail!("calibration was cancelled"),
        }
        loop {
            if let Event::CalibrationChanged(next) = client.next_event()? {
                state = next;
                break;
            }
        }
    }
}

fn presets(client: &mut Client, command: Option<PresetsCommand>, json: bool) -> Result<()> {
    let request = match command.unwrap_or(PresetsCommand::List) {
        PresetsCommand::List => {
            let presets: Vec<PresetInfo> = client.call_as(Request::PresetsList)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&presets)?);
                return Ok(());
            }
            for preset in presets {
                let kind = if preset.builtin { "built-in" } else { "user" };
                println!("{}\t{}\t{kind}", preset.id, preset.name);
            }
            return Ok(());
        }
        PresetsCommand::Save { name } => Request::PresetsSave { name },
        PresetsCommand::Update { id } => Request::PresetsUpdate { id },
        PresetsCommand::Rename { id, name } => Request::PresetsRename { id, name },
        PresetsCommand::Duplicate { id } => Request::PresetsDuplicate { id },
        PresetsCommand::Delete { id } => {
            client.call(Request::PresetsDelete { id })?;
            return Ok(());
        }
        PresetsCommand::Export { id, file } => {
            let document = client.call(Request::PresetsExport { id })?;
            let mut text = serde_json::to_string_pretty(&document)?;
            text.push('\n');
            match file {
                Some(file) => std::fs::write(&file, text)
                    .with_context(|| format!("cannot write {}", file.display()))?,
                None => print!("{text}"),
            }
            return Ok(());
        }
        PresetsCommand::Import { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let document = serde_json::from_str(&text)
                .with_context(|| format!("{} is not JSON", file.display()))?;
            Request::PresetsImport { document }
        }
    };
    let preset: PresetInfo = client.call_as(request)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&preset)?);
    } else {
        println!("{}\t{}", preset.id, preset.name);
    }
    Ok(())
}

fn ab(client: &mut Client, command: AbCommand, json: bool) -> Result<()> {
    let request = match command {
        AbCommand::Status => Request::AbGet,
        AbCommand::Record { seconds } => {
            let mut state: AbState = client.call_as(Request::AbRecord {
                max_seconds: Some(seconds),
            })?;
            if !json {
                println!("Recording {seconds} s: say something, then stay silent for 2 seconds…");
            }
            while state.recording.is_some() {
                if let Event::AbChanged(next) = client.next_event()? {
                    state = next;
                }
            }
            if state.clip_ms.is_none() {
                anyhow::bail!("the recording was interrupted");
            }
            return print_ab(&state, json);
        }
        AbCommand::Play { device } => Request::AbPlay { device },
        AbCommand::Stop => Request::AbStop,
        AbCommand::Select { side: None } => Request::AbToggle,
        AbCommand::Select { side: Some(side) } => Request::AbSelect {
            side: match side {
                Side::A => AbSide::A,
                Side::B => AbSide::B,
            },
        },
        AbCommand::Seek { position_ms } => Request::AbSeek { position_ms },
        AbCommand::Waveform { points } => {
            let waveform = client.call(Request::AbWaveform { points })?;
            println!("{}", serde_json::to_string(&waveform)?);
            return Ok(());
        }
        AbCommand::Match { state } => {
            let enabled = match state {
                Switch::On => true,
                Switch::Off => false,
                Switch::Toggle => !client.call_as::<AbState>(Request::AbGet)?.loudness_match,
            };
            Request::AbSetLoudnessMatch { enabled }
        }
    };
    let state: AbState = client.call_as(request)?;
    print_ab(&state, json)
}

fn print_ab(state: &AbState, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(state)?);
        return Ok(());
    }
    let clip = state.clip_ms.map_or("none".to_owned(), |ms| {
        format!("{:.1} s", ms as f32 / 1000.0)
    });
    println!("clip:     {clip}");
    println!(
        "playing:  {}{}",
        state.playing,
        state
            .device
            .as_deref()
            .map_or(String::new(), |device| format!(" → {device}"))
    );
    println!("side:     {:?}", state.side);
    println!("position: {:.1} s", state.position_ms as f32 / 1000.0);
    println!("match:    {}", state.loudness_match);
    Ok(())
}

fn print_report(report: &CalibrationReport) {
    println!("noise floor:  {:.1} dBFS", report.noise_floor_dbfs);
    println!(
        "speech:       {:.1} dBFS RMS, peak {:.1} dBFS",
        report.speech_rms_dbfs, report.speech_peak_dbfs
    );
    println!(
        "SNR:          {:.1} dB → {:.1} dB ({:?} noise)",
        report.snr_db, report.snr_after_db, report.noise_level
    );
    println!("gain:         {:+.1} dB", report.gain_db);
    let hum = if report.low_frequency_noise {
        " (low-frequency noise)"
    } else {
        ""
    };
    println!("high-pass:    {:.0} Hz{hum}", report.high_pass_freq_hz);
    println!("noise supp.:  {:.0}%", report.noise_intensity_pct);
    println!(
        "expander:     {:.0} dBFS threshold",
        report.expander_threshold_db
    );
    if report.suggest_studio {
        println!("tip:          the room is noisy; try Studio noise suppression");
    }
}

fn print_state(state: &MicState) {
    let latency_ms = state.latency_samples as f32 * 1000.0 / 48_000.0;
    let device = state.device.as_deref().unwrap_or("none");
    let input = match (state.device.is_some(), state.input_connected) {
        (false, _) => "",
        (true, true) => " (connected)",
        (true, false) => " (disconnected)",
    };
    let modified = if state.preset_modified {
        " (modified)"
    } else {
        ""
    };
    println!(
        "preset:   {}{modified}",
        state.preset.as_deref().unwrap_or("custom")
    );
    println!("input:    {device}{input}");
    println!("muted:    {}", state.settings.muted);
    println!("bypass:   {}", state.settings.bypass);
    println!("gain:     {:+.1} dB", state.settings.gain.params.gain_db);
    println!("latency:  {latency_ms:.2} ms");
}
