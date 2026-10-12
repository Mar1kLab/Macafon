//! Linux microphone backend; unsafe code bridges PipeWire control and audio FFI.

mod channel_node;
mod connection;
mod default_source;
mod filter;
mod graph;
mod inspect;
mod mixer;
mod nodes;
mod session;
pub use default_source::{
    default_sink_journal, default_source_journal, previous_default_sink, restore_default_sink,
    restore_default_source, set_default_sink, set_default_source,
};
mod meter;
mod player;
mod profiler;
mod routing;

pub use channel_node::{CHANNEL_PREFIX, channel_node_name};
pub use inspect::{Inspection, inspect, version_at_least};
/// `node.latency` every Macafon node is created with: 128 frames = 2.7 ms (§9.5).
pub const DEFAULT_QUANTUM_FRAMES: u32 = 128;

/// `node.name` of the virtual microphone (MIC-010).
pub const VIRTUAL_MIC_NAME: &str = "macafon_mic";

pub use mixer::{ChannelRequest, MixerHost, PlaybackStream, Route};
pub use profiler::DriverLoad;
pub use routing::route_stream;

pub use macafon_engine::{AbRt, ChannelRt, MicRt};
use pipewire as pw;

pub(crate) use connection::Connection;
pub(crate) use graph::{Graph, Port};
pub use nodes::{Node, list_nodes};
pub(crate) use nodes::{enrich_node, node};
use session::run_session;
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

/// The daemon side of the microphone and the mixer, driven on the PipeWire control loop.
pub trait MicrophoneHost: MixerHost {
    /// Message type delivered from other threads through [`ControlSender`].
    type Message: 'static;

    /// Called once after SIGINT/SIGTERM are blocked for this thread: helper threads spawned
    /// here inherit the mask, so only the control loop handles shutdown signals.
    fn started(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Subscribe to graph cycle timing only while Studio needs overload detection.
    fn needs_processing_budget(&self) -> bool {
        false
    }
    /// Completed own-node cycle measurements delivered on the control loop.
    /// A zero duration resets the streak at subscription start or a sampling gap.
    fn processing_budget(&mut self, _elapsed_ns: u64, _budget_ns: u64) {}

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
    /// Graph rate and quantum seen by the microphone node; polled every 100 ms.
    fn graph_clock(&mut self, _rate_hz: u32, _quantum: u32) {}
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
