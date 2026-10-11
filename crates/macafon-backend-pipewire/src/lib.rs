//! Linux M0 backend. The only unsafe code bridges the unwrapped pw_filter API.

mod default_source;
mod filter;
pub use default_source::{default_source_journal, restore_default_source, set_default_source};
mod meter;
mod player;
mod routing;

pub use routing::route_stream;

pub use macafon_engine::{AbRt, MicRt};
use pipewire::{self as pw, properties::properties, types::ObjectType};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("PipeWire: {0}")]
    PipeWire(#[from] pw::Error),
    #[error("PipeWire connection lost: {0}")]
    Disconnected(String),
    #[error(transparent)]
    Parameter(#[from] macafon_dsp::ParameterError),
    #[error(transparent)]
    Engine(#[from] macafon_engine::EngineError),
    #[error("{0}")]
    Message(String),
}

pub(crate) struct Connection {
    pub mainloop: pw::main_loop::MainLoopRc,
    pub core: pw::core::CoreRc,
    failure: Rc<RefCell<Option<Error>>>,
    _errors: pw::core::Listener,
}

impl Connection {
    pub fn new() -> Result<Self, Error> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        Self::with_mainloop(mainloop)
    }

    fn with_mainloop(mainloop: pw::main_loop::MainLoopRc) -> Result<Self, Error> {
        let context = pw::context::ContextRc::new(
            &mainloop,
            Some(properties! {
                "cpu.zero.denormals" => "true",
            }),
        )?;
        let core = context
            .connect_rc(None)
            .map_err(|error| Error::Disconnected(error.to_string()))?;
        let failure = Rc::new(RefCell::new(None));
        let errors = failure.clone();
        let loop_ = mainloop.clone();
        let listener = core
            .add_listener_local()
            .error(move |id, _, result, message| {
                // EPIPE/ECONNRESET/ENOTCONN can be reported on an object before the core.
                let error = if id == pw::core::PW_ID_CORE || matches!(result, -32 | -104 | -107) {
                    Error::Disconnected(message.to_owned())
                } else {
                    Error::Message(message.to_owned())
                };
                *errors.borrow_mut() = Some(error);
                loop_.quit();
            })
            .register();
        Ok(Self {
            mainloop,
            core,
            failure,
            _errors: listener,
        })
    }

    pub fn check(&self) -> Result<(), Error> {
        if let Some(error) = self.failure.borrow_mut().take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    pub fn roundtrip(&self) -> Result<(), Error> {
        let pending = self.core.sync(0)?;
        let finished = Rc::new(Cell::new(false));
        let done = finished.clone();
        let loop_ = self.mainloop.clone();
        let _listener = self
            .core
            .add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq == pending {
                    done.set(true);
                    loop_.quit();
                }
            })
            .register();
        let loop_ = self.mainloop.clone();
        let _timeout = self.mainloop.loop_().add_timer(move |_| loop_.quit());
        _timeout
            .update_timer(Some(Duration::from_secs(5)), None)
            .into_result()
            .map_err(|error| Error::Message(error.to_string()))?;
        self.mainloop.run();
        self.check()?;
        if finished.get() {
            Ok(())
        } else {
            Err(Error::Message("PipeWire request timed out".into()))
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: u32,
    pub serial: String,
    pub name: String,
    pub description: String,
    pub media_class: String,
    pub virtual_node: bool,
    pub device_id: Option<u32>,
    pub stable_id: String,
    pub kind: &'static str,
    pub form_factor: String,
}

fn node(global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>) -> Option<Node> {
    if global.type_ != ObjectType::Node {
        return None;
    }
    let props = global.props?;
    Some(Node {
        id: global.id,
        serial: props.get("object.serial")?.to_owned(),
        name: props.get("node.name")?.to_owned(),
        description: props
            .get("node.description")
            .or_else(|| props.get("application.name"))
            .or_else(|| props.get("node.name"))?
            .to_owned(),
        media_class: props.get("media.class").unwrap_or("").to_owned(),
        virtual_node: props.get("node.virtual") == Some("true"),
        device_id: props.get("device.id").and_then(|id| id.parse().ok()),
        stable_id: stable_device_id(
            props.get("device.serial"),
            props
                .get("device.bus-path")
                .or_else(|| props.get("device.bus-id")),
            props.get("node.name")?,
        ),
        form_factor: props.get("device.form-factor").unwrap_or("").to_owned(),
        kind: device_kind(
            props.get("device.bus").unwrap_or(""),
            props.get("device.form-factor").unwrap_or(""),
            props.get("node.name")?,
        ),
    })
}

fn stable_device_id(serial: Option<&str>, bus_path: Option<&str>, name: &str) -> String {
    format!(
        "{}:{name}",
        serial
            .filter(|s| !s.is_empty())
            .or(bus_path)
            .unwrap_or(name)
    )
}

fn device_kind(bus: &str, form_factor: &str, name: &str) -> &'static str {
    if bus == "bluetooth" || name.starts_with("bluez_") {
        "bluetooth"
    } else if matches!(form_factor, "speaker" | "speakers") {
        "speakers"
    } else if matches!(form_factor, "headset" | "headphone") {
        "headset"
    } else if bus == "usb" || name.contains(".usb-") {
        "usb"
    } else if matches!(bus, "pci" | "platform") || form_factor == "internal" {
        "builtin"
    } else {
        "unknown"
    }
}

#[derive(Clone)]
struct Hardware {
    id: u32,
    serial: Option<String>,
    bus_path: Option<String>,
    bus: String,
    form_factor: String,
}

fn hardware(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) -> Option<Hardware> {
    if global.type_ != ObjectType::Device {
        return None;
    }
    let props = global.props?;
    Some(Hardware {
        id: global.id,
        serial: props.get("device.serial").map(str::to_owned),
        bus_path: props
            .get("device.bus-path")
            .or_else(|| props.get("device.bus-id"))
            .map(str::to_owned),
        bus: props.get("device.bus").unwrap_or("").to_owned(),
        form_factor: props.get("device.form-factor").unwrap_or("").to_owned(),
    })
}

fn enrich_node(node: &mut Node, devices: &[Hardware]) {
    if let Some(device) = devices
        .iter()
        .find(|device| Some(device.id) == node.device_id)
    {
        if device.serial.is_some() || device.bus_path.is_some() {
            node.stable_id = stable_device_id(
                device.serial.as_deref(),
                device.bus_path.as_deref(),
                &node.name,
            );
        }
        node.kind = device_kind(&device.bus, &device.form_factor, &node.name);
        if !device.form_factor.is_empty() {
            node.form_factor = device.form_factor.clone();
        }
    }
}

pub fn list_nodes() -> Result<Vec<Node>, Error> {
    let connection = Connection::new()?;
    let registry = connection.core.get_registry()?;
    let nodes = Rc::new(RefCell::new(Vec::new()));
    let collected = nodes.clone();
    let devices = Rc::new(RefCell::new(Vec::new()));
    let hardware_devices = devices.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(device) = hardware(global) {
                hardware_devices.borrow_mut().push(device);
            }
            if let Some(node) = node(global) {
                collected.borrow_mut().push(node);
            }
        })
        .register();
    connection.roundtrip()?;
    let mut result = nodes.borrow().clone();
    for node in &mut result {
        enrich_node(node, &devices.borrow());
    }
    result.sort_by_key(|node| node.id);
    Ok(result)
}

#[derive(Clone)]
struct Port {
    id: u32,
    node_id: u32,
    direction: String,
    channel: String,
}

struct InputLink {
    source: String,
    output: u32,
    input: u32,
    output_right: u32,
    input_right: u32,
    _links: Vec<pw::link::Link>,
}

#[derive(Default)]
struct Graph {
    nodes: Vec<Node>,
    devices: Vec<Hardware>,
    ports: Vec<Port>,
    link: Option<InputLink>,
    inputs_changed: bool,
}

impl Graph {
    fn connect_input(
        &mut self,
        core: &pw::core::CoreRc,
        input_name: &str,
        filter_id: u32,
    ) -> Result<(), Error> {
        if self.link.is_some() {
            return Ok(());
        }
        // Refuse our virtual sources and monitor nodes: an accidental feedback loop is unsafe.
        let Some(source) = self
            .nodes
            .iter()
            .find(|n| n.name == input_name && is_physical_input(n))
        else {
            return Ok(());
        };
        let output = self
            .ports
            .iter()
            .filter(|p| p.node_id == source.id && p.direction == "out")
            .min_by_key(|p| match p.channel.as_str() {
                "MONO" => 0,
                "FL" => 1,
                _ => 2,
            });
        let input = self
            .ports
            .iter()
            .find(|p| p.node_id == filter_id && p.direction == "in" && p.channel == "FL");
        if let (Some(output), Some(input)) = (output, input) {
            let output_right = self
                .ports
                .iter()
                .find(|p| p.node_id == source.id && p.direction == "out" && p.channel == "FR")
                .unwrap_or(output);
            let Some(input_right) = self
                .ports
                .iter()
                .find(|p| p.node_id == filter_id && p.direction == "in" && p.channel == "FR")
            else {
                return Ok(());
            };
            let mut links = Vec::with_capacity(2);
            for (out, inp) in [(output, input), (output_right, input_right)] {
                links.push(core.create_object::<pw::link::Link>(
                    "link-factory",
                    &properties! {
                        "link.output.node" => source.id.to_string(),
                        "link.output.port" => out.id.to_string(),
                        "link.input.node" => filter_id.to_string(),
                        "link.input.port" => inp.id.to_string(),
                        "object.linger" => "false",
                        "link.passive" => "true",
                    },
                )?);
            }
            tracing::info!(source = %source.description, "Microphone connected");
            self.link = Some(InputLink {
                source: input_name.to_owned(),
                output: output.id,
                input: input.id,
                output_right: output_right.id,
                input_right: input_right.id,
                _links: links,
            });
        }
        Ok(())
    }
}

/// Start or stop the A/B playback stream (PRD §6.4).
#[allow(clippy::large_enum_variant)] // one transient value per request
pub enum PlayerRequest {
    /// Replace any running player; `target` is a sink `node.name`, `None` for the default.
    Start {
        player: AbRt,
        target: Option<String>,
    },
    Stop,
}

/// The daemon side of the microphone, driven on the PipeWire control loop.
pub trait MicrophoneHost {
    /// Message type delivered from other threads through [`ControlSender`].
    type Message: 'static;

    /// Called once after SIGINT/SIGTERM are blocked for this thread: helper threads spawned
    /// here inherit the mask, so only the control loop handles shutdown signals.
    fn started(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Build the RT chain for a new graph session: at startup and after a server restart.
    fn start_session(&mut self) -> Result<MicRt, Error>;
    /// The session ended; its RT chain has been dropped.
    fn end_session(&mut self) {}
    /// Physical microphone `node.name` to link; polled every 100 ms.
    fn input(&self) -> Option<String>;
    /// Duplicate the processed mono signal into FL/FR outputs for this graph session.
    fn input_channel(&self) -> macafon_dsp::InputChannel {
        macafon_dsp::InputChannel::Auto
    }
    fn input_channel_detected(&mut self, _channel: macafon_dsp::InputChannel) {}

    fn stereo(&self) -> bool {
        false
    }
    /// Current algorithmic latency to report to the graph; polled every 100 ms.
    fn latency_samples(&self) -> u32;
    /// Run the chain even if nothing records the virtual microphone; polled every 100 ms.
    fn keep_processing(&self) -> bool {
        false
    }
    fn input_connected(&mut self, _connected: bool) {}
    /// Pending A/B playback change; polled every 100 ms.
    fn player_request(&mut self) -> Option<PlayerRequest> {
        None
    }
    /// The A/B stream could not be created.
    fn player_failed(&mut self, _error: Error) {}
    /// Physical inputs (`Audio/Source`, not ours, not monitors) changed.
    fn inputs_changed(&mut self, _inputs: &[Node]) {}
    fn device_metering(&self) -> bool {
        false
    }
    fn device_meter(&mut self, _name: &str, _peak_dbfs: f32) {}
    fn handle(&mut self, message: Self::Message);
    /// Checked after every message: stop the daemon like SIGTERM does (BG-005).
    fn quit_requested(&self) -> bool {
        false
    }
    /// Periodic control-loop housekeeping, every 100 ms.
    fn tick(&mut self) {}
}

/// Thread-safe handle that wakes the control loop with a host message.
pub struct ControlSender<T>(pw::channel::Sender<T>);

impl<T> Clone for ControlSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> ControlSender<T> {
    /// Returns the message back if the control loop has exited.
    pub fn send(&self, message: T) -> Result<(), T> {
        self.0.send(message)
    }
}

pub struct ControlReceiver<T: 'static>(pw::channel::Receiver<T>);

pub fn control_channel<T>() -> (ControlSender<T>, ControlReceiver<T>) {
    let (sender, receiver) = pw::channel::channel();
    (ControlSender(sender), ControlReceiver(receiver))
}

fn is_physical_input(node: &Node) -> bool {
    node.media_class == "Audio/Source"
        && !node.virtual_node
        && !node.name.starts_with("macafon_")
        && !node.name.ends_with(".monitor")
}

/// Run the virtual microphone until SIGINT/SIGTERM, surviving PipeWire restarts.
pub fn run_microphone<H: MicrophoneHost + 'static>(
    host: Rc<RefCell<H>>,
    control: ControlReceiver<H::Message>,
) -> Result<(), Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let stopping = Rc::new(Cell::new(false));
    // Block termination signals before creating data-loop threads, so they inherit
    // the signal mask and only the control loop handles shutdown.
    let loop_ = mainloop.clone();
    let stop = stopping.clone();
    let _interrupt = mainloop
        .loop_()
        .add_signal_local(pw::loop_::Signal::INT, move || {
            stop.set(true);
            loop_.quit();
        });
    let loop_ = mainloop.clone();
    let stop = stopping.clone();
    let _terminate = mainloop
        .loop_()
        .add_signal_local(pw::loop_::Signal::TERM, move || {
            stop.set(true);
            loop_.quit();
        });
    host.borrow_mut().started()?;
    let handler = host.clone();
    let loop_ = mainloop.clone();
    let stop = stopping.clone();
    let _control = control.0.attach(mainloop.loop_(), move |message| {
        let mut host = handler.borrow_mut();
        host.handle(message);
        if host.quit_requested() {
            stop.set(true);
            loop_.quit();
        }
    });
    // Housekeeping belongs to the daemon lifetime, including reconnect waits.
    // This timer runs on the control loop, never on the audio data loop.
    let handler = host.clone();
    let housekeeping = mainloop
        .loop_()
        .add_timer(move |_| handler.borrow_mut().tick());
    housekeeping
        .update_timer(
            Some(Duration::from_millis(50)),
            Some(Duration::from_millis(100)),
        )
        .into_result()
        .map_err(|error| Error::Message(error.to_string()))?;
    let waiting = Rc::new(Cell::new(false));
    while !stopping.get() {
        // Rebuild all DSP and graph storage on the control thread. run_session drops
        // its filter (and joins RT processing) before returning to the retry loop.
        let result = run_session(&mainloop, &host, waiting.clone());
        host.borrow_mut().end_session();
        match result {
            Ok(()) => return Ok(()),
            Err(_) if stopping.get() => return Ok(()),
            Err(Error::Disconnected(message)) => {
                if !waiting.replace(true) {
                    tracing::warn!(error = %message, "PipeWire unavailable; waiting for reconnect");
                }
                let loop_ = mainloop.clone();
                let retry = mainloop.loop_().add_timer(move |_| loop_.quit());
                retry
                    .update_timer(Some(Duration::from_millis(500)), None)
                    .into_result()
                    .map_err(|error| Error::Message(error.to_string()))?;
                // Keep handling signals and control messages while the server is absent.
                mainloop.run();
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn run_session<H: MicrophoneHost + 'static>(
    mainloop: &pw::main_loop::MainLoopRc,
    host: &Rc<RefCell<H>>,
    waiting: Rc<Cell<bool>>,
) -> Result<(), Error> {
    let connection = Connection::with_mainloop(mainloop.clone())?;
    let processor = host.borrow_mut().start_session()?;
    let filter = Rc::new(filter::Filter::new(
        &connection,
        processor,
        host.borrow().stereo(),
    )?);
    let registry = connection.core.get_registry()?;
    let graph = Rc::new(RefCell::new(Graph::default()));
    let graph_add = graph.clone();
    let graph_remove = graph.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            let mut graph = graph_add.borrow_mut();
            if let Some(device) = hardware(global) {
                graph.devices.push(device);
                graph.inputs_changed = true;
            }
            if let Some(node) = node(global) {
                graph.inputs_changed |= is_physical_input(&node);
                graph.nodes.push(node);
            }
            if global.type_ == ObjectType::Port {
                if let Some(props) = global.props {
                    if let Some(node_id) = props.get("node.id").and_then(|id| id.parse().ok()) {
                        graph.ports.push(Port {
                            id: global.id,
                            node_id,
                            direction: props.get("port.direction").unwrap_or("").to_owned(),
                            channel: props.get("audio.channel").unwrap_or("").to_owned(),
                        });
                    }
                }
            }
        })
        .global_remove(move |id| {
            let mut graph = graph_remove.borrow_mut();
            if graph.link.as_ref().is_some_and(|link| {
                link.output == id
                    || link.input == id
                    || link.output_right == id
                    || link.input_right == id
            }) {
                graph.link = None;
            }
            if let Some(node) = graph.nodes.iter().find(|node| node.id == id) {
                graph.inputs_changed |= is_physical_input(node);
            }
            graph.nodes.retain(|n| n.id != id);
            graph.devices.retain(|device| device.id != id);
            graph.ports.retain(|p| p.id != id);
        })
        .register();
    let loop_ = connection.mainloop.clone();
    let core = connection.core.clone();
    let session_filter = filter.clone();
    let graph_rate_hz = filter.graph_rate_hz.clone();
    let failure = connection.failure.clone();
    let host = host.clone();
    let ready = Cell::new(false);
    let reported_latency = Cell::new(host.borrow().latency_samples());

    let connected = Cell::new(None);
    // Dropped with the timer, before the connection.
    let player = RefCell::new(None);
    let meters = RefCell::new(Vec::<(String, meter::Meter)>::new());
    let timer = connection.mainloop.loop_().add_timer(move |_| {
        let filter = &session_filter;
        let rate_hz = graph_rate_hz.load(std::sync::atomic::Ordering::Relaxed);
        if rate_hz != 0 && rate_hz != macafon_dsp::SAMPLE_RATE_HZ {
            *failure.borrow_mut() = Some(Error::Message(format!(
                "Macafon requires a 48000 Hz graph, got {rate_hz} Hz"
            )));
            loop_.quit();
            return;
        }
        // SAFETY: the timer runs on the control loop and is dropped before the filter.
        let state = unsafe { pw::sys::pw_filter_get_state(filter.pointer(), std::ptr::null_mut()) };
        if state == pw::sys::pw_filter_state_PW_FILTER_STATE_ERROR {
            *failure.borrow_mut() = Some(Error::Message(
                "Virtual microphone entered an error state".into(),
            ));
            loop_.quit();
            return;
        }
        if matches!(
            state,
            pw::sys::pw_filter_state_PW_FILTER_STATE_PAUSED
                | pw::sys::pw_filter_state_PW_FILTER_STATE_STREAMING
        ) && !ready.replace(true)
        {
            if waiting.replace(false) {
                tracing::info!("PipeWire reconnected; virtual microphone restored");
            } else {
                tracing::info!("Macafon Microphone running; Ctrl+C to stop");
            }
        }
        let mut host = host.borrow_mut();
        match host.player_request() {
            Some(PlayerRequest::Start { player: rt, target }) => {
                player.replace(None);
                match player::Player::new(&core, rt, target.as_deref()) {
                    Ok(started) => {
                        player.replace(Some(started));
                    }
                    Err(error) => host.player_failed(error),
                }
            }
            Some(PlayerRequest::Stop) => {
                player.replace(None);
            }
            None => {}
        }
        let latency = host.latency_samples();
        if latency != reported_latency.get() {
            match filter.set_latency(latency) {
                Ok(()) => reported_latency.set(latency),
                Err(error) => tracing::warn!(%error, "Cannot report latency"),
            }
        }
        filter.set_keep_processing(host.keep_processing());
        filter.set_input_channel(host.input_channel());
        let detected = filter
            .detected_channel
            .load(std::sync::atomic::Ordering::Relaxed);
        host.input_channel_detected(if detected == 2 {
            macafon_dsp::InputChannel::Right
        } else {
            macafon_dsp::InputChannel::Left
        });
        let mut graph = graph.borrow_mut();
        if std::mem::take(&mut graph.inputs_changed) {
            let mut inputs: Vec<Node> = graph
                .nodes
                .iter()
                .filter(|n| is_physical_input(n))
                .cloned()
                .collect();
            for node in &mut inputs {
                enrich_node(node, &graph.devices);
            }
            host.inputs_changed(&inputs);
        }
        let mut meters = meters.borrow_mut();
        if host.device_metering() {
            meters.retain(|(name, _)| {
                graph
                    .nodes
                    .iter()
                    .any(|node| &node.name == name && is_physical_input(node))
            });
            for node in graph.nodes.iter().filter(|node| is_physical_input(node)) {
                if !meters.iter().any(|(name, _)| name == &node.name) {
                    match meter::Meter::new(&core, &node.name) {
                        Ok(meter) => meters.push((node.name.clone(), meter)),
                        Err(error) => tracing::warn!(%error, "Cannot meter physical input"),
                    }
                }
            }
            for (name, meter) in meters.iter_mut() {
                if let Some(source) = graph.nodes.iter().find(|node| &node.name == name) {
                    if let Err(error) = meter.connect_source(&core, source, &graph.ports) {
                        tracing::warn!(%error, "Cannot link input meter");
                    }
                }
                let peak = meter.read_peak();
                host.device_meter(
                    name,
                    if peak > 0.0 {
                        20.0 * peak.log10()
                    } else {
                        -120.0
                    },
                );
            }
        } else {
            meters.clear();
        }
        let input = host.input();
        if graph
            .link
            .as_ref()
            .is_some_and(|link| Some(&link.source) != input.as_ref())
        {
            graph.link = None;
        }
        if let Some(input_name) = &input {
            // SAFETY: the filter remains alive until after the timer is dropped.
            let id = unsafe { pw::sys::pw_filter_get_node_id(filter.pointer()) };
            if let Err(error) = graph.connect_input(&core, input_name, id) {
                *failure.borrow_mut() = Some(error);
                loop_.quit();
            }
        }
        let now_connected = graph.link.is_some();
        if connected.replace(Some(now_connected)) != Some(now_connected)
            && !now_connected
            && input.is_some()
            && ready.get()
        {
            tracing::warn!("Microphone disconnected; producing silence");
        }
        host.input_connected(now_connected);
    });
    timer
        .update_timer(
            Some(Duration::from_millis(50)),
            Some(Duration::from_millis(100)),
        )
        .into_result()
        .map_err(|error| Error::Message(error.to_string()))?;
    connection.mainloop.run();
    drop(timer);
    connection.check()
}

#[cfg(test)]
mod input_identity_tests {
    use super::*;
    #[test]
    fn device_identity_and_kind_use_hardware_properties() {
        assert_eq!(
            stable_device_id(Some("serial123"), Some("usb-1"), "mic"),
            "serial123:mic"
        );
        assert_eq!(stable_device_id(None, Some("usb-1"), "mic"), "usb-1:mic");
        assert_eq!(device_kind("usb", "headset", "mic"), "headset");
        assert_eq!(device_kind("bluetooth", "headset", "mic"), "bluetooth");
        assert_eq!(device_kind("pci", "", "mic"), "builtin");
    }
}
