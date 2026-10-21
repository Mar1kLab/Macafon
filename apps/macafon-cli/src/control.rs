//! Control commands over the daemon client.

use crate::args::*;
use crate::output::{print_ab, print_json, print_mixer, print_report, print_state};
use anyhow::{Context, Result};
use macafon_ipc::{
    AbState, AutostartState, CalibrationState, CalibrationStep, ChannelState, Client, Device,
    Event, MicState, MixerState, PresetInfo, Request,
};
use std::io::Write;
use std::time::{Duration, Instant};

pub(super) fn run(json: bool, command: Ctl) -> Result<()> {
    let endpoint = macafon_ipc::Endpoint::for_current_user()
        .context("cannot locate the daemon endpoint (XDG_RUNTIME_DIR is not set)")?;
    let (mut client, _) = Client::connect(&endpoint, "macafon-cli")
        .with_context(|| format!("is macafond running? ({endpoint})"))?;
    let client = &mut client;
    let request = match command {
        Ctl::Status => Request::MicGet,
        Ctl::Display { command } => return display(client, command, json),
        Ctl::Dictation { state } => return dictation(client, state, json),
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
        Ctl::Autostart { state } => return autostart(client, state, json),
        Ctl::Quit => {
            return match client.call(Request::DaemonQuit) {
                // The daemon may exit before its reply is written.
                Ok(_) | Err(macafon_ipc::ClientError::Closed | macafon_ipc::ClientError::Io(_)) => {
                    Ok(())
                }
                Err(error) => Err(error.into()),
            };
        }
        Ctl::Presets { command } => return presets(client, command, json),
        Ctl::Mixer { state } => {
            let request = match state {
                None => Request::MixerGet,
                Some(switch) => Request::MixerSetEnabled {
                    enabled: switch.resolve(|| Ok(mixer(client)?.enabled))?,
                },
            };
            return print_mixer(&client.call_as(request)?, json);
        }
        Ctl::Channel { id, action } if id == "master" => return master(client, action, json),
        Ctl::Channel { id, action } => return channel(client, id, action, json),
        Ctl::Apps { command } => return apps(client, command, json),
        Ctl::Soundpad { command } => return soundpad(client, command, json),
        Ctl::Doctor => return doctor(client, json),
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
        Ctl::Hotkeys { command } => return hotkeys(client, command, json),
        Ctl::Preset { id }
        | Ctl::Mic {
            command: MicCommand::Preset { id },
        } => preset_request(id),
        Ctl::Mute { state }
        | Ctl::Mic {
            command: MicCommand::Mute { state },
        } => match state {
            Switch::On => Request::MicSetMuted { muted: true },
            Switch::Off => Request::MicSetMuted { muted: false },
            Switch::Toggle => Request::MicToggleMute,
        },
        Ctl::Bypass { state }
        | Ctl::Mic {
            command: MicCommand::Bypass { state },
        } => Request::MicSetBypass {
            bypass: state.resolve(|| Ok(mic(client)?.settings.bypass))?,
        },
        Ctl::Device { name } => Request::MicSetDevice {
            device: none_as_absent(name),
        },
        Ctl::InputChannel { channel } => Request::MicSetInputChannel {
            channel: channel.into(),
        },
        Ctl::FallbackDevice { name } => Request::MicSetFallbackDevice {
            device: none_as_absent(name),
        },
        Ctl::Gain { gain_db } => {
            let mut settings = mic(client)?.settings;
            settings.gain.enabled = true;
            settings.gain.params.gain_db = gain_db;
            Request::MicSet { settings }
        }
        Ctl::Calibrate => return calibrate(client, json),
        Ctl::Ab { command } => return ab(client, command, json),
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
        return print_json(&reply);
    }
    print_state(&serde_json::from_value(reply)?);
    Ok(())
}

fn mic(client: &mut Client) -> Result<MicState> {
    Ok(client.call_as(Request::MicGet)?)
}

fn mixer(client: &mut Client) -> Result<MixerState> {
    Ok(client.call_as(Request::MixerGet)?)
}

/// `none` on the command line clears an optional device or shortcut.
fn none_as_absent(name: String) -> Option<String> {
    (name != "none").then_some(name)
}

/// A volume argument: `40` sets the volume, `+5` / `-5` adjust it.
struct Volume {
    pct: f32,
    relative: bool,
}

impl Volume {
    fn parse(value: &str) -> Result<Self> {
        Ok(Self {
            pct: value
                .trim_start_matches('+')
                .parse()
                .with_context(|| format!("not a volume: {value}"))?,
            relative: value.starts_with(['+', '-']),
        })
    }
}

fn dictation(client: &mut Client, state: Switch, json: bool) -> Result<()> {
    let enabled = state.resolve(|| {
        let current: macafon_ipc::DictationState = client.call_as(Request::DictationGet)?;
        Ok(current.prefs.enabled)
    })?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let result: macafon_ipc::DictationState = loop {
        match client.call_as(Request::DictationSetPrefs {
            patch: macafon_ipc::DictationPrefsPatch {
                enabled: Some(enabled),
                ..Default::default()
            },
        }) {
            Ok(result) => break result,
            Err(macafon_ipc::ClientError::Rejected(error))
                if error.code == macafon_ipc::ErrorCode::Busy && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => return Err(error.into()),
        }
    };
    if json {
        return print_json(&result);
    }
    println!("dictation: {}", result.prefs.enabled);
    Ok(())
}

fn autostart(client: &mut Client, state: Option<Switch>, json: bool) -> Result<()> {
    let request = match state {
        None => Request::AutostartGet,
        Some(switch) => Request::AutostartSet {
            enabled: switch.resolve(|| {
                let current: AutostartState = client.call_as(Request::AutostartGet)?;
                Ok(current.enabled)
            })?,
        },
    };
    let state: AutostartState = client.call_as(request)?;
    if json {
        return print_json(&state);
    }
    println!("autostart: {} ({:?})", state.enabled, state.method);
    Ok(())
}

fn master(client: &mut Client, action: ChannelAction, json: bool) -> Result<()> {
    let state = mixer(client)?;
    let request = match action {
        ChannelAction::Volume { value } => {
            let volume = Volume::parse(&value)?;
            let pct = if volume.relative {
                state.master_volume_pct + volume.pct
            } else {
                volume.pct
            };
            Request::MixerSetMaster {
                volume_pct: Some(pct.clamp(0.0, 100.0)),
                muted: None,
            }
        }
        ChannelAction::Mute { state: switch } => Request::MixerSetMaster {
            volume_pct: None,
            muted: Some(switch.resolve(|| Ok(state.master_muted))?),
        },
        ChannelAction::Default => {
            anyhow::bail!("Master cannot be the default application channel")
        }
    };
    print_mixer(&client.call_as(request)?, json)
}

fn channel(client: &mut Client, id: String, action: ChannelAction, json: bool) -> Result<()> {
    let current = |client: &mut Client, id: &str| -> Result<ChannelState> {
        mixer(client)?
            .channels
            .into_iter()
            .find(|channel| channel.id == id)
            .with_context(|| format!("no channel {id:?}"))
    };
    let request = match action {
        ChannelAction::Volume { value } => {
            let volume = Volume::parse(&value)?;
            let delta_pct = if volume.relative {
                volume.pct
            } else {
                volume.pct - current(client, &id)?.settings.volume_pct
            };
            Request::ChannelAdjustVolume { id, delta_pct }
        }
        ChannelAction::Mute { state } => {
            if !changes(state, current(client, &id)?.settings.muted) {
                return Ok(());
            }
            Request::ChannelToggleMute { id }
        }
        ChannelAction::Default => Request::ChannelSetDefault { id },
    };
    print_mixer(&client.call_as(request)?, json)
}

fn apps(client: &mut Client, command: Option<AppsCommand>, json: bool) -> Result<()> {
    let request = match command {
        None | Some(AppsCommand::List) => Request::MixerGet,
        Some(AppsCommand::Move { key, channel }) => Request::AppsMove {
            app_key: key,
            channel,
        },
    };
    let state: MixerState = client.call_as(request)?;
    if json {
        return print_json(&state.apps);
    }
    for app in &state.apps {
        let playing = if app.active { "\t(playing)" } else { "" };
        println!("{}\t{}\t{}{playing}", app.key, app.channel, app.name);
    }
    Ok(())
}

fn soundpad(client: &mut Client, command: Option<SoundpadCommand>, json: bool) -> Result<()> {
    let request = match command {
        None | Some(SoundpadCommand::List) => {
            let state: Option<macafon_ipc::SoundpadState> = client.call_as(Request::SoundpadGet)?;
            let state = state.context("the soundpad is not available here")?;
            if json {
                return print_json(&state.sounds);
            }
            for sound in &state.sounds {
                println!("{}\t{}", sound.id, sound.name);
            }
            return Ok(());
        }
        Some(SoundpadCommand::Play { id }) => Request::SoundpadPlay { id },
        Some(SoundpadCommand::Random { category }) => Request::SoundpadPlayRandom { category },
        Some(SoundpadCommand::Stop) => Request::SoundpadStop,
        Some(SoundpadCommand::Pause) => Request::SoundpadSetPaused { paused: None },
    };
    client.call(request)?;
    Ok(())
}

fn doctor(client: &mut Client, json: bool) -> Result<()> {
    let report: macafon_ipc::DoctorReport = client.call_as(Request::DoctorRun)?;
    if json {
        return print_json(&report);
    }
    for check in &report.checks {
        let status = format!("{:?}", check.status).to_lowercase();
        let values = if check.values.is_empty() {
            String::new()
        } else {
            format!(": {}", check.values.join(", "))
        };
        println!("{status:<8} {:?}{values}", check.id);
    }
    Ok(())
}

fn hotkeys(client: &mut Client, command: Option<HotkeysCommand>, json: bool) -> Result<()> {
    let request = match command {
        None | Some(HotkeysCommand::List) => Request::HotkeysGet,
        Some(HotkeysCommand::Set {
            action,
            shortcut,
            secondary,
        }) => Request::HotkeysSet {
            action,
            secondary,
            shortcut: none_as_absent(shortcut),
        },
    };
    let state: macafon_ipc::HotkeysState = client.call_as(request)?;
    if json {
        return print_json(&state);
    }
    println!("backend: {:?}", state.backend);
    for binding in &state.bindings {
        println!(
            "{:<28} {:<20} {}",
            binding.action,
            binding.shortcut.as_deref().unwrap_or("-"),
            binding.error.as_deref().unwrap_or(&binding.command)
        );
    }
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
                    return print_json(&state);
                }
                println!("\r[{}]\n", "#".repeat(30));
                print_report(report);
                return Ok(());
            }
            CalibrationState::Failed { message, .. } => {
                if json {
                    print_json(&state)?;
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
                return print_json(&presets);
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
        return print_json(&preset);
    }
    println!("{}\t{}", preset.id, preset.name);
    Ok(())
}

fn ab(client: &mut Client, command: AbCommand, json: bool) -> Result<()> {
    let request = match command {
        AbCommand::Status => Request::AbGet,
        AbCommand::Outputs => return print_json(&client.call(Request::AbOutputs)?),
        AbCommand::Export { side, file } => {
            let path = file.to_str().context("WAV path must be UTF-8")?.to_owned();
            client.call(Request::AbExport {
                side: side.into(),
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
        AbCommand::Select { side: Some(side) } => Request::AbSelect { side: side.into() },
        AbCommand::Seek { position_ms } => Request::AbSeek { position_ms },
        AbCommand::Waveform { points } => {
            let waveform = client.call(Request::AbWaveform { points })?;
            println!("{}", serde_json::to_string(&waveform)?);
            return Ok(());
        }
        AbCommand::Match { state } => Request::AbSetLoudnessMatch {
            enabled: state
                .resolve(|| Ok(client.call_as::<AbState>(Request::AbGet)?.loudness_match))?,
        },
    };
    let state: AbState = client.call_as(request)?;
    print_ab(&state, json)
}

/// Whether `state` changes a switch that is `current` now: the daemon only toggles a channel's
/// mute, so an already matching state sends nothing.
fn changes(state: Switch, current: bool) -> bool {
    match state {
        Switch::On => !current,
        Switch::Off => current,
        Switch::Toggle => true,
    }
}

fn preset_request(id: String) -> Request {
    match id.as_str() {
        "next" => Request::MicCyclePreset { step: 1 },
        "previous" | "prev" => Request::MicCyclePreset { step: -1 },
        _ => Request::MicSetPreset { preset: id },
    }
}

fn display(client: &mut Client, command: Option<DisplayCommand>, json: bool) -> Result<()> {
    let request = match command {
        None | Some(DisplayCommand::List) => Request::DisplaysGet,
        Some(DisplayCommand::Apply { id }) => Request::DisplaysApply { id },
        Some(DisplayCommand::Next) => {
            let state: macafon_ipc::DisplaysState = client.call_as(Request::DisplaysGet)?;
            let index = state
                .active
                .as_ref()
                .and_then(|id| {
                    state
                        .profiles
                        .iter()
                        .position(|info| &info.profile.id == id)
                })
                .map_or(0, |index| (index + 1) % state.profiles.len());
            let info = state
                .profiles
                .get(index)
                .context("no display profiles saved")?;
            Request::DisplaysApply {
                id: info.profile.id.clone(),
            }
        }
        Some(DisplayCommand::Revert) => Request::DisplaysRevert,
        Some(DisplayCommand::AllOn) => Request::DisplaysAllOn,
        Some(DisplayCommand::Save { name, icon }) => Request::DisplaysSave { name, icon },
    };
    let state: macafon_ipc::DisplaysState = client.call_as(request)?;
    if json {
        return print_json(&state);
    }
    for info in state.profiles {
        println!(
            "{}\t{}\t{}\t{}",
            info.profile.id,
            info.profile.name,
            if state.active.as_ref() == Some(&info.profile.id) {
                "*"
            } else {
                ""
            },
            info.missing.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_signs_adjust_and_bare_numbers_set() {
        let up = Volume::parse("+5").expect("valid");
        let down = Volume::parse("-5").expect("valid");
        let set = Volume::parse("40").expect("valid");
        assert_eq!((up.pct, up.relative), (5.0, true));
        assert_eq!((down.pct, down.relative), (-5.0, true));
        assert_eq!((set.pct, set.relative), (40.0, false));
        assert!(Volume::parse("loud").is_err());
    }

    #[test]
    fn channel_mute_toggles_only_on_a_change() {
        assert!(changes(Switch::On, false) && !changes(Switch::On, true));
        assert!(changes(Switch::Off, true) && !changes(Switch::Off, false));
        assert!(changes(Switch::Toggle, true) && changes(Switch::Toggle, false));
        assert!(!Switch::Toggle.resolve(|| Ok(true)).expect("no error"));
        assert!(Switch::On.resolve(|| unreachable!()).expect("no error"));
    }
}
