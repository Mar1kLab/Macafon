//! Linux M0 backend. The only unsafe code bridges the unwrapped pw_filter API.

mod filter;
mod routing;

pub use routing::route_stream;

use miceq_dsp::MicrophoneProcessor;
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
    Parameter(#[from] miceq_dsp::ParameterError),
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
    })
}

pub fn list_nodes() -> Result<Vec<Node>, Error> {
    let connection = Connection::new()?;
    let registry = connection.core.get_registry()?;
    let nodes = Rc::new(RefCell::new(Vec::new()));
    let collected = nodes.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(node) = node(global) {
                collected.borrow_mut().push(node);
            }
        })
        .register();
    connection.roundtrip()?;
    let mut result = nodes.borrow().clone();
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

#[derive(Default)]
struct Graph {
    nodes: Vec<Node>,
    ports: Vec<Port>,
    link: Option<(u32, u32, pw::link::Link)>,
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
        let Some(source) = self.nodes.iter().find(|n| {
            n.name == input_name
                && n.media_class == "Audio/Source"
                && !n.name.starts_with("miceq_")
                && !n.name.ends_with(".monitor")
        }) else {
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
            .find(|p| p.node_id == filter_id && p.direction == "in");
        if let (Some(output), Some(input)) = (output, input) {
            let link = core.create_object::<pw::link::Link>(
                "link-factory",
                &properties! {
                    "link.output.node" => source.id.to_string(),
                    "link.output.port" => output.id.to_string(),
                    "link.input.node" => filter_id.to_string(),
                    "link.input.port" => input.id.to_string(),
                    "object.linger" => "false",
                    "link.passive" => "true",
                },
            )?;
            tracing::info!(source = %source.description, "Microphone connected");
            self.link = Some((output.id, input.id, link));
        }
        Ok(())
    }
}

pub fn run_microphone(
    prepare: impl Fn() -> Result<MicrophoneProcessor, miceq_dsp::ParameterError>,
    input_name: Option<String>,
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
    let waiting = Rc::new(Cell::new(false));
    while !stopping.get() {
        // Rebuild all DSP and graph storage on the control thread. run_session drops
        // its filter (and joins RT processing) before returning to the retry loop.
        match run_session(&mainloop, &prepare, input_name.clone(), waiting.clone()) {
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
                // Keep handling SIGINT/SIGTERM while the server is absent.
                mainloop.run();
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn run_session(
    mainloop: &pw::main_loop::MainLoopRc,
    prepare: &impl Fn() -> Result<MicrophoneProcessor, miceq_dsp::ParameterError>,
    input_name: Option<String>,
    waiting: Rc<Cell<bool>>,
) -> Result<(), Error> {
    let connection = Connection::with_mainloop(mainloop.clone())?;
    let processor = prepare()?;
    let filter = filter::Filter::new(&connection, processor)?;
    let registry = connection.core.get_registry()?;
    let graph = Rc::new(RefCell::new(Graph::default()));
    let graph_add = graph.clone();
    let graph_remove = graph.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            let mut graph = graph_add.borrow_mut();
            if let Some(node) = node(global) {
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
            if graph
                .link
                .as_ref()
                .is_some_and(|(output, input, _)| *output == id || *input == id)
            {
                graph.link = None;
            }
            graph.nodes.retain(|n| n.id != id);
            graph.ports.retain(|p| p.id != id);
        })
        .register();
    let loop_ = connection.mainloop.clone();
    let core = connection.core.clone();
    let pointer = filter.pointer();
    let graph_rate_hz = filter.graph_rate_hz.clone();
    let failure = connection.failure.clone();
    let ready = Cell::new(false);
    let timer = connection.mainloop.loop_().add_timer(move |_| {
        let rate_hz = graph_rate_hz.load(std::sync::atomic::Ordering::Relaxed);
        if rate_hz != 0 && rate_hz != miceq_dsp::SAMPLE_RATE_HZ {
            *failure.borrow_mut() = Some(Error::Message(format!(
                "M0 requires a 48000 Hz graph, got {rate_hz} Hz"
            )));
            loop_.quit();
            return;
        }
        // SAFETY: the timer runs on the control loop and is dropped before the filter.
        let state = unsafe { pw::sys::pw_filter_get_state(pointer, std::ptr::null_mut()) };
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
                tracing::info!("micEQ Microphone running; Ctrl+C to stop");
            }
        }
        if let Some(input_name) = &input_name {
            // SAFETY: the filter remains alive until after the timer is dropped.
            let id = unsafe { pw::sys::pw_filter_get_node_id(pointer) };
            if let Err(error) = graph.borrow_mut().connect_input(&core, input_name, id) {
                *failure.borrow_mut() = Some(error);
                loop_.quit();
            }
        }
    });
    timer
        .update_timer(
            Some(Duration::from_millis(50)),
            Some(Duration::from_millis(100)),
        )
        .into_result()
        .map_err(|error| Error::Message(error.to_string()))?;
    connection.mainloop.run();
    connection.check()
}
