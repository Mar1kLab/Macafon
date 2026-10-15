//! Doctor (PRD §9.6): turn a graph snapshot and the daemon state into plain checks.

use crate::platform::CHANNEL_PREFIX;
use macafon_backend::{Inspection, version_at_least};
use macafon_ipc::{DoctorCheck, DoctorCheckId, DoctorReport, DoctorStatus, MicState, MixerState};

/// Node name fragments of other noise suppressors that may process the same microphone.
const OTHER_PROCESSORS: &[&str] = &[
    "easyeffects",
    "noisetorch",
    "rnnoise",
    "deepfilter",
    "noise_suppress",
    "krisp",
];

pub struct Facts<'a> {
    /// The PipeWire graph; `None` on platforms without one.
    pub inspection: Option<Result<Inspection, String>>,
    pub mic: &'a MicState,
    pub mixer: &'a MixerState,
    /// `(rate, quantum)` seen by the microphone node.
    pub clock: Option<(u32, u32)>,
    /// Noise suppression fallbacks in the last 10 minutes.
    pub overloads: usize,
    /// Readable names of physical microphones by `node.name`.
    pub inputs: &'a [macafon_ipc::Device],
    pub hotkeys: macafon_ipc::HotkeyBackend,
}

fn check(id: DoctorCheckId, status: DoctorStatus, values: Vec<String>) -> DoctorCheck {
    DoctorCheck { id, status, values }
}

pub fn report(facts: Facts) -> DoctorReport {
    use DoctorCheckId as Id;
    use DoctorStatus as S;
    let mut checks = Vec::new();
    let inspection = facts
        .inspection
        .as_ref()
        .and_then(|graph| graph.as_ref().ok());
    match &facts.inspection {
        None => {}
        Some(Ok(graph)) if version_at_least(&graph.server_version, 1, 0) => checks.push(check(
            Id::Pipewire,
            S::Ok,
            vec![graph.server_version.clone()],
        )),
        Some(Ok(graph)) => checks.push(check(
            Id::Pipewire,
            S::Error,
            vec![graph.server_version.clone()],
        )),
        Some(Err(error)) => checks.push(check(Id::Pipewire, S::Error, vec![error.clone()])),
    }
    if let Some(graph) = inspection {
        match &graph.session_manager {
            Some((name, version))
                if name.starts_with("WirePlumber") && version_at_least(version, 0, 5) =>
            {
                checks.push(check(Id::SessionManager, S::Ok, vec![version.clone()]))
            }
            Some((_, version)) => {
                checks.push(check(Id::SessionManager, S::Warning, vec![version.clone()]))
            }
            None => checks.push(check(Id::SessionManager, S::Error, Vec::new())),
        }
    }
    let mic = facts.mic;
    let readable = |name: &str| {
        facts
            .inputs
            .iter()
            .find(|device| device.name == name)
            .map_or_else(|| name.to_owned(), |device| device.description.clone())
    };
    let device = mic
        .active_device
        .as_deref()
        .or(mic.device.as_deref())
        .map(readable);
    checks.push(match (&mic.device, mic.input_connected) {
        (None, _) => check(Id::MicrophoneInput, S::Warning, Vec::new()),
        (Some(_), true) => check(
            Id::MicrophoneInput,
            S::Ok,
            device.clone().into_iter().collect(),
        ),
        (Some(name), false) => check(Id::MicrophoneInput, S::Error, vec![readable(name)]),
    });
    if let Some(graph) = inspection {
        let mic_node = graph.nodes.iter().find(|node| node.name == "macafon_mic");
        let consumers: Vec<String> = mic_node
            .map(|mic| {
                let mut names: Vec<String> = graph
                    .links
                    .iter()
                    .filter(|(output, _)| *output == mic.id)
                    .filter_map(|(_, input)| graph.nodes.iter().find(|node| node.id == *input))
                    .filter(|node| !node.name.starts_with("macafon_"))
                    .map(|node| node.description.clone())
                    .collect();
                names.sort();
                names.dedup();
                names
            })
            .unwrap_or_default();
        checks.push(if consumers.is_empty() {
            check(Id::MicrophoneUsed, S::Info, Vec::new())
        } else {
            check(Id::MicrophoneUsed, S::Ok, consumers)
        });
        let mut others: Vec<String> = graph
            .nodes
            .iter()
            .filter(|node| !node.name.starts_with("macafon"))
            .filter(|node| {
                let text = format!("{} {}", node.name, node.description).to_lowercase();
                OTHER_PROCESSORS
                    .iter()
                    .any(|fragment| text.contains(fragment))
            })
            .map(|node| node.description.clone())
            .collect();
        others.sort();
        others.dedup();
        checks.push(if others.is_empty() {
            check(Id::DoubleProcessing, S::Ok, Vec::new())
        } else {
            check(Id::DoubleProcessing, S::Warning, others)
        });
    }
    let mixer = facts.mixer;
    if !mixer.enabled {
        checks.push(check(Id::Mixer, S::Info, Vec::new()));
    } else if let Some(graph) = inspection {
        let missing: Vec<String> = mixer
            .channels
            .iter()
            .filter(|channel| {
                let name = format!("{CHANNEL_PREFIX}{}", channel.id);
                let node = graph.nodes.iter().find(|node| node.name == name);
                !node.is_some_and(|node| graph.links.iter().any(|(output, _)| *output == node.id))
            })
            .map(|channel| channel.name.clone())
            .collect();
        checks.push(match (&mixer.output, missing.is_empty()) {
            (None, _) => check(Id::Mixer, S::Error, Vec::new()),
            (Some(output), true) => {
                let description = mixer
                    .outputs
                    .iter()
                    .find(|device| &device.name == output)
                    .map_or_else(|| output.clone(), |device| device.description.clone());
                check(Id::Mixer, S::Ok, vec![description])
            }
            (Some(_), false) => check(Id::Mixer, S::Error, missing),
        });
        let unmovable: Vec<String> = mixer
            .apps
            .iter()
            .filter(|app| app.streams > 0 && !app.movable)
            .map(|app| app.name.clone())
            .collect();
        checks.push(if unmovable.is_empty() {
            check(Id::UnmovableApps, S::Ok, Vec::new())
        } else {
            check(Id::UnmovableApps, S::Info, unmovable)
        });
    }
    let processing_values = facts
        .clock
        .map(|(rate, quantum)| {
            vec![
                quantum.to_string(),
                rate.to_string(),
                facts.overloads.to_string(),
            ]
        })
        .unwrap_or_else(|| vec![String::new(), String::new(), facts.overloads.to_string()]);
    checks.push(check(
        Id::Processing,
        if facts.overloads > 0 {
            S::Warning
        } else {
            S::Ok
        },
        processing_values,
    ));
    checks.push(match inspection.and_then(|graph| graph.load) {
        Some(load) => check(
            Id::Xruns,
            if load.xrun_count > 0 {
                S::Warning
            } else {
                S::Ok
            },
            vec![
                load.xrun_count.to_string(),
                format!("{:.0}", load.cpu_load[1] * 100.0),
            ],
        ),
        None => check(Id::Xruns, S::Info, Vec::new()),
    });
    checks.push(match facts.hotkeys {
        macafon_ipc::HotkeyBackend::Portal => check(Id::Hotkeys, S::Ok, vec!["portal".into()]),
        macafon_ipc::HotkeyBackend::X11 => check(Id::Hotkeys, S::Ok, vec!["X11".into()]),
        macafon_ipc::HotkeyBackend::Unavailable => check(Id::Hotkeys, S::Info, Vec::new()),
    });
    DoctorReport {
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        pipewire_version: inspection.map(|graph| graph.server_version.clone()),
        session_manager: inspection
            .and_then(|graph| graph.session_manager.as_ref())
            .map(|(name, version)| format!("{name} {version}")),
        quantum: facts.clock.map(|(_, quantum)| quantum),
        sample_rate_hz: facts.clock.map(|(rate, _)| rate),
        checks,
    }
}
