//! IPC server: one reader and one writer thread per client, a shared meter pump.
//!
//! Only the control loop touches daemon state; client threads forward requests to it and wait
//! for the reply. Outgoing lines go through bounded queues, so a stalled client never blocks
//! the control loop or other clients — it just misses events.

use crate::host::ControlMessage;
use anyhow::{Context, Result};
use macafon_backend::ControlSender;
use macafon_dsp::{ChannelTelemetry, Telemetry};
use macafon_ipc::Endpoint;
use macafon_ipc::{
    ErrorBody, ErrorCode, Event, FrameError, Hello, MAX_METER_HZ, Outcome, PROTOCOL_VERSION,
    Platform, Request, Response, ServerMessage, SoundpadState, decode_request, encode,
    read_protocol_line,
};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::io::{BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::time::Duration;

const OUTGOING_LINES: usize = 256;
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Clients {
    /// Clients that completed `hello`, with their meter rate if subscribed.
    registered: HashMap<u64, (mpsc::SyncSender<String>, Option<u32>)>,
    device_meters: HashSet<u64>,
    /// Clients that said hello as the UI.
    ui: HashSet<u64>,
}

pub struct Hub {
    clients: Mutex<Clients>,
    meters_wanted: Condvar,
    telemetry: Arc<Telemetry>,
    /// Live channels in display order; empty while the mixer is off.
    mixer_telemetry: Mutex<Vec<(String, Arc<ChannelTelemetry>)>>,
    next_id: AtomicU64,
    soundpad_updates: mpsc::SyncSender<SoundpadState>,
}

impl Hub {
    pub fn new() -> Arc<Self> {
        let (soundpad_updates, updates) = mpsc::sync_channel(1);
        Arc::new_cyclic(|weak: &std::sync::Weak<Self>| {
            let weak = weak.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("macafon-soundpad-events".into())
                .spawn(move || {
                    while let Ok(state) = updates.recv() {
                        let Some(hub) = weak.upgrade() else {
                            break;
                        };
                        hub.broadcast(&Event::SoundpadChanged(state));
                    }
                })
            {
                tracing::warn!(%error, "Cannot start soundpad event worker");
            }
            Self {
                clients: Mutex::default(),
                meters_wanted: Condvar::new(),
                telemetry: Arc::default(),
                mixer_telemetry: Mutex::default(),
                next_id: AtomicU64::new(1),
                soundpad_updates,
            }
        })
    }

    pub fn telemetry(&self) -> Arc<Telemetry> {
        self.telemetry.clone()
    }

    fn clients(&self) -> MutexGuard<'_, Clients> {
        // A panicking client thread cannot leave the map inconsistent; keep serving.
        self.clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn broadcast(&self, event: &Event) {
        if self.clients().registered.is_empty() {
            return;
        }
        // ServerMessage is untagged: borrowing the event produces the same wire frame
        // without cloning its entire state (including potentially large sound libraries).
        let Ok(line) = encode(event) else {
            return;
        };
        for (out, _) in self.clients().registered.values() {
            let _ = out.try_send(line.clone());
        }
    }

    pub fn has_clients(&self) -> bool {
        !self.clients().registered.is_empty()
    }

    /// Serialize large sound inventories away from the control loop. A full queue is retried
    /// from the latest daemon state on a later tick instead of accumulating stale snapshots.
    pub fn broadcast_soundpad(&self, state: SoundpadState) -> bool {
        self.soundpad_updates.try_send(state).is_ok()
    }

    pub fn set_mixer_telemetry(&self, channels: Vec<(String, Arc<ChannelTelemetry>)>) {
        let metering = self.metering();
        channels
            .iter()
            .for_each(|(_, telemetry)| telemetry.set_enabled(metering));
        *self
            .mixer_telemetry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = channels;
    }

    fn mixer_channels(&self) -> Vec<(String, Arc<ChannelTelemetry>)> {
        self.mixer_telemetry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Some client is subscribed to meters.
    pub fn metering(&self) -> bool {
        self.clients()
            .registered
            .values()
            .any(|(_, hz)| hz.is_some())
    }

    fn set_meter_rate(&self, client: u64, hz: Option<u32>) {
        if let Some(entry) = self.clients().registered.get_mut(&client) {
            entry.1 = hz;
        }
        self.meters_wanted.notify_all();
    }

    /// A UI process is connected and can take `ui.show` instead of a new launch.
    pub fn ui_connected(&self) -> bool {
        !self.clients().ui.is_empty()
    }

    pub fn device_metering(&self) -> bool {
        !self.clients().device_meters.is_empty()
    }

    pub fn device_meter(&self, name: &str, peak_dbfs: f32) {
        let Ok(line) = encode(&ServerMessage::Event(Event::DeviceMeter {
            name: name.to_owned(),
            peak_dbfs,
        })) else {
            return;
        };
        let clients = self.clients();
        for client in &clients.device_meters {
            if let Some((out, _)) = clients.registered.get(client) {
                let _ = out.try_send(line.clone());
            }
        }
    }

    fn remove(&self, client: u64) {
        let mut clients = self.clients();
        clients.registered.remove(&client);
        clients.device_meters.remove(&client);
        clients.ui.remove(&client);
        self.meters_wanted.notify_all();
    }

    /// Sample meters at the highest subscribed rate; sleep with metering off otherwise.
    fn pump_meters(&self) {
        loop {
            let hz = {
                let mut clients = self.clients();
                loop {
                    let wanted = clients.registered.values().filter_map(|(_, hz)| *hz).max();
                    if let Some(hz) = wanted {
                        break hz;
                    }
                    self.telemetry.set_enabled(false);
                    self.mixer_channels()
                        .iter()
                        .for_each(|(_, telemetry)| telemetry.set_enabled(false));
                    clients = self
                        .meters_wanted
                        .wait(clients)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            self.telemetry.set_enabled(true);
            let channels = self.mixer_channels();
            channels
                .iter()
                .for_each(|(_, telemetry)| telemetry.set_enabled(true));
            std::thread::sleep(Duration::from_secs(1) / hz);
            let mut lines = Vec::with_capacity(2);
            lines.extend(encode(&ServerMessage::Event(Event::Meters(self.telemetry.take()))).ok());
            if !channels.is_empty() {
                let meters = crate::mixer::take_meters(&channels);
                lines.extend(encode(&ServerMessage::Event(Event::MixerMeters(meters))).ok());
            }
            for (out, rate) in self.clients().registered.values() {
                if rate.is_some() {
                    for line in &lines {
                        let _ = out.try_send(line.clone());
                    }
                }
            }
        }
    }
}

/// Removes the socket file when the daemon exits; the named pipe vanishes with the process.
pub struct SocketGuard(#[cfg(unix)] PathBuf);

#[cfg(unix)]
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Write half of a client connection.
trait Outgoing: Write + Send + 'static {
    /// Also wake the reader, so a client that stopped reading is dropped.
    fn close(&mut self) {}
}

#[cfg(unix)]
impl Outgoing for UnixStream {
    fn close(&mut self) {
        let _ = self.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(windows)]
impl Outgoing for macafon_ipc::transport::SendHalf {}

fn spawn_meters(hub: &Arc<Hub>) -> Result<()> {
    let pump = hub.clone();
    std::thread::Builder::new()
        .name("macafon-meters".into())
        .spawn(move || pump.pump_meters())?;
    Ok(())
}

fn spawn_client(
    connection: (impl Read + Send + 'static, impl Outgoing),
    hub: &Arc<Hub>,
    control: &ControlSender<ControlMessage>,
) {
    let (hub, control) = (hub.clone(), control.clone());
    let spawned = std::thread::Builder::new()
        .name("macafon-client".into())
        .spawn(move || serve(connection.0, connection.1, &hub, &control));
    if let Err(error) = spawned {
        tracing::warn!(%error, "Cannot start client thread");
    }
}

/// Bind the per-user pipe (DACL: current user only, PRD §10.4, §12.2.3) and serve clients on
/// background threads. Fails while another daemon owns the name (single instance).
#[cfg(windows)]
pub fn start(
    endpoint: &Endpoint,
    hub: Arc<Hub>,
    control: ControlSender<ControlMessage>,
) -> Result<SocketGuard> {
    let listener = macafon_ipc::transport::Listener::bind(endpoint)
        .with_context(|| format!("cannot bind {endpoint}; is another macafond running?"))?;
    spawn_meters(&hub)?;
    std::thread::Builder::new()
        .name("macafon-ipc".into())
        .spawn(move || {
            loop {
                match listener.accept() {
                    Ok(connection) => spawn_client(connection, &hub, &control),
                    Err(error) => tracing::warn!(%error, "IPC accept failed"),
                }
            }
        })?;
    tracing::info!(pipe = %endpoint, "Control pipe ready");
    Ok(SocketGuard())
}

/// Bind the socket (owner-only, PRD §10.4) and serve clients on background threads.
#[cfg(unix)]
pub fn start(
    endpoint: &Endpoint,
    hub: Arc<Hub>,
    control: ControlSender<ControlMessage>,
) -> Result<SocketGuard> {
    let path = endpoint.path();
    let directory = path.parent().context("socket path has no directory")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            anyhow::bail!("another macafond is already running ({})", path.display());
        }
        std::fs::remove_file(path)
            .with_context(|| format!("cannot remove stale {}", path.display()))?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("cannot bind {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    spawn_meters(&hub)?;
    std::thread::Builder::new()
        .name("macafon-ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream.and_then(|stream| Ok((stream.try_clone()?, stream))) {
                    Ok(connection) => spawn_client(connection, &hub, &control),
                    Err(error) => tracing::warn!(%error, "IPC accept failed"),
                }
            }
        })?;
    tracing::info!(socket = %path.display(), "Control socket ready");
    Ok(SocketGuard(path.to_owned()))
}

fn serve(
    reader: impl Read,
    mut writer: impl Outgoing,
    hub: &Hub,
    control: &ControlSender<ControlMessage>,
) {
    let id = hub.next_id.fetch_add(1, Ordering::Relaxed);
    let (out, lines) = mpsc::sync_channel::<String>(OUTGOING_LINES);
    let spawned = std::thread::Builder::new()
        .name("macafon-client-out".into())
        .spawn(move || {
            for line in lines {
                if writer.write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
            writer.close();
        });
    if spawned.is_err() {
        return;
    }
    let mut reader = BufReader::new(reader);
    let mut greeted = false;
    let mut ui = false;
    loop {
        let line = match read_protocol_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) | Err(FrameError::Io(_)) => break,
            Err(FrameError::Incomplete) => {
                send(
                    &out,
                    None,
                    Err(ErrorBody::new(ErrorCode::BadRequest, "line too long")),
                );
                break;
            }
        };
        let envelope = match decode_request(&line) {
            Ok(envelope) => envelope,
            Err(response) => {
                let _ =
                    out.try_send(encode(&ServerMessage::Response(response)).unwrap_or_default());
                continue;
            }
        };
        let reply = match envelope.request {
            Request::Hello { protocol, client } => {
                if protocol != PROTOCOL_VERSION {
                    Err(ErrorBody::new(
                        ErrorCode::ProtocolMismatch,
                        format!("macafond speaks protocol {PROTOCOL_VERSION}, client {protocol}"),
                    ))
                } else {
                    if !greeted {
                        greeted = true;
                        ui = client == "macafon-ui" || client.starts_with("macafon-ui-");
                        let mut clients = hub.clients();
                        clients.registered.insert(id, (out.clone(), None));
                        if ui {
                            clients.ui.insert(id);
                        }
                        drop(clients);
                        tracing::debug!(client, id, "IPC client connected");
                    }
                    serde_json::to_value(Hello {
                        protocol: PROTOCOL_VERSION,
                        daemon_version: env!("CARGO_PKG_VERSION").into(),
                        platform: Platform::current(),
                        capabilities: Platform::current().capabilities(),
                    })
                    .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
                }
            }
            _ if !greeted => Err(ErrorBody::new(ErrorCode::BadRequest, "send hello first")),
            Request::DevicesSubscribe => {
                hub.clients().device_meters.insert(id);
                Ok(serde_json::Value::Null)
            }
            Request::DevicesUnsubscribe => {
                hub.clients().device_meters.remove(&id);
                Ok(serde_json::Value::Null)
            }
            Request::MetersSubscribe { hz } => {
                hub.set_meter_rate(id, Some(hz.clamp(1, MAX_METER_HZ)));
                Ok(serde_json::Value::Null)
            }
            Request::MetersUnsubscribe => {
                hub.set_meter_rate(id, None);
                Ok(serde_json::Value::Null)
            }
            request => forward(control, request),
        };
        send(&out, Some(envelope.id), reply);
    }
    hub.remove(id);
    if ui {
        let _ = control.send(ControlMessage::Disconnected);
    }
}

fn forward(
    control: &ControlSender<ControlMessage>,
    request: Request,
) -> Result<serde_json::Value, ErrorBody> {
    let (reply, outcome) = mpsc::channel();
    let internal = |message: &str| ErrorBody::new(ErrorCode::Internal, message);
    control
        .send(ControlMessage::Request { request, reply })
        .map_err(|_| internal("daemon is shutting down"))?;
    match outcome.recv_timeout(REPLY_TIMEOUT) {
        Ok(Outcome::Result(value)) => Ok(value),
        Ok(Outcome::Error(error)) => Err(error),
        Err(_) => Err(internal("daemon did not answer in time")),
    }
}

fn send(
    out: &mpsc::SyncSender<String>,
    id: Option<u64>,
    reply: Result<serde_json::Value, ErrorBody>,
) {
    let outcome = match reply {
        Ok(value) => Outcome::Result(value),
        Err(error) => Outcome::Error(error),
    };
    if let Ok(line) = encode(&ServerMessage::Response(Response { id, outcome })) {
        // Replies wait for room: only events may be dropped for a slow client.
        let _ = out.send(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_library_broadcast_preserves_the_full_event_shape() {
        let hub = Hub::new();
        let (out, incoming) = mpsc::sync_channel(1);
        hub.clients().registered.insert(1, (out, None));
        let state = SoundpadState {
            folder: "sounds".into(),
            default_folder: false,
            categories: vec!["Empty".into()],
            sounds: Vec::new(),
            prefs: macafon_ipc::SoundpadPrefs {
                to_microphone: true,
                to_headphones: true,
                microphone_volume_pct: 100.0,
                headphones_volume_pct: 100.0,
                mute_voice_while_playing: false,
                monitor_device: None,
                show_in_overlay: true,
                overlay_view: Default::default(),
                overlay_controls: Default::default(),
            },
            playback: None,
            microphone_ready: false,
        };
        let expected = encode(&Event::SoundpadChanged(state.clone())).expect("event");
        assert!(hub.broadcast_soundpad(state));
        assert_eq!(
            incoming
                .recv_timeout(Duration::from_secs(5))
                .expect("deferred event"),
            expected
        );
    }

    #[test]
    fn broadcast_preserves_event_wire_format() {
        let hub = Hub::new();
        let event = Event::SoundpadPlayback(None);
        assert_no_alloc::assert_no_alloc(|| hub.broadcast(&event));
        let (out, incoming) = mpsc::sync_channel(1);
        hub.clients().registered.insert(1, (out, None));
        hub.broadcast(&event);
        assert_eq!(
            incoming.try_recv().expect("broadcast frame"),
            encode(&ServerMessage::Event(event)).expect("event frame")
        );
    }
}
