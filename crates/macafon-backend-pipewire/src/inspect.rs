//! One-shot snapshot of the graph for diagnostics (PRD §9.6): server and session manager
//! versions, nodes and links. Runs on its own connection; never touches the audio session.

use crate::{Connection, Error, Inspection, node};
use pipewire::{self as pw, types::ObjectType};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

/// Profiler events arrive every cycle; this is many cycles even at quantum 1024.
const LOAD_SAMPLE: Duration = Duration::from_millis(300);

pub fn inspect() -> Result<Inspection, Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let connection = Connection::with_mainloop(mainloop, true)?;
    let result = Rc::new(RefCell::new(Inspection::default()));
    let version = result.clone();
    let _core = connection
        .core
        .add_listener_local()
        .info(move |info| version.borrow_mut().server_version = info.version().to_owned())
        .register();
    let registry = connection.core.get_registry_rc()?;
    let bind = registry.clone();
    let clients = Rc::new(RefCell::new(Vec::new()));
    let collected = result.clone();
    let bound = clients.clone();
    let profiler = Rc::new(Cell::new(None));
    let profiler_found = profiler.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Profiler {
                profiler_found.set(Some((global.id, global.version)));
            }
            if let Some(node) = node(global) {
                collected.borrow_mut().nodes.push(node);
            }
            if global.type_ == ObjectType::Link {
                if let Some(props) = global.props {
                    let id = |key| props.get(key).and_then(|v: &str| v.parse().ok());
                    if let (Some(output), Some(input)) =
                        (id("link.output.node"), id("link.input.node"))
                    {
                        collected.borrow_mut().links.push((output, input));
                    }
                }
            }
            if global.type_ == ObjectType::Client {
                if let Ok(client) = bind.bind::<pw::client::Client, _>(global) {
                    let found = collected.clone();
                    let listener = client
                        .add_listener_local()
                        .info(move |info| {
                            let props = info.props();
                            let get = |key: &str| props.and_then(|p| p.get(key)).map(str::to_owned);
                            let name = get("application.name").unwrap_or_default();
                            if name.starts_with("WirePlumber") || name.contains("media-session") {
                                found.borrow_mut().session_manager =
                                    Some((name, get("application.version").unwrap_or_default()));
                            }
                        })
                        .register();
                    bound.borrow_mut().push((client, listener));
                }
            }
        })
        .register();
    connection.roundtrip()?;
    // Bound clients answer with their info after one more round trip.
    connection.roundtrip()?;
    drop(clients);
    let mic = result
        .borrow()
        .nodes
        .iter()
        .find(|node| node.name == crate::VIRTUAL_MIC_NAME)
        .map(|node| node.id);
    if let (Some((id, version)), Some(node_id), true) =
        (profiler.get(), mic, connection.profiler_available)
    {
        let sample = result.clone();
        let loop_ = connection.mainloop.clone();
        let _profiler =
            crate::profiler::Profiler::load(registry, id, version, node_id, move |load| {
                sample.borrow_mut().load = Some(load);
                loop_.quit();
            })?;
        let loop_ = connection.mainloop.clone();
        let deadline = connection.mainloop.loop_().add_timer(move |_| loop_.quit());
        deadline
            .update_timer(Some(LOAD_SAMPLE), None)
            .into_result()
            .map_err(|error| Error::Message(error.to_string()))?;
        connection.mainloop.run();
        connection.check()?;
    }
    let inspection = result.borrow().clone();
    Ok(inspection)
}
