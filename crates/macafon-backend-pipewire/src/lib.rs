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
pub use inspect::inspect;

pub use routing::route_stream;

pub use macafon_backend::{
    AbRt, ChannelRequest, ChannelRt, ControlSender, DEFAULT_QUANTUM_FRAMES, DriverLoad, Inspection,
    MicRt, MicrophoneHost, MixerHost, Node, PlaybackStream, PlayerRequest, Route, VIRTUAL_MIC_NAME,
    version_at_least,
};
use pipewire as pw;

pub(crate) use connection::Connection;
pub(crate) use graph::{Graph, Port};
pub use nodes::list_nodes;
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
    #[error(transparent)]
    Host(#[from] macafon_backend::Error),
}

impl From<Error> for macafon_backend::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::Host(error) => error,
            Error::Parameter(error) => Self::Parameter(error),
            Error::Engine(error) => Self::Engine(error),
            Error::Message(message) => Self::Message(message),
            error => Self::Platform(error.to_string()),
        }
    }
}

pub struct ControlReceiver<T: 'static>(pw::channel::Receiver<T>);

pub fn control_channel<T: Send + 'static>() -> (ControlSender<T>, ControlReceiver<T>) {
    let (sender, receiver) = pw::channel::channel();
    (
        ControlSender::new(move |message| sender.send(message)),
        ControlReceiver(receiver),
    )
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
