//! Windows: WASAPI and the free VB-CABLE (PRD §12.2, ADR-0004).
//!
//! Channels are volume groups on Windows (§12.2.2): the backend sets app volumes and the
//! default output itself, so the channel-node functions here have nothing to do.

use anyhow::{Context, Result};
use macafon_backend::{Inspection, MicrophoneHost, Node};
use macafon_backend_windows as win;
use macafon_ipc::Endpoint;
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

/// No PipeWire graph to inspect; Windows checks come with M3.3.
pub fn inspect() -> Option<Result<Inspection, String>> {
    None
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
