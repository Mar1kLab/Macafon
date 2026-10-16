//! Windows: WASAPI and the free VB-CABLE (PRD §12.2, ADR-0004).
//!
//! Channels are volume groups on Windows (§12.2.2): the backend sets app volumes and the
//! default output itself, so the channel-node functions here have nothing to do.

use anyhow::{Context, Result};
use macafon_backend::{Inspection, MicrophoneHost, Node};
use macafon_backend_windows as win;
use macafon_ipc::{DoctorCheck, DoctorCheckId, DoctorStatus, Endpoint};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub use win::{ControlReceiver, control_channel};

/// Channels are volume groups, not devices: nothing to make the default output.
pub const CHANNEL_NODES: bool = false;

/// Channel nodes do not exist on Windows; only used to build names nobody looks up.
pub const CHANNEL_PREFIX: &str = "macafon_channel_";

const NO_CHANNELS: &str = "output channels are not available on Windows yet";

pub fn channel_node_name(id: &str) -> String {
    format!("{CHANNEL_PREFIX}{id}")
}

pub fn endpoint() -> Result<Endpoint> {
    Endpoint::for_current_user().context("cannot read the current user's SID")
}

fn state_dir() -> Result<PathBuf, String> {
    macafon_config::state_dir().map_err(|error| error.to_string())
}

/// Run the microphone until Ctrl+C, logoff, shutdown or Quit. A default microphone left over
/// from a crash is restored before start, Macafon's own change after it (WIN-MIC-005).
pub fn run<H: MicrophoneHost + 'static>(
    host: Rc<RefCell<H>>,
    control: ControlReceiver<H::Message>,
    flush: impl FnOnce(),
) -> Result<()> {
    let _com = win::ComGuard::new().context("cannot initialise COM")?;
    let state_dir = state_dir().map_err(anyhow::Error::msg)?;
    if let Err(error) = restore_default_source() {
        tracing::warn!(%error, "Cannot restore previous default microphone yet");
    }
    win::run_microphone(host, control, &state_dir, || {
        flush();
        if let Err(error) = restore_default_source() {
            tracing::warn!(%error, "Cannot restore previous default microphone; recovery journal retained");
        }
    })
    .context("Cannot run microphone")
}

/// MIC-012: `CABLE Output` becomes the default microphone, the original is journaled.
pub fn set_default_source() -> Result<(), String> {
    win::set_default_microphone(&state_dir()?).map_err(|error| error.to_string())
}

pub fn restore_default_source() -> Result<(), String> {
    win::restore_default_microphone(&state_dir()?).map_err(|error| error.to_string())
}

pub fn set_default_sink(_target: &str) -> Result<(), String> {
    Err(NO_CHANNELS.into())
}

pub fn restore_default_sink() -> Result<(), String> {
    Ok(())
}

pub fn previous_default_sink() -> Option<String> {
    None
}

/// Real output devices for the A/B player; the cable is hidden (WIN-MIC-009).
pub fn output_devices() -> Result<Vec<Node>, String> {
    win::output_devices().map_err(|error| error.to_string())
}

/// No PipeWire graph to inspect; the cable checks come from `doctor_checks`.
pub fn inspect() -> Option<Result<Inspection, String>> {
    None
}

/// VB-CABLE checks (WIN-MIC-001/004/007/008) from a read-only snapshot. `microphone` is the
/// connected physical input, if any.
pub fn doctor_checks(microphone: Option<&str>) -> Vec<DoctorCheck> {
    use crate::doctor::check;
    use DoctorCheckId as Id;
    use DoctorStatus as S;
    let pid = std::process::id();
    // The microphone may vanish between the state and the snapshot; the cable still counts.
    let facts = match win::diagnostics::collect(microphone, pid)
        .or_else(|_| win::diagnostics::collect(None, pid))
    {
        Ok(facts) => facts,
        Err(error) => {
            tracing::warn!(%error, "Cannot collect VB-CABLE facts");
            return vec![check(Id::VbCable, S::Error, Vec::new())];
        }
    };
    let cable = &facts.cable;
    let ignored: Vec<String> = cable
        .other_vb_audio
        .iter()
        .map(|endpoint| endpoint.name.clone())
        .collect();
    let mut checks = vec![if !cable.installed {
        check(Id::VbCable, S::Error, Vec::new())
    } else if cable.active_endpoints.is_empty() {
        check(Id::VbCable, S::Warning, Vec::new())
    } else if !ignored.is_empty() {
        check(Id::VbCable, S::Info, ignored)
    } else {
        check(Id::VbCable, S::Ok, Vec::new())
    }];
    if cable.active_endpoints.is_empty() {
        return checks;
    }
    let resampled: Vec<String> = facts
        .endpoints
        .iter()
        .filter_map(|facts| {
            let format = facts.mix_format.as_ref().ok()?;
            format
                .needs_resampling()
                .then(|| format!("{} ({} Hz)", facts.endpoint.name, format.rate_hz))
        })
        .collect();
    checks.push(if resampled.is_empty() {
        check(Id::CableFormat, S::Ok, Vec::new())
    } else {
        check(Id::CableFormat, S::Warning, resampled)
    });
    let mut writers: Vec<String> = facts
        .foreign_writers
        .iter()
        .map(|session| {
            session
                .executable
                .as_deref()
                .and_then(|path| path.rsplit(['\\', '/']).next())
                .map_or_else(|| session.display_name.clone(), str::to_owned)
        })
        .collect();
    writers.sort();
    writers.dedup();
    checks.push(if writers.is_empty() {
        check(Id::CableWriters, S::Ok, Vec::new())
    } else {
        check(Id::CableWriters, S::Warning, writers)
    });
    checks.push(if facts.cable_default_roles.is_empty() {
        check(Id::CableDefaultOutput, S::Ok, Vec::new())
    } else {
        DoctorCheck {
            fixable: true,
            ..check(Id::CableDefaultOutput, S::Warning, Vec::new())
        }
    });
    // Engine periods of the microphone and the cable input, in ms; not end-to-end latency.
    let period_ms = |facts: &win::diagnostics::EndpointFacts| {
        facts.engine_periods.as_ref().ok().map(|periods| {
            let ms = |frames: u32| frames as f32 * 1000.0 / periods.mix_rate_hz.max(1) as f32;
            format!("{:.1}", ms(periods.min_frames))
        })
    };
    let cable_input = facts
        .endpoints
        .iter()
        .find(|facts| facts.endpoint.flow == win::DataFlow::Render && facts.endpoint.is_vb_cable());
    let mic = facts.endpoints.iter().find(|facts| {
        facts.endpoint.flow == win::DataFlow::Capture && !facts.endpoint.is_vb_audio()
    });
    checks.push(check(
        Id::CableLatency,
        S::Info,
        vec![
            mic.and_then(period_ms).unwrap_or_else(|| "?".into()),
            cable_input
                .and_then(period_ms)
                .unwrap_or_else(|| "?".into()),
        ],
    ));
    checks
}

/// One-click Doctor fixes: give the system output back to a real device (WIN-MIC-004).
pub fn doctor_fix(check: DoctorCheckId) -> Result<(), String> {
    match check {
        DoctorCheckId::CableDefaultOutput => {
            win::fix_cable_default_output(&state_dir()?).map_err(|error| error.to_string())
        }
        _ => Err("no automatic fix for this check".into()),
    }
}

/// `macafond devices`: active endpoints and the VB-CABLE.
pub fn print_devices() -> Result<()> {
    let _com = win::ComGuard::new()?;
    let endpoints = win::endpoints()?;
    let cable = win::find_vb_cable(&endpoints);
    for endpoint in &endpoints {
        let role = match &cable {
            Some(cable) if cable.input.id == endpoint.id => "\tcable input",
            Some(cable) if cable.output.id == endpoint.id => "\tcable output",
            _ => "",
        };
        println!(
            "{:?}\t{}\t{}{role}",
            endpoint.flow, endpoint.id, endpoint.name
        );
    }
    if cable.is_none() {
        println!("VB-CABLE is not installed");
    }
    Ok(())
}

/// `macafond cleanup` for the uninstaller (§12.2.3): Quit a running daemon (it restores on
/// its way out), restore whatever a crash left in the journals, remove the Run value.
pub fn cleanup() -> Result<()> {
    let endpoint = endpoint()?;
    if let Ok((mut client, _)) = macafon_ipc::Client::connect(&endpoint, "macafond-cleanup") {
        // The daemon may exit before its reply is written.
        drop(client.call(macafon_ipc::Request::DaemonQuit));
    }
    let _ownership = claim_cleanup_pipe(&endpoint, std::time::Duration::from_secs(10))?;
    let state_dir = state_dir().map_err(anyhow::Error::msg)?;
    win::restore_all(&state_dir).context("cannot restore system audio settings")?;
    crate::autostart::set(false).map_err(anyhow::Error::msg)?;
    Ok(())
}

fn wait_for_daemon_exit(endpoint: &Endpoint, timeout: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while macafon_ipc::transport::connect(endpoint).is_ok() {
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "macafond is still running; cannot restore its journals concurrently"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(())
}

fn claim_cleanup_pipe(
    endpoint: &Endpoint,
    timeout: std::time::Duration,
) -> Result<macafon_ipc::transport::Listener> {
    wait_for_daemon_exit(endpoint, timeout)?;
    // Keep startup and a second cleanup out until all journal writes have finished.
    macafon_ipc::transport::Listener::bind(endpoint)
        .context("cannot claim daemon pipe for exclusive audio recovery")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_refuses_to_restore_while_the_daemon_still_owns_the_pipe() {
        let endpoint = Endpoint::pipe(format!("macafon-cleanup-test-{}", std::process::id()));
        let listener = macafon_ipc::transport::Listener::bind(&endpoint).expect("fake daemon");
        assert!(wait_for_daemon_exit(&endpoint, std::time::Duration::ZERO).is_err());
        drop(listener);
        assert!(wait_for_daemon_exit(&endpoint, std::time::Duration::ZERO).is_ok());
    }

    #[test]
    fn cleanup_holds_the_single_instance_name_until_recovery_finishes() {
        let endpoint = Endpoint::pipe(format!("macafon-cleanup-owner-test-{}", std::process::id()));
        let owner =
            claim_cleanup_pipe(&endpoint, std::time::Duration::ZERO).expect("claim recovery");
        assert!(macafon_ipc::transport::Listener::bind(&endpoint).is_err());
        drop(owner);
        assert!(macafon_ipc::transport::Listener::bind(&endpoint).is_ok());
    }
}
