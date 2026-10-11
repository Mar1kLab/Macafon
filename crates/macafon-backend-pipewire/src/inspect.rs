//! One-shot snapshot of the graph for diagnostics (PRD §9.6): server and session manager
//! versions, nodes and links. Runs on its own connection; never touches the audio session.

use crate::{Connection, Error, Node, node};
use pipewire::{self as pw, types::ObjectType};
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, Default)]
pub struct Inspection {
    pub server_version: String,
    /// `application.name` and `application.version` of the session manager, if one runs.
    pub session_manager: Option<(String, String)>,
    pub nodes: Vec<Node>,
    /// `(output node id, input node id)` of every link.
    pub links: Vec<(u32, u32)>,
}

pub fn inspect() -> Result<Inspection, Error> {
    let connection = Connection::new()?;
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
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
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
    let inspection = result.borrow().clone();
    Ok(inspection)
}

/// `major.minor` of a version string, for comparisons.
pub fn version_at_least(version: &str, major: u32, minor: u32) -> bool {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let found = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    found >= (major, minor)
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare_by_major_and_minor() {
        assert!(super::version_at_least("1.6.2", 1, 0));
        assert!(super::version_at_least("0.5.13", 0, 5));
        assert!(!super::version_at_least("0.4.17", 0, 5));
        assert!(!super::version_at_least("", 1, 0));
    }
}
