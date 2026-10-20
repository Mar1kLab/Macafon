//! The daemon's control loop on Windows (PRD §12.2.1, M3.1): drives a [`MicrophoneHost`] like
//! the PipeWire backend does on Linux, with the microphone path of [`MicrophonePipeline`].
//!
//! There is no graph here: the loop polls the host every 100 ms, rescans endpoints every
//! second, keeps the pipeline pointed at the selected microphone and the VB-CABLE, and starts
//! the A/B player. Channel-node and stream-routing methods of the host are not used (§12.2.2).

use crate::com::ComGuard;
use crate::devices::{DataFlow, Endpoint, EndpointRole, VirtualCable, endpoints, find_vb_cable};
use crate::groups::VolumeGroups;
use crate::journal::{self, SystemDefaults};
use crate::microphone::{MicrophoneConfig, MicrophonePipeline};
use crate::monitor::Monitor;
use crate::player::Player;
use macafon_backend::{ControlSender, Error, MicrophoneHost, Node, PlayerRequest, SoundpadOutput};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{Handset, Headphones, Headset, Speakers};
use windows::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    SetConsoleCtrlHandler,
};
use windows::core::BOOL;

const TICK: Duration = Duration::from_millis(100);
const SCAN_INTERVAL: Duration = Duration::from_secs(1);
const RESTART_DELAY: Duration = Duration::from_millis(500);
/// Kept above the minimum safe drift level (render buffer + one capture period); see
/// `docs/development/windows-m3.md`.
const LATENCY_MARGIN_MS: f64 = 2.0;
/// How long a console close / logoff / shutdown waits for the restore before Windows ends the
/// process anyway (the system allows about 5 s).
const SHUTDOWN_GRACE: Duration = Duration::from_secs(4);

#[path = "session_shutdown.rs"]
mod session_shutdown;

static STOP: AtomicBool = AtomicBool::new(false);
static FINISHED: AtomicBool = AtomicBool::new(false);

pub struct ControlReceiver<T>(mpsc::Receiver<T>);

/// The host's message channel; the loop wakes on every message.
pub fn control_channel<T: Send + 'static>() -> (ControlSender<T>, ControlReceiver<T>) {
    let (sender, receiver) = mpsc::channel();
    (
        ControlSender::new(move |message| sender.send(message).map_err(|error| error.0)),
        ControlReceiver(receiver),
    )
}

fn platform(error: crate::Error) -> Error {
    Error::Platform(error.to_string())
}

/// Run the microphone until Ctrl+C, console close, logoff or shutdown, or until the host asks
/// to quit. The pipeline survives device loss on its own; the session is rebuilt only if an
/// audio thread stops on an unrecoverable error. `cleanup` runs after the audio stops and
/// before a console close, logoff or shutdown is allowed to end the process. Restore journals
/// live in `state_dir`.
pub fn run_microphone<H: MicrophoneHost>(
    host: Rc<RefCell<H>>,
    control: ControlReceiver<H::Message>,
    state_dir: &Path,
    cleanup: impl FnOnce(),
) -> Result<(), Error> {
    STOP.store(false, Ordering::Release);
    FINISHED.store(false, Ordering::Release);
    let _shutdown = session_shutdown::Listener::spawn()
        .map_err(|error| Error::Platform(format!("Cannot listen for session shutdown: {error}")))?;
    let _com = ComGuard::new().map_err(platform)?;
    // SAFETY: registers a plain function; it only touches atomics.
    if let Err(error) = unsafe { SetConsoleCtrlHandler(Some(console_handler), true) } {
        tracing::warn!(%error, "Cannot handle console shutdown signals");
    }
    let started = host.borrow_mut().started();
    if let Err(error) = started {
        cleanup();
        FINISHED.store(true, Ordering::Release);
        return Err(error);
    }
    let mut state = LoopState {
        groups: Some(VolumeGroups::start(state_dir)),
        ..LoopState::default()
    };
    let result = state.run(&host, &control);
    state.player = None;
    state.monitor = None;
    if let Some(groups) = &mut state.groups {
        groups.stop();
    }
    if let Some(pipeline) = state.pipeline.take() {
        if let Err(error) = pipeline.stop() {
            tracing::warn!(%error, "Audio thread failed while stopping");
        }
        host.borrow_mut().end_session();
    }
    cleanup();
    FINISHED.store(true, Ordering::Release);
    result
}

unsafe extern "system" fn console_handler(event: u32) -> BOOL {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            STOP.store(true, Ordering::Release);
            BOOL::from(true)
        }
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            STOP.store(true, Ordering::Release);
            // Returning ends the process: give the loop time to restore system defaults.
            wait_for_cleanup();
            BOOL::from(true)
        }
        _ => BOOL::from(false),
    }
}

#[derive(Default)]
struct LoopState {
    pipeline: Option<MicrophonePipeline>,
    player: Option<Player>,
    restart_at: Option<Instant>,
    scanned_at: Option<Instant>,
    endpoints: Vec<Endpoint>,
    cable: Option<VirtualCable>,
    inputs: Option<Vec<String>>,
    outputs: Option<Vec<String>>,
    default_capture: Option<Option<String>>,
    default_render: Option<Option<String>>,
    input: Option<String>,
    clock_reported_at: Option<Instant>,
    groups: Option<VolumeGroups>,
    /// Soundpad monitor stream and the endpoint it plays into (§7.6).
    monitor: Option<(String, Monitor)>,
    monitor_retry_at: Option<Instant>,
}

impl LoopState {
    fn run<H: MicrophoneHost>(
        &mut self,
        host: &Rc<RefCell<H>>,
        control: &ControlReceiver<H::Message>,
    ) -> Result<(), Error> {
        let mut next_tick = Instant::now();
        while !STOP.load(Ordering::Acquire) {
            let wait = next_tick.saturating_duration_since(Instant::now());
            match control.0.recv_timeout(wait) {
                Ok(message) => {
                    let mut host = host.borrow_mut();
                    host.handle(message);
                    if host.quit_requested() {
                        break;
                    }
                    if host.take_mix_changed() {
                        if let Some(groups) = &mut self.groups {
                            groups.apply_mix(&mut *host);
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            next_tick = Instant::now() + TICK;
            self.tick(&mut *host.borrow_mut())?;
        }
        Ok(())
    }

    fn tick<H: MicrophoneHost>(&mut self, host: &mut H) -> Result<(), Error> {
        if self
            .scanned_at
            .is_none_or(|at| at.elapsed() >= SCAN_INTERVAL)
        {
            self.scanned_at = Some(Instant::now());
            match endpoints() {
                Ok(endpoints) => self.scan(host, endpoints),
                Err(error) => tracing::debug!(%error, "Cannot list audio endpoints"),
            }
        }
        if self
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| !pipeline.is_running())
        {
            if let Some(pipeline) = self.pipeline.take() {
                if let Err(error) = pipeline.stop() {
                    tracing::warn!(%error, "Microphone stopped; restarting");
                }
            }
            host.end_session();
            self.restart_at = Some(Instant::now() + RESTART_DELAY);
        }
        if self.pipeline.is_none() && self.restart_at.is_none_or(|at| Instant::now() >= at) {
            self.start_session(host)?;
        }
        if let Some(pipeline) = &mut self.pipeline {
            // NFR-S3: the chain's time per capture packet, like the PipeWire profiler.
            let measure = host.needs_processing_budget();
            pipeline.set_budget_monitoring(measure);
            if measure {
                pipeline.drain_budget(|sample| {
                    host.processing_budget(sample.elapsed_ns, sample.budget_ns);
                });
            }
        }
        if let Some(pipeline) = &self.pipeline {
            let wanted = host.input().filter(|id| self.is_input(id));
            if wanted != self.input {
                pipeline.set_input(wanted.clone());
                self.input = wanted;
            }
            host.input_connected(self.input.is_some() && pipeline.input_connected());
            if self
                .clock_reported_at
                .is_none_or(|at| at.elapsed() >= SCAN_INTERVAL)
            {
                self.clock_reported_at = Some(Instant::now());
                let period_ms = pipeline.capture_period_ms();
                let rate = macafon_dsp::SAMPLE_RATE_HZ;
                host.graph_clock(rate, (period_ms * f64::from(rate) / 1000.0).round() as u32);
            }
        }
        // No channel nodes on Windows: drop the DSP the host built for them.
        drop(host.channel_requests());
        if let Some(groups) = &mut self.groups {
            groups.tick(host);
        }
        if let Some(request) = host.player_request() {
            self.player = None;
            if let PlayerRequest::Start { player, target } = request {
                let target = target.or_else(|| {
                    crate::devices::default_device(DataFlow::Render, EndpointRole::Console)
                        .ok()
                        .flatten()
                });
                match target {
                    Some(target) => match Player::start(target, player) {
                        Ok(player) => self.player = Some(player),
                        Err(error) => host.player_failed(platform(error)),
                    },
                    None => host.player_failed(Error::Message("no output device".into())),
                }
            }
        }
        self.soundpad_monitor(host);
        host.tick();
        Ok(())
    }

    /// Keep the soundpad monitor on the output the host wants: reopened when the device or
    /// the default output changes or the stream fails, retried every second.
    fn soundpad_monitor<H: MicrophoneHost>(&mut self, host: &mut H) {
        let target = host.soundpad_monitor().and_then(|device| {
            device
                .filter(|id| {
                    self.endpoints
                        .iter()
                        .any(|e| &e.id == id && is_physical(e, DataFlow::Render))
                })
                .or_else(|| self.default_render.clone().flatten())
                .filter(|id| {
                    !self
                        .cable
                        .as_ref()
                        .is_some_and(|cable| &cable.input.id == id)
                })
        });
        let current = self
            .monitor
            .as_ref()
            .filter(|(_, monitor)| monitor.is_running())
            .map(|(id, _)| id);
        if current == target.as_ref() {
            return;
        }
        if self.monitor.take().is_some() {
            self.monitor_retry_at = None;
        }
        let Some(target) = target else { return };
        if self.monitor_retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        let Some(tap) = host.soundpad_tap(SoundpadOutput::Monitor) else {
            return;
        };
        match Monitor::start(target.clone(), tap) {
            Ok(monitor) => {
                self.monitor = Some((target, monitor));
                self.monitor_retry_at = None;
            }
            Err(error) => {
                tracing::warn!(%error, "Cannot open the SoundDeck monitor");
                self.monitor_retry_at = Some(Instant::now() + SCAN_INTERVAL);
            }
        }
    }

    fn start_session<H: MicrophoneHost>(&mut self, host: &mut H) -> Result<(), Error> {
        let processor = match host.start_session() {
            Ok(processor) => processor,
            Err(error) => {
                host.end_session();
                return Err(error);
            }
        };
        self.input = host.input().filter(|id| self.is_input(id));
        let config = MicrophoneConfig {
            input_id: self.input.clone(),
            output_id: self.cable.as_ref().map(|cable| cable.input.id.clone()),
            latency_margin_ms: LATENCY_MARGIN_MS,
            stream_buffer_ms: 0.0,
            low_latency_render: true,
        };
        let soundpad = host.soundpad_tap(SoundpadOutput::Microphone);
        match MicrophonePipeline::start(&config, processor, soundpad) {
            Ok(pipeline) => {
                tracing::info!(
                    cable = self.cable.is_some(),
                    input = self.input.as_deref().unwrap_or("none"),
                    "Microphone started"
                );
                self.pipeline = Some(pipeline);
                self.restart_at = None;
            }
            Err(error) => {
                tracing::warn!(%error, "Cannot start the microphone; retrying");
                host.end_session();
                self.restart_at = Some(Instant::now() + RESTART_DELAY);
            }
        }
        Ok(())
    }

    /// A capture endpoint the user may pick: not the cable (WIN-MIC-009).
    fn is_input(&self, id: &str) -> bool {
        self.endpoints
            .iter()
            .any(|endpoint| endpoint.id == id && is_physical(endpoint, DataFlow::Capture))
    }

    fn scan<H: MicrophoneHost>(&mut self, host: &mut H, endpoints: Vec<Endpoint>) {
        let cable = find_vb_cable(&endpoints);
        let cable_input = cable.as_ref().map(|cable| cable.input.id.clone());
        if cable_input != self.cable.as_ref().map(|cable| cable.input.id.clone()) {
            match &cable_input {
                Some(_) => tracing::info!("VB-CABLE found"),
                None => tracing::warn!("VB-CABLE not found; processing without output"),
            }
            if let Some(pipeline) = &self.pipeline {
                pipeline.set_output(cable_input.clone());
            }
            host.virtual_mic_changed(cable_input.is_some());
        }
        self.cable = cable;
        self.endpoints = endpoints;
        let nodes = |flow| -> Vec<Node> {
            self.endpoints
                .iter()
                .filter(|endpoint| is_physical(endpoint, flow))
                .map(node)
                .collect()
        };
        let (inputs, outputs) = (nodes(DataFlow::Capture), nodes(DataFlow::Render));
        let names = |nodes: &[Node]| nodes.iter().map(|node| node.name.clone()).collect();
        if self.inputs.as_ref() != Some(&names(&inputs)) {
            self.inputs = Some(names(&inputs));
            host.inputs_changed(&inputs);
        }
        if self.outputs.as_ref() != Some(&names(&outputs)) {
            self.outputs = Some(names(&outputs));
            host.outputs_changed(&outputs);
        }
        let capture = crate::devices::default_device(DataFlow::Capture, EndpointRole::Console)
            .ok()
            .flatten()
            .map(|id| match &self.cable {
                // The host knows the cable as Macafon's virtual microphone (MIC-012).
                Some(cable) if cable.output.id == id => {
                    macafon_backend::VIRTUAL_MIC_NAME.to_owned()
                }
                _ => id,
            });
        if self.default_capture.as_ref() != Some(&capture) {
            host.default_source_changed(capture.as_deref());
            self.default_capture = Some(capture);
        }
        if let Some(groups) = &mut self.groups {
            groups.scan_outputs(host, &self.endpoints);
        }
        let render = crate::devices::default_device(DataFlow::Render, EndpointRole::Console)
            .ok()
            .flatten();
        if self.default_render.as_ref() != Some(&render) {
            host.default_sink_changed(render.as_deref());
            self.default_render = Some(render);
        }
    }
}

/// Real devices only: the cable is Macafon's own (WIN-MIC-009).
fn is_physical(endpoint: &Endpoint, flow: DataFlow) -> bool {
    endpoint.flow == flow && !endpoint.is_vb_cable()
}

fn node(endpoint: &Endpoint) -> Node {
    let adapter = endpoint
        .adapter_instance_id
        .as_deref()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let is = |factor: windows::Win32::Media::Audio::EndpointFormFactor| {
        endpoint.form_factor == Some(factor.0 as u32)
    };
    let headset = is(Headset) || is(Handset);
    let kind = if adapter.starts_with("BTH") {
        "bluetooth"
    } else if headset {
        "headset"
    } else if adapter.starts_with("USB\\") {
        "usb"
    } else if adapter.starts_with("HDAUDIO\\") || adapter.starts_with("INTELAUDIO\\") {
        "builtin"
    } else {
        "unknown"
    };
    Node {
        id: 0,
        serial: String::new(),
        name: endpoint.id.clone(),
        description: endpoint.name.clone(),
        media_class: match endpoint.flow {
            DataFlow::Capture => "Audio/Source".into(),
            DataFlow::Render => "Audio/Sink".into(),
        },
        virtual_node: false,
        device_id: None,
        // MMDevice IDs survive reboots and reconnects to the same port.
        stable_id: endpoint.id.clone(),
        kind,
        form_factor: if is(Headphones) {
            "headphone"
        } else if headset {
            "headset"
        } else if is(Speakers) {
            "speaker"
        } else {
            ""
        }
        .into(),
    }
}

/// Make `CABLE Output` the default microphone for every role (MIC-012), journaling the user's
/// defaults in `state_dir` first.
pub fn set_default_microphone(state_dir: &Path) -> Result<(), Error> {
    let cable = endpoints()
        .map_err(platform)
        .map(|endpoints| find_vb_cable(&endpoints))?
        .ok_or_else(|| Error::Message("VB-CABLE is not installed".into()))?;
    let path = journal::journal_path(state_dir, DataFlow::Capture);
    journal::set_default(
        &mut SystemDefaults,
        &path,
        DataFlow::Capture,
        &cable.output.id,
    )
    .map_err(platform)
}

/// Give the default microphone back where it is still the cable (Quit, start-up after a
/// crash). A missing journal is a no-op.
pub fn restore_default_microphone(state_dir: &Path) -> Result<(), Error> {
    let path = journal::journal_path(state_dir, DataFlow::Capture);
    journal::restore(&mut SystemDefaults, &path).map_err(platform)
}

/// Doctor fix (WIN-MIC-004): move every render role off the cable to a real device. This is
/// the user's own correction, so Quit does not undo it; the output journal of the mixer is
/// only read, to prefer the device the user had before.
pub fn fix_cable_default_output(state_dir: &Path) -> Result<(), Error> {
    let active = endpoints().map_err(platform)?;
    let journal = journal::journal_path(state_dir, DataFlow::Render);
    let scratch = state_dir.join("cable-output-fix.json");
    if journal.exists() {
        std::fs::copy(&journal, &scratch).map_err(|error| Error::Message(error.to_string()))?;
    }
    let result = crate::diagnostics::fix_cable_default(&mut SystemDefaults, &scratch, &active);
    if let Err(error) = std::fs::remove_file(&scratch) {
        tracing::debug!(%error, "Cable fix scratch journal not removed");
    }
    match result.map_err(platform)? {
        Some(id) => tracing::info!(device = id, "Default output moved off VB-CABLE"),
        None => tracing::info!("Default output was not on VB-CABLE"),
    }
    Ok(())
}

/// Uninstall and recovery: give back the default microphone and output, and the app volumes
/// and mutes Macafon changed (WIN-MIC-005, WIN-CH-008). The daemon must not be running.
pub fn restore_all(state_dir: &Path) -> Result<(), Error> {
    let _com = ComGuard::new().map_err(platform)?;
    let microphone = restore_default_microphone(state_dir);
    let input_levels = crate::input_level::restore(state_dir).map_err(platform);
    let output = journal::restore(
        &mut SystemDefaults,
        &journal::journal_path(state_dir, DataFlow::Render),
    );
    // Starting the watcher restores the sessions that are running now; the journal keeps the
    // rest for when their apps return.
    let sessions = crate::mixer::SessionWatcher::start(state_dir).and_then(|w| w.restore());
    // Try everything, then report the first failure so the uninstaller can stop.
    let output = output.map_err(platform);
    let sessions = sessions.map_err(platform);
    for error in [&microphone, &input_levels, &output, &sessions]
        .into_iter()
        .filter_map(|result| result.as_ref().err())
    {
        tracing::warn!(%error, "Cannot restore system audio settings");
    }
    microphone.and(input_levels).and(output).and(sessions)
}

/// Real output devices for the A/B player and the output picker (WIN-MIC-009).
pub fn output_devices() -> Result<Vec<Node>, Error> {
    Ok(endpoints()
        .map_err(platform)?
        .iter()
        .filter(|endpoint| is_physical(endpoint, DataFlow::Render))
        .map(node)
        .collect())
}

fn wait_for_cleanup() {
    STOP.store(true, Ordering::Release);
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    while !FINISHED.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}
