//! The contract between the daemon and a platform audio backend (PRD §11.2): the host traits
//! a backend drives on its control loop and the device and stream types it reports.
//!
//! Linux (`macafon-backend-pipewire`) drives every method. Windows (`macafon-backend-windows`)
//! has no graph and no channel nodes (§12.2): it drives the microphone part and leaves the
//! channel-node and stream-routing methods at their defaults.

pub use macafon_engine::{AbRt, ChannelRt, MicRt};
use std::sync::Arc;

/// `node.latency` every Macafon node is created with: 128 frames = 2.7 ms (§9.5).
pub const DEFAULT_QUANTUM_FRAMES: u32 = 128;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Parameter(#[from] macafon_dsp::ParameterError),
    #[error(transparent)]
    Engine(#[from] macafon_engine::EngineError),
    /// A platform audio API failed.
    #[error("{0}")]
    Platform(String),
    #[error("{0}")]
    Message(String),
}

/// A physical audio device as the backend reports it.
#[derive(Debug, Clone)]
pub struct Node {
    /// Backend-local object id; 0 where the platform has none.
    pub id: u32,
    pub serial: String,
    /// Stable selection key: PipeWire `node.name`, Windows endpoint id.
    pub name: String,
    pub description: String,
    pub media_class: String,
    pub virtual_node: bool,
    pub device_id: Option<u32>,
    /// Hardware identity that survives reconnects and port changes (device profiles).
    pub stable_id: String,
    /// `usb`, `builtin`, `headset`, `bluetooth` or `unknown`.
    pub kind: &'static str,
    pub form_factor: String,
}

/// Start or stop the A/B playback stream (PRD §6.4).
#[allow(clippy::large_enum_variant)] // one transient value per request
pub enum PlayerRequest {
    /// Replace any running player; `target` is an output device name, `None` for the default.
    Start {
        player: AbRt,
        target: Option<String>,
    },
    Stop,
}

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
    /// Real output devices (not ours) changed (OUT-001, OUT-004).
    fn outputs_changed(&mut self, _outputs: &[Node]) {}
    /// The default output changed; `None` when unset.
    fn default_sink_changed(&mut self, _name: Option<&str>) {}
    /// The default microphone changed; `None` when unset (MIC-012).
    fn default_source_changed(&mut self, _name: Option<&str>) {}
    /// Physical output the channels play into, polled every 100 ms.
    fn output_device(&self) -> Option<String> {
        None
    }
    /// Playback streams appeared, changed or vanished (APP-001).
    fn streams_changed(&mut self, _streams: &[PlaybackStream]) {}
    /// Preferred quantum for Macafon's nodes in frames (§9.5); polled every 100 ms.
    fn quantum_frames(&self) -> u32 {
        DEFAULT_QUANTUM_FRAMES
    }
    /// Linear gain Macafon applies to `stream` on top of the app's own volume (CH-008);
    /// consulted every 100 ms for each stream.
    fn stream_volume(&self, _stream: &PlaybackStream) -> f32 {
        1.0
    }
    /// Where `stream` belongs; consulted every 100 ms for each stream.
    fn route(&self, _stream: &PlaybackStream) -> Route {
        Route::Ignore
    }
}

/// The daemon side of the microphone and the mixer, driven on the backend's control loop.
pub trait MicrophoneHost: MixerHost {
    /// Message type delivered from other threads through [`ControlSender`].
    type Message: 'static;

    /// Called once before the first session. On Linux SIGINT/SIGTERM are already blocked for
    /// this thread, so helper threads spawned here inherit the mask.
    fn started(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Subscribe to cycle timing only while Studio needs overload detection.
    fn needs_processing_budget(&self) -> bool {
        false
    }
    /// Completed own-node cycle measurements delivered on the control loop.
    /// A zero duration resets the streak at subscription start or a sampling gap.
    fn processing_budget(&mut self, _elapsed_ns: u64, _budget_ns: u64) {}

    /// Build the RT chain for a new session: at startup and after the audio server restarts.
    fn start_session(&mut self) -> Result<MicRt, Error>;
    /// The session ended; its RT chain has been dropped.
    fn end_session(&mut self) {}
    /// Physical microphone ([`Node::name`]) to use; polled every 100 ms.
    fn input(&self) -> Option<String>;
    fn input_channel(&self) -> macafon_dsp::InputChannel {
        macafon_dsp::InputChannel::Auto
    }
    fn input_channel_detected(&mut self, _channel: macafon_dsp::InputChannel) {}
    /// Duplicate the processed mono signal into left/right outputs for this session.
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
    /// Physical inputs (not ours, not monitors) changed.
    fn inputs_changed(&mut self, _inputs: &[Node]) {}
    fn device_metering(&self) -> bool {
        false
    }
    fn device_meter(&mut self, _name: &str, _peak_dbfs: f32) {}
    /// Rate and quantum seen by the microphone; polled every 100 ms.
    fn graph_clock(&mut self, _rate_hz: u32, _quantum: u32) {}
    fn handle(&mut self, message: Self::Message);
    /// Checked after every message: stop the daemon like SIGTERM does (BG-005).
    fn quit_requested(&self) -> bool {
        false
    }
    /// Periodic control-loop housekeeping, every 100 ms.
    fn tick(&mut self) {}
}

type SendFn<T> = dyn Fn(T) -> Result<(), T> + Send + Sync;

/// Thread-safe handle that wakes the control loop with a host message.
pub struct ControlSender<T>(Arc<SendFn<T>>);

impl<T> Clone for ControlSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> ControlSender<T> {
    /// Wrap a backend's wake-up channel; `send` returns the message if the loop has exited.
    pub fn new(send: impl Fn(T) -> Result<(), T> + Send + Sync + 'static) -> Self {
        Self(Arc::new(send))
    }

    /// Returns the message back if the control loop has exited.
    pub fn send(&self, message: T) -> Result<(), T> {
        (self.0)(message)
    }
}
