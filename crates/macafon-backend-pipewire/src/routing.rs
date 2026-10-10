use crate::{Connection, Error, node};
use pipewire::{self as pw, types::ObjectType};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

struct ObservedLink {
    output_node: u32,
    input_node: u32,
    usable: bool,
}

type LinkBindings = HashMap<u32, (pw::link::LinkListener, pw::link::Link)>;
type TargetProperty = Option<(Option<String>, String)>;

/// Move one stream through WirePlumber and confirm its actual links. On timeout,
/// restore the previous target property unless another client has changed it.
pub fn route_stream(stream_id: u32, sink_name: &str) -> Result<(), Error> {
    let connection = Connection::new()?;
    let registry = connection.core.get_registry_rc()?;
    let nodes = Rc::new(RefCell::new(Vec::new()));
    let metadata = Rc::new(RefCell::new(None));
    let links = Rc::new(RefCell::new(HashMap::<u32, ObservedLink>::new()));
    let bindings = Rc::new(RefCell::new(LinkBindings::new()));
    let stream_binding = Rc::new(RefCell::new(None));
    let restrictions = Rc::new(Cell::new((false, false)));
    let bound_stream = stream_binding.clone();
    let stream_restrictions = restrictions.clone();
    let collected = nodes.clone();
    let found_metadata = metadata.clone();
    let bind_registry = registry.clone();
    let observed = links.clone();
    let bound = bindings.clone();
    let removed_links = links.clone();
    let removed_bindings = bindings.clone();
    let failure = connection.failure.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(node) = node(global) {
                collected.borrow_mut().push(node);
            }
            if global.type_ == ObjectType::Node && global.id == stream_id {
                let stream = match bind_registry.bind::<pw::node::Node, _>(global) {
                    Ok(stream) => stream,
                    Err(error) => {
                        *failure.borrow_mut() = Some(error.into());
                        return;
                    }
                };
                let flags = stream_restrictions.clone();
                let listener = stream
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(props) = info.props() {
                            let enabled = |key| {
                                props
                                    .get(key)
                                    .is_some_and(|value| matches!(value, "true" | "1"))
                            };
                            flags.set((enabled("node.dont-reconnect"), enabled("node.dont-move")));
                        }
                    })
                    .register();
                *bound_stream.borrow_mut() = Some((listener, stream));
            }
            if global.type_ == ObjectType::Metadata
                && global
                    .props
                    .is_some_and(|p| p.get("metadata.name") == Some("default"))
            {
                *found_metadata.borrow_mut() =
                    Some(bind_registry.bind::<pw::metadata::Metadata, _>(global));
            }
            if global.type_ == ObjectType::Link {
                let link = match bind_registry.bind::<pw::link::Link, _>(global) {
                    Ok(link) => link,
                    Err(error) => {
                        *failure.borrow_mut() = Some(error.into());
                        return;
                    }
                };
                let observed = observed.clone();
                let id = global.id;
                let listener = link
                    .add_listener_local()
                    .info(move |info| {
                        observed.borrow_mut().insert(
                            id,
                            ObservedLink {
                                output_node: info.output_node_id(),
                                input_node: info.input_node_id(),
                                usable: matches!(
                                    info.state(),
                                    pw::link::LinkState::Active | pw::link::LinkState::Paused
                                ),
                            },
                        );
                    })
                    .register();
                bound.borrow_mut().insert(id, (listener, link));
            }
        })
        .global_remove(move |id| {
            removed_links.borrow_mut().remove(&id);
            removed_bindings.borrow_mut().remove(&id);
        })
        .register();
    connection.roundtrip()?;
    // Registry properties are sparse; receive the bound stream's full NodeInfo.
    connection.roundtrip()?;
    let sink = {
        let nodes = nodes.borrow();
        nodes
            .iter()
            .find(|node| node.id == stream_id && node.media_class == "Stream/Output/Audio")
            .ok_or_else(|| Error::Message("stream ID is not an audio playback stream".into()))?;
        if restrictions.get().0 {
            return Err(Error::Message(
                "stream has node.dont-reconnect=true; WirePlumber metadata cannot move it".into(),
            ));
        }
        if restrictions.get().1 {
            return Err(Error::Message(
                "stream has node.dont-move=true; WirePlumber metadata cannot move it".into(),
            ));
        }
        nodes
            .iter()
            .find(|node| node.name == sink_name && node.media_class == "Audio/Sink")
            .cloned()
            .ok_or_else(|| Error::Message("target is not an audio sink".into()))?
    };
    let metadata = metadata.borrow_mut().take().ok_or_else(|| {
        Error::Message("default metadata unavailable; is WirePlumber running?".into())
    })??;
    let current_target = Rc::new(RefCell::new(TargetProperty::None));
    let target = current_target.clone();
    let _metadata_listener = metadata
        .add_listener_local()
        .property(move |subject, key, type_, value| {
            if subject == stream_id && key == Some("target.object") {
                *target.borrow_mut() =
                    value.map(|value| (type_.map(str::to_owned), value.to_owned()));
            }
            0
        })
        .register();
    // Binding metadata and links in registry callbacks creates additional requests;
    // synchronize once more to receive their initial values before changing anything.
    connection.roundtrip()?;
    let previous_target = current_target.borrow().clone();
    metadata.set_property(
        stream_id,
        "target.object",
        Some("Spa:Id"),
        Some(&sink.serial),
    );

    let confirmed = Rc::new(Cell::new(false));
    let done = confirmed.clone();
    let loop_ = connection.mainloop.clone();
    let started = Instant::now();
    let sink_id = sink.id;
    let timer = connection.mainloop.loop_().add_timer(move |_| {
        let links = links.borrow();
        let mut outgoing = links
            .values()
            .filter(|link| link.output_node == stream_id && link.usable)
            .peekable();
        if outgoing.peek().is_some() && outgoing.all(|link| link.input_node == sink_id) {
            done.set(true);
            loop_.quit();
        } else if started.elapsed() >= Duration::from_secs(3) {
            loop_.quit();
        }
    });
    timer
        .update_timer(
            Some(Duration::from_millis(25)),
            Some(Duration::from_millis(25)),
        )
        .into_result()
        .map_err(|error| Error::Message(error.to_string()))?;
    connection.mainloop.run();
    // Disable the poll timer before any rollback roundtrip can run the loop again.
    drop(timer);
    let result = connection.check();
    if result.is_ok() && confirmed.get() {
        return Ok(());
    }
    let mut restored = false;
    if current_target
        .borrow()
        .as_ref()
        .is_some_and(|(_, value)| value == &sink.serial)
    {
        let (type_, value) = previous_target
            .as_ref()
            .map_or((None, None), |(type_, value)| {
                (type_.as_deref(), Some(value.as_str()))
            });
        metadata.set_property(stream_id, "target.object", type_, value);
        connection.roundtrip()?;
        restored = true;
    }
    result?;
    let outcome = if restored {
        "previous target restored"
    } else {
        "newer target metadata preserved"
    };
    Err(Error::Message(format!(
        "WirePlumber did not move the stream; {outcome}. Check linking.allow-moving-streams and node.dont-move"
    )))
}
