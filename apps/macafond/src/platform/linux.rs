//! Linux: the PipeWire backend (PRD §12.1).

use anyhow::{Context, Result};
use macafon_backend::{Inspection, MicrophoneHost, Node};
use macafon_backend_pipewire as pw;
use macafon_ipc::Endpoint;
use std::cell::RefCell;
use std::rc::Rc;

/// Channels are PipeWire nodes the default output can point to (OUT-005).
pub const CHANNEL_NODES: bool = true;

pub use pw::{CHANNEL_PREFIX, ControlReceiver, channel_node_name, control_channel};

pub fn endpoint() -> Result<Endpoint> {
    Endpoint::for_current_user().context("XDG_RUNTIME_DIR is not set")
}

/// Run the virtual microphone until SIGINT/SIGTERM or Quit. Defaults left over from a crash are
/// restored before start and Macafon's own changes after it.
pub fn run<H: MicrophoneHost + 'static>(
    host: Rc<RefCell<H>>,
    control: ControlReceiver<H::Message>,
    flush: impl FnOnce(),
) -> Result<()> {
    restore_defaults("yet");
    let result = pw::run_microphone(host, control)
        .context("Cannot run microphone; check PipeWire and WirePlumber");
    flush();
    restore_defaults("; recovery journal retained");
    result
}

fn restore_defaults(retry: &str) {
    if let Err(error) = restore_default_source() {
        tracing::warn!(%error, "Cannot restore previous default microphone{retry}");
    }
    if let Err(error) = restore_default_sink() {
        tracing::warn!(%error, "Cannot restore previous default output{retry}");
    }
}

/// MIC-012: Macafon Microphone becomes the default microphone, the original is journaled.
pub fn set_default_source() -> Result<(), String> {
    pw::default_source_journal()
        .and_then(|journal| pw::set_default_source(&journal))
        .map_err(|error| error.to_string())
}

pub fn restore_default_source() -> Result<(), String> {
    pw::default_source_journal()
        .and_then(|journal| pw::restore_default_source(&journal))
        .map_err(|error| error.to_string())
}

/// OUT-005: the default output becomes `target`, the original is journaled.
pub fn set_default_sink(target: &str) -> Result<(), String> {
    pw::default_sink_journal()
        .and_then(|journal| pw::set_default_sink(&journal, target))
        .map_err(|error| error.to_string())
}

pub fn restore_default_sink() -> Result<(), String> {
    pw::default_sink_journal()
        .and_then(|journal| pw::restore_default_sink(&journal))
        .map_err(|error| error.to_string())
}

pub fn previous_default_sink() -> Option<String> {
    pw::default_sink_journal()
        .ok()
        .and_then(|journal| pw::previous_default_sink(&journal))
}

/// Real output devices for the A/B player: sinks that are not Macafon channels.
pub fn output_devices() -> Result<Vec<Node>, String> {
    let nodes = pw::list_nodes().map_err(|error| error.to_string())?;
    Ok(nodes
        .into_iter()
        .filter(|node| node.media_class == "Audio/Sink" && !node.name.starts_with(CHANNEL_PREFIX))
        .collect())
}

/// The graph snapshot for the Doctor.
pub fn inspect() -> Option<Result<Inspection, String>> {
    Some(pw::inspect().map_err(|error| error.to_string()))
}

/// `macafond devices`: physical inputs, sinks and playback streams.
pub fn print_devices() -> Result<()> {
    for node in pw::list_nodes()? {
        if matches!(
            node.media_class.as_str(),
            "Audio/Source" | "Audio/Sink" | "Stream/Output/Audio"
        ) && !node.name.starts_with("macafon_")
            && !node.name.ends_with(".monitor")
        {
            println!(
                "{}\t{}\t{}\t{}",
                node.id, node.media_class, node.name, node.description
            );
        }
    }
    Ok(())
}

/// `macafond route`: M0 experiment, move one playback stream through WirePlumber metadata.
pub fn route(stream_id: u32, sink_name: &str) -> Result<()> {
    pw::route_stream(stream_id, sink_name)?;
    tracing::info!(stream_id, sink = %sink_name, "Stream moved; PipeWire links confirmed");
    Ok(())
}
