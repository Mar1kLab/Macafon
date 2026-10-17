//! Control commands over the daemon client.

use crate::args::*;
use crate::output::{print_ab, print_mixer, print_report, print_state};
use anyhow::{Context, Result};
use macafon_ipc::{
    AbSide, AbState, AutostartState, CalibrationState, CalibrationStep, Client, Device, Event,
    InputChannel, MicState, MixerState, PresetInfo, Request,
};
use std::io::Write;

pub(super) fn run(json: bool, command: Ctl) -> Result<()> {
    let endpoint = macafon_ipc::Endpoint::for_current_user()
        .context("cannot locate the daemon endpoint (XDG_RUNTIME_DIR is not set)")?;
    let (mut client, _) = Client::connect(&endpoint, "macafon-cli")
        .with_context(|| format!("is macafond running? ({endpoint})"))?;
    let request = match command {
        Ctl::Status => Request::MicGet,
        Ctl::SetDefault { confirm } => {
            if !confirm {
                anyhow::bail!(
                    "This changes the system microphone to Macafon. Pass --confirm to proceed; the previous default is restored on Quit."
                );
            }
            client.call(Request::MicSetDefaultSource { confirmed: true })?;
            return Ok(());
        }
        Ctl::RestoreDefault => {
            client.call(Request::MicRestoreDefaultSource)?;
            return Ok(());
        }
        Ctl::Autostart { state } => {
            let request = match state {
                None => Request::AutostartGet,
                Some(Switch::On) => Request::AutostartSet { enabled: true },
                Some(Switch::Off) => Request::AutostartSet { enabled: false },
                Some(Switch::Toggle) => {
                    let current: AutostartState = client.call_as(Request::AutostartGet)?;
                    Request::AutostartSet {
                        enabled: !current.enabled,
                    }
                }
            };
            let state: AutostartState = client.call_as(request)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                println!("autostart: {} ({:?})", state.enabled, state.method);
            }
            return Ok(());
        }
        Ctl::Quit => {
            return match client.call(Request::DaemonQuit) {
                // The daemon may exit before its reply is written.
                Ok(_) | Err(macafon_ipc::ClientError::Closed | macafon_ipc::ClientError::Io(_)) => {
                    Ok(())
                }
                Err(error) => Err(error.into()),
            };
        }
        Ctl::Presets { command } => return presets(&mut client, command, json),
        Ctl::Mixer { state } => {
            let request = match state {
                None => Request::MixerGet,
                Some(Switch::On) => Request::MixerSetEnabled { enabled: true },
                Some(Switch::Off) => Request::MixerSetEnabled { enabled: false },
                Some(Switch::Toggle) => {
                    let current: MixerState = client.call_as(Request::MixerGet)?;
                    Request::MixerSetEnabled {
                        enabled: !current.enabled,
                    }
                }
            };
            return print_mixer(&client.call_as(request)?, json);
        }
        Ctl::Channel { id, action } if id == "master" => {
            let state: MixerState = client.call_as(Request::MixerGet)?;
            let request = match action {
                ChannelAction::Volume { value } => {
                    let parsed: f32 = value
                        .trim_start_matches('+')
                        .parse()
                        .with_context(|| format!("not a volume: {value}"))?;
                    let volume = if value.starts_with(['+', '-']) {
                        state.master_volume_pct + parsed
                    } else {
                        parsed
                    };
                    Request::MixerSetMaster {
                        volume_pct: Some(volume.clamp(0.0, 100.0)),
                        muted: None,
                    }
                }
                ChannelAction::Mute { state: switch } => Request::MixerSetMaster {
                    volume_pct: None,
                    muted: Some(match switch {
                        Switch::On => true,
                        Switch::Off => false,
                        Switch::Toggle => !state.master_muted,
                    }),
                },
                ChannelAction::Default => {
                    anyhow::bail!("Master cannot be the default application channel")
                }
            };
            return print_mixer(&client.call_as(request)?, json);
        }
        Ctl::Channel { id, action } => {
            let request = match action {
                ChannelAction::Volume { value } => {
                    let number: f32 = value
                        .trim_start_matches('+')
                        .parse()
                        .with_context(|| format!("not a volume: {value}"))?;
                    if value.starts_with(['+', '-']) {
                        Request::ChannelAdjustVolume {
                            id,
                            delta_pct: number,
                        }
                    } else {
                        let state: MixerState = client.call_as(Request::MixerGet)?;
                        let current = state
                            .channels
                            .iter()
                            .find(|channel| channel.id == id)
                            .with_context(|| format!("no channel {id:?}"))?
                            .settings
                            .volume_pct;
                        Request::ChannelAdjustVolume {
                            id,
                            delta_pct: number - current,
                        }
                    }
                }
                ChannelAction::Mute { state } => {
                    let current: MixerState = client.call_as(Request::MixerGet)?;
                    let muted = current
                        .channels
                        .iter()
                        .find(|channel| channel.id == id)
                        .with_context(|| format!("no channel {id:?}"))?
                        .settings
                        .muted;
                    match state {
                        Switch::On if muted => return Ok(()),
                        Switch::Off if !muted => return Ok(()),
                        _ => Request::ChannelToggleMute { id },
                    }
                }
                ChannelAction::Default => Request::ChannelSetDefault { id },
            };
            return print_mixer(&client.call_as(request)?, json);
        }
        Ctl::Apps { command } => {
            let request = match command {
                None | Some(AppsCommand::List) => Request::MixerGet,
                Some(AppsCommand::Move { key, channel }) => Request::AppsMove {
                    app_key: key,
                    channel,
                },
            };
            let state: MixerState = client.call_as(request)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state.apps)?);
            } else {
                for app in &state.apps {
                    println!(
                        "{}\t{}\t{}{}",
                        app.key,
                        app.channel,
                        app.name,
                        if app.active { "\t(playing)" } else { "" }
                    );
                }
            }
            return Ok(());
        }
        Ctl::Doctor => {
            let report: macafon_ipc::DoctorReport = client.call_as(Request::DoctorRun)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for check in &report.checks {
                    println!(
                        "{:<8} {:?}{}",
                        format!("{:?}", check.status).to_lowercase(),
                        check.id,
                        if check.values.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", check.values.join(", "))
                        }
                    );
                }
            }
            return Ok(());
        }
        Ctl::Output { name } => {
            let request = Request::MixerSetOutput {
                device: (name != "default").then_some(name),
            };
            return print_mixer(&client.call_as(request)?, json);
        }
        Ctl::Devices => {
            let devices: Vec<Device> = client.call_as(Request::DevicesList)?;
            for device in devices {
                println!("{}\t{}", device.name, device.description);
            }
            return Ok(());
        }
        Ctl::Preset { id } => preset_request(id),
        Ctl::Mic { command } => match command {
            MicCommand::Mute { state } => match state {
                Switch::On => Request::MicSetMuted { muted: true },
                Switch::Off => Request::MicSetMuted { muted: false },
                Switch::Toggle => Request::MicToggleMute,
            },
            MicCommand::Bypass { state } => {
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
            MicCommand::Preset { id } => preset_request(id),
        },
        Ctl::Hotkeys { command } => {
            let request = match command {
                None | Some(HotkeysCommand::List) => Request::HotkeysGet,
                Some(HotkeysCommand::Set {
                    action,
                    shortcut,
                    secondary,
                }) => Request::HotkeysSet {
                    action,
                    secondary,
                    shortcut: (shortcut != "none").then_some(shortcut),
                },
            };
            let state: macafon_ipc::HotkeysState = client.call_as(request)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                println!("backend: {:?}", state.backend);
                for binding in &state.bindings {
                    println!(
                        "{:<28} {:<20} {}",
                        binding.action,
                        binding.shortcut.as_deref().unwrap_or("-"),
                        binding.error.as_deref().unwrap_or(&binding.command)
                    );
                }
            }
            return Ok(());
        }
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
        Ctl::InputChannel { channel } => Request::MicSetInputChannel {
            channel: match channel {
                Channel::Auto => InputChannel::Auto,
                Channel::Left => InputChannel::Left,
                Channel::Right => InputChannel::Right,
                Channel::Mix => InputChannel::Mix,
            },
        },
        Ctl::FallbackDevice { name } => Request::MicSetFallbackDevice {
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
        AbCommand::Outputs => {
            let outputs = client.call(Request::AbOutputs)?;
            println!("{}", serde_json::to_string_pretty(&outputs)?);
            return Ok(());
        }
        AbCommand::Export { side, file } => {
            let path = file.to_str().context("WAV path must be UTF-8")?.to_owned();
            client.call(Request::AbExport {
                side: match side {
                    Side::A => AbSide::A,
                    Side::B => AbSide::B,
                },
                path,
            })?;
            return Ok(());
        }
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

fn preset_request(id: String) -> Request {
    match id.as_str() {
        "next" => Request::MicCyclePreset { step: 1 },
        "previous" | "prev" => Request::MicCyclePreset { step: -1 },
        _ => Request::MicSetPreset { preset: id },
    }
}
