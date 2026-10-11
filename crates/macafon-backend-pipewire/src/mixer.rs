//! Output channels in the graph session (PRD §7, §12.1): channel nodes, their links to the
//! real output device, playback stream tracking and routing through the `default` metadata.

use crate::channel_node::{ChannelNode, channel_node_name};
use crate::{Error, Graph, Node, Port};
use macafon_engine::ChannelRt;
use pipewire::{self as pw, properties::properties, types::ObjectType};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// A change to the set of channel nodes, requested by the host on the control loop.
#[allow(clippy::large_enum_variant)] // transient, one per user action
pub enum ChannelRequest {
    Create {
        id: String,
        description: String,
        processor: ChannelRt,
    },
    Remove {
        id: String,
    },
    Describe {
        id: String,
        description: String,
    },
    Latency {
        id: String,
        samples: u32,
    },
}

/// What to do with one application stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Send it to the channel with this id.
    Channel(String),
    /// Undo Macafon's target, if Macafon set it.
    Release,
    /// Leave it alone.
    Ignore,
}

/// An application's playback stream (`Stream/Output/Audio`), from its full node info.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackStream {
    pub id: u32,
    pub serial: String,
    pub node_name: String,
    pub binary: Option<String>,
    pub flatpak_id: Option<String>,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub icon_name: Option<String>,
    pub media_role: Option<String>,
    pub process_id: Option<u32>,
    /// The node is running: the app is playing right now (APP-003).
    pub running: bool,
    /// `node.dont-move` / `node.dont-reconnect` forbid moving it through metadata.
    pub movable: bool,
    /// Current `target.object` in the default metadata.
    pub target: Option<String>,
}

impl PlaybackStream {
    /// Macafon's own streams (the A/B player) are never routed through channels.
    pub fn ours(&self) -> bool {
        self.node_name.starts_with("macafon")
            || self
                .binary
                .as_deref()
                .is_some_and(|binary| binary.starts_with("macafon"))
    }
}

/// The host side of the mixer; every method runs on the control loop.
pub trait MixerHost {
    /// Pending channel node changes, polled every 100 ms.
    fn channel_requests(&mut self) -> Vec<ChannelRequest> {
        Vec::new()
    }
    /// Real output devices (`Audio/Sink`, not ours) changed (OUT-001, OUT-004).
    fn outputs_changed(&mut self, _outputs: &[Node]) {}
    /// `default.audio.sink` changed; `None` when unset.
    fn default_sink_changed(&mut self, _name: Option<&str>) {}
    /// `default.audio.source` changed; `None` when unset (MIC-012).
    fn default_source_changed(&mut self, _name: Option<&str>) {}
    /// Physical sink the channels play into (`node.name`), polled every 100 ms.
    fn output_device(&self) -> Option<String> {
        None
    }
    /// Playback streams appeared, changed or vanished (APP-001).
    fn streams_changed(&mut self, _streams: &[PlaybackStream]) {}
    /// Where `stream` belongs; consulted every 100 ms for each stream.
    fn route(&self, _stream: &PlaybackStream) -> Route {
        Route::Ignore
    }
}

#[derive(Default)]
pub(crate) struct StreamInfo {
    pub streams: HashMap<u32, PlaybackStream>,
    pub changed: bool,
}

pub(crate) type SharedStreams = Rc<RefCell<StreamInfo>>;

/// Metadata state shared with the registry and metadata listeners.
#[derive(Default)]
pub(crate) struct MetadataState {
    pub targets: HashMap<u32, String>,
    pub default_sink: Option<String>,
    pub default_changed: bool,
    pub default_source: Option<String>,
    pub source_changed: bool,
}

pub(crate) struct BoundMetadata {
    pub metadata: pw::metadata::Metadata,
    _listener: pw::metadata::MetadataListener,
}

/// Bind a playback stream to receive its full properties and state.
pub(crate) fn bind_stream(
    registry: &pw::registry::RegistryRc,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    streams: &SharedStreams,
) -> Option<(pw::node::Node, pw::node::NodeListener)> {
    let node: pw::node::Node = registry.bind(global).ok()?;
    let id = global.id;
    let serial = global
        .props
        .and_then(|props| props.get("object.serial"))
        .unwrap_or("")
        .to_owned();
    let shared = streams.clone();
    let listener = node
        .add_listener_local()
        .info(move |info| {
            let mut state = shared.borrow_mut();
            let entry = state.streams.entry(id).or_insert_with(|| PlaybackStream {
                id,
                serial: serial.clone(),
                movable: true,
                ..PlaybackStream::default()
            });
            let previous = entry.clone();
            entry.running = matches!(info.state(), pw::node::NodeState::Running);
            // Unchanged properties arrive as an empty dictionary, not as null.
            let props = info
                .props()
                .filter(|_| info.change_mask().contains(pw::node::NodeChangeMask::PROPS));
            if let Some(props) = props {
                let get = |key: &str| props.get(key).map(str::to_owned);
                let flag = |key: &str| props.get(key).is_some_and(|v| matches!(v, "true" | "1"));
                entry.node_name = get("node.name").unwrap_or_default();
                entry.binary = get("application.process.binary");
                entry.flatpak_id = get("pipewire.access.portal.app_id");
                entry.app_id = get("application.id");
                entry.app_name = get("application.name");
                entry.icon_name = get("application.icon-name");
                entry.media_role = get("media.role");
                entry.process_id = props
                    .get("application.process.id")
                    .and_then(|pid| pid.parse().ok());
                entry.movable = !flag("node.dont-move") && !flag("node.dont-reconnect");
            }
            let changed = *entry != previous;
            state.changed |= changed;
        })
        .register();
    Some((node, listener))
}

pub(crate) fn bind_metadata(
    registry: &pw::registry::RegistryRc,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    state: &Rc<RefCell<MetadataState>>,
) -> Option<BoundMetadata> {
    let metadata: pw::metadata::Metadata = registry.bind(global).ok()?;
    let shared = state.clone();
    let listener = metadata
        .add_listener_local()
        .property(move |subject, key, _type, value| {
            let mut state = shared.borrow_mut();
            match key {
                Some("target.object") => {
                    match value {
                        Some(value) => state.targets.insert(subject, value.to_owned()),
                        None => state.targets.remove(&subject),
                    };
                }
                Some("default.audio.sink") if subject == 0 => {
                    let name = value.and_then(json_name);
                    if state.default_sink != name {
                        state.default_sink = name;
                        state.default_changed = true;
                    }
                }
                Some("default.audio.source") if subject == 0 => {
                    let name = value.and_then(json_name);
                    if state.default_source != name {
                        state.default_source = name;
                        state.source_changed = true;
                    }
                }
                None => {
                    state.targets.remove(&subject);
                }
                _ => {}
            }
            0
        })
        .register();
    Some(BoundMetadata {
        metadata,
        _listener: listener,
    })
}

fn json_name(value: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

pub(crate) fn is_output_device(node: &Node) -> bool {
    node.media_class == "Audio/Sink" && !node.name.starts_with("macafon_")
}

struct OutputLink {
    sink_id: u32,
    ports: Vec<u32>,
    _links: Vec<pw::link::Link>,
}

/// Session-lifetime mixer objects; dropped before the connection.
#[derive(Default)]
pub(crate) struct MixerSession {
    channels: Vec<ChannelNode>,
    links: HashMap<String, OutputLink>,
    /// Streams whose target Macafon set, with the value written (for release).
    /// Keyed by stream id with the stream's `object.serial`: ids are reused, serials are not.
    routed: HashMap<u32, (String, String)>,
    outputs: Vec<Node>,
}

impl MixerSession {
    pub fn tick<H: MixerHost + ?Sized>(
        &mut self,
        host: &mut H,
        core: &pw::core::CoreRc,
        graph: &mut Graph,
        streams: &SharedStreams,
        metadata: &Rc<RefCell<MetadataState>>,
        bound: Option<&BoundMetadata>,
    ) {
        for request in host.channel_requests() {
            if let Err(error) = self.handle(core, request) {
                tracing::warn!(%error, "Cannot change channel node");
            }
        }
        if std::mem::take(&mut graph.outputs_changed) {
            self.outputs = graph
                .nodes
                .iter()
                .filter(|node| is_output_device(node))
                .cloned()
                .collect();
            for node in &mut self.outputs {
                crate::enrich_node(node, &graph.devices);
            }
            host.outputs_changed(&self.outputs);
        }
        {
            let mut state = metadata.borrow_mut();
            let sink =
                std::mem::take(&mut state.default_changed).then(|| state.default_sink.clone());
            let source =
                std::mem::take(&mut state.source_changed).then(|| state.default_source.clone());
            drop(state);
            if let Some(name) = sink {
                host.default_sink_changed(name.as_deref());
            }
            if let Some(name) = source {
                host.default_source_changed(name.as_deref());
            }
        }
        let snapshot = {
            let mut state = streams.borrow_mut();
            let targets = &metadata.borrow().targets;
            for stream in state.streams.values_mut() {
                let target = targets.get(&stream.id).cloned();
                if stream.target != target {
                    stream.target = target;
                }
            }
            std::mem::take(&mut state.changed).then(|| {
                let mut list: Vec<PlaybackStream> = state.streams.values().cloned().collect();
                list.sort_by_key(|stream| stream.id);
                list
            })
        };
        if let Some(list) = snapshot {
            self.routed
                .retain(|id, (serial, _)| list.iter().any(|s| s.id == *id && &s.serial == serial));
            host.streams_changed(&list);
        }
        self.link_outputs(core, graph, host.output_device());
        if let Some(bound) = bound {
            self.apply_routes(host, graph, streams, bound);
        }
    }

    fn handle(&mut self, core: &pw::core::CoreRc, request: ChannelRequest) -> Result<(), Error> {
        match request {
            ChannelRequest::Create {
                id,
                description,
                processor,
            } => {
                self.remove(&id);
                let node = ChannelNode::new(core, &id, &description, processor)?;
                tracing::info!(channel = %id, "Channel created");
                self.channels.push(node);
            }
            ChannelRequest::Remove { id } => self.remove(&id),
            ChannelRequest::Describe { id, description } => {
                if let Some(node) = self.channels.iter().find(|node| node.id == id) {
                    node.set_description(&description);
                }
            }
            ChannelRequest::Latency { id, samples } => {
                if let Some(node) = self.channels.iter_mut().find(|node| node.id == id) {
                    node.set_latency(samples)?;
                }
            }
        }
        Ok(())
    }

    fn remove(&mut self, id: &str) {
        self.links.remove(id);
        self.channels.retain(|node| node.id != id);
    }

    /// Keep every channel's outputs linked to the selected device (OUT-001); relink when
    /// the device or a port disappears.
    fn link_outputs(&mut self, core: &pw::core::CoreRc, graph: &Graph, target: Option<String>) {
        let sink = target.as_deref().and_then(|name| {
            graph
                .nodes
                .iter()
                .find(|node| node.name == name && is_output_device(node))
        });
        let present = |id: &u32| graph.ports.iter().any(|port| port.id == *id);
        self.links.retain(|_, link| {
            Some(link.sink_id) == sink.map(|sink| sink.id) && link.ports.iter().all(present)
        });
        let Some(sink) = sink else {
            return;
        };
        for channel in &self.channels {
            if self.links.contains_key(&channel.id) {
                continue;
            }
            let node_id = channel.node_id();
            if node_id == pw::sys::PW_ID_ANY {
                continue;
            }
            match link_pair(core, &graph.ports, node_id, sink.id) {
                Ok(Some(link)) => {
                    tracing::info!(channel = %channel.id, output = %sink.description, "Channel linked");
                    self.links.insert(channel.id.clone(), link);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "Cannot link channel to output"),
            }
        }
    }

    /// Write `target.object` so WirePlumber moves streams without restarting them (APP-004).
    fn apply_routes<H: MixerHost + ?Sized>(
        &mut self,
        host: &H,
        graph: &Graph,
        streams: &SharedStreams,
        bound: &BoundMetadata,
    ) {
        let streams = streams.borrow();
        for stream in streams.streams.values() {
            if stream.node_name.is_empty() || stream.ours() {
                continue;
            }
            match host.route(stream) {
                Route::Channel(id) => {
                    if !stream.movable {
                        continue;
                    }
                    let name = channel_node_name(&id);
                    let Some(node) = graph.nodes.iter().find(|node| node.name == name) else {
                        continue;
                    };
                    if stream.target.as_deref() != Some(node.serial.as_str())
                        && self.routed.get(&stream.id)
                            != Some(&(stream.serial.clone(), node.serial.clone()))
                    {
                        bound.metadata.set_property(
                            stream.id,
                            "target.object",
                            Some("Spa:Id"),
                            Some(&node.serial),
                        );
                        self.routed
                            .insert(stream.id, (stream.serial.clone(), node.serial.clone()));
                    }
                }
                Route::Release => self.release(stream.id, stream.target.as_deref(), bound),
                Route::Ignore => {}
            }
        }
    }

    fn release(&mut self, id: u32, current: Option<&str>, bound: &BoundMetadata) {
        if let Some((_, value)) = self.routed.remove(&id) {
            // A newer choice by another client stays.
            if current.is_none_or(|current| current == value) {
                bound.metadata.set_property(id, "target.object", None, None);
            }
        }
    }

    /// Clear every target Macafon set (mixer off, Quit), before the channels disappear.
    pub fn release_all(&mut self, streams: &SharedStreams, bound: Option<&BoundMetadata>) {
        let Some(bound) = bound else {
            return;
        };
        let routed: Vec<u32> = self.routed.keys().copied().collect();
        let streams = streams.borrow();
        for id in routed {
            let current = streams
                .streams
                .get(&id)
                .and_then(|stream| stream.target.clone());
            self.release(id, current.as_deref(), bound);
        }
    }

    /// Graph clock seen by any running channel.
    pub fn clock(&self) -> Option<(u32, u32)> {
        self.channels
            .iter()
            .map(ChannelNode::clock)
            .find(|(rate, quantum)| *rate != 0 && *quantum != 0)
    }

    pub fn has_channels(&self) -> bool {
        !self.channels.is_empty()
    }
}

/// Link a channel's FL/FR outputs to the sink's playback ports by channel position; mono and
/// AUX sinks get the first ports in order.
fn link_pair(
    core: &pw::core::CoreRc,
    ports: &[Port],
    channel_id: u32,
    sink_id: u32,
) -> Result<Option<OutputLink>, Error> {
    let outputs: Vec<&Port> = ports
        .iter()
        .filter(|port| port.node_id == channel_id && port.direction == "out")
        .collect();
    let inputs: Vec<&Port> = ports
        .iter()
        .filter(|port| port.node_id == sink_id && port.direction == "in")
        .collect();
    fn find<'a>(list: &[&'a Port], channel: &str) -> Option<&'a Port> {
        list.iter().find(|port| port.channel == channel).copied()
    }
    let (Some(out_l), Some(out_r)) = (find(&outputs, "FL"), find(&outputs, "FR")) else {
        return Ok(None);
    };
    let (Some(first), second) = (inputs.first().copied(), inputs.get(1).copied()) else {
        return Ok(None);
    };
    let in_l = find(&inputs, "FL").unwrap_or(first);
    let in_r = find(&inputs, "FR").or(second).unwrap_or(first);
    let mut links = Vec::with_capacity(2);
    for (out, inp) in [(out_l, in_l), (out_r, in_r)] {
        links.push(core.create_object::<pw::link::Link>(
            "link-factory",
            &properties! {
                "link.output.node" => channel_id.to_string(),
                "link.output.port" => out.id.to_string(),
                "link.input.node" => sink_id.to_string(),
                "link.input.port" => inp.id.to_string(),
                "object.linger" => "false",
            },
        )?);
    }
    let mut port_ids: Vec<u32> = [out_l.id, out_r.id, in_l.id, in_r.id].into();
    port_ids.dedup();
    Ok(Some(OutputLink {
        sink_id,
        ports: port_ids,
        _links: links,
    }))
}

/// Registry ids of playback streams that need binding.
pub(crate) fn is_playback_stream(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) -> bool {
    global.type_ == ObjectType::Node
        && global
            .props
            .is_some_and(|props| props.get("media.class") == Some("Stream/Output/Audio"))
}

pub(crate) type StreamBindings = HashMap<u32, (pw::node::Node, pw::node::NodeListener)>;

pub(crate) fn forget_stream(streams: &SharedStreams, bindings: &RefCell<StreamBindings>, id: u32) {
    if bindings.borrow_mut().remove(&id).is_some() {
        let mut state = streams.borrow_mut();
        state.streams.remove(&id);
        state.changed = true;
    }
}
