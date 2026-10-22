//! Bridge between the WebView and the daemon socket (PRD §11.1). The UI never touches audio:
//! it sends protocol requests through `call` and receives events and connection status.
//!
//! One background thread owns the connection: it connects (starting `macafond` once if the
//! socket or pipe is missing, §9.7), performs the `hello` handshake, reads replies and events,
//! and reconnects after the daemon restarts (systemd brings it back within 2 s, NFR-S6).

use macafon_ipc::transport::{self, RecvHalf, SendHalf};
use macafon_ipc::{Endpoint, PROTOCOL_VERSION, read_protocol_line};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
#[cfg(test)]
use std::io::BufRead;
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long to wait for the socket after starting the daemon or losing the connection.
const CONNECT_WINDOW: Duration = Duration::from_secs(4);
const CONNECT_POLL: Duration = Duration::from_millis(150);
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Connecting,
    Connected { version: String },
    Unavailable { reason: String },
}

/// Errors as the UI sees them: daemon error codes plus `unavailable` and `timeout`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}

impl BridgeError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    fn unavailable() -> Self {
        Self::new("unavailable", "macafond is not running")
    }
}

/// Where events and status changes go: the Tauri app, or a channel in tests.
pub trait Sink: Send + Sync + 'static {
    fn event(&self, event: Value);
    fn status(&self, status: &Status);
}

type Reply = mpsc::Sender<Result<Value, BridgeError>>;

struct Connection {
    writer: mpsc::SyncSender<String>,
    pending: HashMap<u64, (Reply, Instant)>,
    next_id: u64,
}

struct State {
    connection: Option<Connection>,
    status: Status,
    /// Set by `retry`; the connector waits on it while the daemon is unavailable.
    retry: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    sink: Box<dyn Sink>,
    endpoint: Endpoint,
    launcher: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves plain data behind; keep going.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_status(&self, status: Status) {
        let mut state = self.lock();
        if state.status != status {
            state.status = status.clone();
            drop(state);
            self.sink.status(&status);
        }
    }
}

#[derive(Clone)]
pub struct Bridge {
    shared: Arc<Shared>,
}

impl Bridge {
    /// Start the connector thread. `launcher` starts the daemon when the endpoint is missing.
    pub fn start(
        endpoint: Endpoint,
        sink: impl Sink,
        launcher: impl Fn() -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                connection: None,
                status: Status::Connecting,
                retry: false,
            }),
            wake: Condvar::new(),
            sink: Box::new(sink),
            endpoint,
            launcher: Box::new(launcher),
        });
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("macafon-bridge".into())
            .spawn(move || connector(&thread_shared))
            .expect("spawning the bridge thread only fails when the system is out of threads");
        Self { shared }
    }

    pub fn status(&self) -> Status {
        self.shared.lock().status.clone()
    }

    /// Try again (and start the daemon again) after it was unavailable.
    pub fn retry(&self) {
        self.shared.lock().retry = true;
        self.shared.wake.notify_all();
    }

    /// Send a request; the receiver gets the daemon's result or error.
    pub fn send(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> mpsc::Receiver<Result<Value, BridgeError>> {
        let (reply, receiver) = mpsc::channel();
        let mut state = self.shared.lock();
        let Some(connection) = state.connection.as_mut() else {
            let _ = reply.send(Err(BridgeError::unavailable()));
            return receiver;
        };
        if connection.pending.len() >= 64 {
            let _ = reply.send(Err(BridgeError::new("busy", "too many pending requests")));
            return receiver;
        }
        let id = connection.next_id;
        connection.next_id += 1;
        let mut request = json!({ "id": id, "method": method });
        if let Some(params) = params.filter(|params| !params.is_null()) {
            request["params"] = params;
        }
        let mut line = request.to_string();
        line.push('\n');
        match connection.writer.try_send(line) {
            Ok(()) => {
                connection
                    .pending
                    .insert(id, (reply, Instant::now() + CALL_TIMEOUT));
                self.shared.wake.notify_all();
            }
            Err(error) => {
                let _ = reply.send(Err(BridgeError::new("unavailable", error.to_string())));
            }
        }
        receiver
    }

    /// Blocking call with a timeout (run off the UI thread).
    pub fn call(&self, method: &str, params: Option<Value>) -> Result<Value, BridgeError> {
        let receiver = self.send(method, params);
        receiver
            .recv_timeout(CALL_TIMEOUT)
            .unwrap_or_else(|_| Err(BridgeError::new("timeout", "macafond did not reply")))
    }
}

fn connector(shared: &Arc<Shared>) {
    // Start the daemon at most once per attempt; a lost connection waits for systemd first.
    let mut may_launch = true;
    loop {
        shared.set_status(Status::Connecting);
        let deadline = Instant::now() + CONNECT_WINDOW;
        let mut launch_error = None;
        let stream = loop {
            match transport::connect(&shared.endpoint) {
                Ok(stream) => break Some(stream),
                Err(_) if may_launch => {
                    may_launch = false;
                    if let Err(error) = (shared.launcher)() {
                        launch_error = Some(error);
                    }
                }
                Err(_) if Instant::now() >= deadline => break None,
                Err(_) => {}
            }
            std::thread::sleep(CONNECT_POLL);
        };
        match stream {
            Some((reader, writer)) => {
                let result = serve(shared, reader, writer);
                // After a lost connection, give systemd time instead of starting a second
                // daemon; after Quit the UI stays offline until Retry.
                may_launch = false;
                match result {
                    Ok(()) => tracing::info!("macafond closed the connection"),
                    // A protocol mismatch will not heal by reconnecting.
                    Err(error) if error.starts_with("protocol") => {
                        unavailable(shared, error);
                        may_launch = true;
                    }
                    Err(error) => tracing::warn!(%error, "macafond connection failed"),
                }
            }
            None => {
                let reason =
                    launch_error.unwrap_or_else(|| format!("no daemon at {}", shared.endpoint));
                unavailable(shared, reason);
                may_launch = true;
            }
        }
    }
}

/// Report the daemon as unavailable and wait for `retry`.
fn unavailable(shared: &Shared, reason: String) {
    shared.set_status(Status::Unavailable { reason });
    let mut state = shared.lock();
    while !state.retry {
        state = shared
            .wake
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    state.retry = false;
}

fn read_line(reader: &mut impl std::io::BufRead) -> Result<Option<Value>, String> {
    let Some(line) = read_protocol_line(reader).map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|error| format!("invalid message from macafond: {error}"))
}

/// Handshake, then dispatch replies and events until the daemon goes away.
fn serve(shared: &Arc<Shared>, reader: RecvHalf, writer: SendHalf) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let abort = transport::AbortHandle::new(&writer).map_err(|error| error.to_string())?;
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = spawn_watchdog(shared.clone(), abort.clone(), finished.clone());
    let (out, lines) = mpsc::sync_channel::<String>(64);
    let writer_thread = spawn_writer(shared.clone(), abort.clone(), writer, lines);
    let mut reader = BufReader::new(transport::AbortableReader {
        reader,
        abort: abort.clone(),
    });
    let result = serve_connection(shared, &mut reader, out);
    // The watchdog keeps cancelling until both halves have left I/O, including a racing one.
    abort.cancel();
    shared.wake.notify_all();
    if let Some(connection) = shared.lock().connection.take() {
        for (_, (reply, _)) in connection.pending {
            let _ = reply.send(Err(BridgeError::unavailable()));
        }
    }
    let _ = writer_thread.join();
    {
        let _state = shared.lock();
        finished.store(true, Ordering::Release);
        shared.wake.notify_all();
    }
    let _ = watchdog.join();
    result
}

/// Writes queued lines, so a stalled daemon never blocks a caller holding the state lock.
fn spawn_writer(
    shared: Arc<Shared>,
    abort: transport::AbortHandle,
    mut writer: SendHalf,
    lines: mpsc::Receiver<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in lines {
            if abort.is_cancelled() || writer.write_all(line.as_bytes()).is_err() {
                abort.cancel();
                shared.wake.notify_all();
                break;
            }
        }
    })
}

/// Aborts the connection when the handshake or the oldest pending call overruns its deadline;
/// after an abort it repeats the cancellation until `finished`.
fn spawn_watchdog(
    shared: Arc<Shared>,
    abort: transport::AbortHandle,
    finished: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let handshake_deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            let state = shared.lock();
            if finished.load(Ordering::Acquire) {
                break;
            }
            let deadline = match &state.connection {
                None => Some(handshake_deadline),
                Some(connection) => connection.pending.values().map(|(_, at)| *at).min(),
            };
            if abort.is_cancelled() || deadline.is_some_and(|at| Instant::now() >= at) {
                drop(state);
                abort.cancel();
                std::thread::sleep(Duration::from_millis(10));
            } else {
                let wait = deadline.map_or(Duration::from_secs(60), |at| {
                    at.saturating_duration_since(Instant::now())
                });
                drop(
                    shared
                        .wake
                        .wait_timeout(state, wait)
                        .unwrap_or_else(|error| error.into_inner()),
                );
            }
        }
    })
}

fn serve_connection(
    shared: &Shared,
    reader: &mut impl std::io::BufRead,
    writer: mpsc::SyncSender<String>,
) -> Result<(), String> {
    let hello = json!({
        "id": 0,
        "method": "hello",
        "params": { "protocol": PROTOCOL_VERSION, "client": "macafon-ui" },
    });
    writer
        .send(format!("{hello}\n"))
        .map_err(|error| error.to_string())?;
    let version = loop {
        let message = read_line(reader)?.ok_or("macafond closed during the handshake")?;
        if message.get("id") != Some(&json!(0)) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(format!("protocol handshake rejected: {error}"));
        }
        break message["result"]["daemon_version"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
    };
    shared.lock().connection = Some(Connection {
        writer,
        pending: HashMap::new(),
        next_id: 1,
    });
    shared.set_status(Status::Connected { version });
    shared.wake.notify_all();
    dispatch(shared, reader)
}

fn dispatch(shared: &Shared, reader: &mut impl std::io::BufRead) -> Result<(), String> {
    while let Some(mut message) = read_line(reader)? {
        if message.get("event").is_some() {
            shared.sink.event(message);
            continue;
        }
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            tracing::warn!(%message, "reply without an id");
            continue;
        };
        let reply = shared
            .lock()
            .connection
            .as_mut()
            .and_then(|connection| connection.pending.remove(&id));
        let Some((reply, _)) = reply else { continue };
        let outcome = match message.get_mut("error") {
            Some(error) => Err(BridgeError::new(
                error["code"].as_str().unwrap_or("internal"),
                error["message"].as_str().unwrap_or_default(),
            )),
            None => Ok(message
                .get_mut("result")
                .map(Value::take)
                .unwrap_or(Value::Null)),
        };
        let _ = reply.send(outcome);
    }
    Ok(())
}

/// `macafond` next to the UI binary, else on PATH; detached so it outlives the UI (BG-001).
pub fn launch_daemon() -> Result<(), String> {
    use std::process::{Command, Stdio};
    let name = if cfg!(windows) {
        "macafond.exe"
    } else {
        "macafond"
    };
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| Path::new(name).to_path_buf());
    let mut command = Command::new(&program);
    command
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    tracing::info!(program = %program.display(), "started macafond");
    // Reap it if it exits while the UI still runs.
    std::thread::spawn(move || match child.wait() {
        Ok(status) if !status.success() => {
            tracing::warn!(%status, "macafond exited; run `macafond run` in a terminal to see why")
        }
        _ => {}
    });
    Ok(())
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// No console window, and Ctrl+C in the UI's terminal does not reach the daemon.
#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Channel(Mutex<mpsc::Sender<Result<Value, Status>>>);

    impl Sink for Channel {
        fn event(&self, event: Value) {
            let _ = self.0.lock().expect("test sink").send(Ok(event));
        }

        fn status(&self, status: &Status) {
            let _ = self.0.lock().expect("test sink").send(Err(status.clone()));
        }
    }

    fn unique_name() -> String {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        format!("macafon-bridge-{}-{n}", std::process::id())
    }

    #[test]
    fn stalled_peer_does_not_lock_status_or_accumulate_unbounded_requests() {
        let (listener, endpoint) = FakeDaemon::bind();
        let (stop, stopped) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (reader, mut writer) = listener.accept();
            let mut reader = BufReader::new(reader);
            read_protocol_line(&mut reader).expect("hello");
            writeln!(writer, r#"{{"id":0,"result":{{"daemon_version":"test"}}}}"#)
                .expect("hello reply");
            // Keep both halves open, but never read the following requests.
            let _ = stopped.recv_timeout(Duration::from_secs(20));
        });
        let (events, sink) = mpsc::channel();
        let bridge = Bridge::start(endpoint, Channel(Mutex::new(events)), || Ok(()));
        wait_status(&sink, |state| matches!(state, Status::Connected { .. }));
        let mut pending = Vec::new();
        for _ in 0..64 {
            pending.push(bridge.send("test", Some(json!({"padding": "x".repeat(64 * 1024)}))));
        }
        let start = Instant::now();
        assert!(matches!(bridge.status(), Status::Connected { .. }));
        let excess = bridge
            .send("test", None)
            .recv_timeout(Duration::from_secs(1))
            .expect("bounded rejection");
        assert!(excess.is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(
            pending
                .remove(0)
                .recv_timeout(CALL_TIMEOUT + Duration::from_secs(2))
                .expect("connection aborted")
                .is_err()
        );
        bridge.retry();
        let _ = stop.send(());
        server.join().expect("fake daemon");
    }

    /// A fake daemon endpoint; `accept` returns the connection halves, and dropping the
    /// listener makes the endpoint unreachable again.
    #[cfg(unix)]
    struct FakeDaemon {
        listener: std::os::unix::net::UnixListener,
        path: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl FakeDaemon {
        fn bind() -> (Self, Endpoint) {
            let path = std::env::temp_dir().join(format!("{}.sock", unique_name()));
            let listener =
                std::os::unix::net::UnixListener::bind(&path).expect("bind the fake daemon");
            (
                Self {
                    listener,
                    path: path.clone(),
                },
                Endpoint::socket(path),
            )
        }

        fn accept(&self) -> (RecvHalf, SendHalf) {
            let (stream, _) = self.listener.accept().expect("bridge connects");
            (stream.try_clone().expect("clone"), stream)
        }
    }

    #[cfg(unix)]
    impl Drop for FakeDaemon {
        fn drop(&mut self) {
            drop(std::fs::remove_file(&self.path));
        }
    }

    #[cfg(windows)]
    struct FakeDaemon(transport::Listener);

    #[cfg(windows)]
    impl FakeDaemon {
        fn bind() -> (Self, Endpoint) {
            let endpoint = Endpoint::pipe(unique_name());
            let listener = transport::Listener::bind(&endpoint).expect("bind the fake daemon");
            (Self(listener), endpoint)
        }

        fn accept(&self) -> (RecvHalf, SendHalf) {
            self.0.accept().expect("bridge connects")
        }
    }

    fn wait_status(
        sink: &mpsc::Receiver<Result<Value, Status>>,
        want: impl Fn(&Status) -> bool,
    ) -> Status {
        loop {
            match sink.recv_timeout(Duration::from_secs(10)) {
                Ok(Err(status)) if want(&status) => return status,
                Ok(_) => {}
                Err(error) => panic!("no matching status: {error}"),
            }
        }
    }

    /// A fake daemon that replies out of order and interleaves events, then disappears with
    /// a request still pending; the bridge matches replies by id and fails the leftover call.
    #[test]
    fn matches_replies_by_id_forwards_events_and_fails_pending_calls() {
        let (listener, endpoint) = FakeDaemon::bind();
        let launched = Arc::new(AtomicUsize::new(0));
        let daemon = std::thread::spawn(move || {
            let (reader, mut writer) = listener.accept();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            let mut read = |reader: &mut BufReader<RecvHalf>| -> Value {
                line.clear();
                reader.read_line(&mut line).expect("request");
                serde_json::from_str(&line).expect("JSON request")
            };
            let hello = read(&mut reader);
            assert_eq!(hello["method"], "hello");
            assert_eq!(hello["params"]["protocol"], PROTOCOL_VERSION);
            writeln!(
                writer,
                r#"{{"id":0,"result":{{"protocol":1,"daemon_version":"9.9.9"}}}}"#
            )
            .expect("hello reply");
            let first = read(&mut reader);
            let second = read(&mut reader);
            let third = read(&mut reader);
            assert_eq!(first["method"], "mic.get");
            assert!(first.get("params").is_none(), "null params are omitted");
            assert_eq!(second["params"]["muted"], true);
            writeln!(
                writer,
                r#"{{"event":"meters","data":{{"input_peak_dbfs":-20.0}}}}"#
            )
            .expect("event");
            writeln!(
                writer,
                r#"{{"id":{},"error":{{"code":"busy","message":"later"}}}}"#,
                second["id"]
            )
            .expect("second reply");
            writeln!(writer, r#"{{"id":{},"result":{{"ok":1}}}}"#, first["id"])
                .expect("first reply");
            // `third` never gets an answer: the daemon goes away.
            drop(listener);
            third["id"].as_u64().expect("numeric id")
        });
        let (events, sink) = mpsc::channel();
        let bridge = Bridge::start(endpoint, Channel(Mutex::new(events)), {
            let launched = launched.clone();
            move || {
                launched.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        });
        let status = wait_status(&sink, |status| matches!(status, Status::Connected { .. }));
        assert_eq!(
            status,
            Status::Connected {
                version: "9.9.9".into()
            }
        );
        let first = bridge.send("mic.get", Some(Value::Null));
        let second = bridge.send("mic.set_muted", Some(json!({"muted": true})));
        let third = bridge.send("ab.get", None);
        assert_eq!(
            second.recv_timeout(CALL_TIMEOUT).expect("second reply"),
            Err(BridgeError::new("busy", "later"))
        );
        assert_eq!(
            first.recv_timeout(CALL_TIMEOUT).expect("first reply"),
            Ok(json!({"ok": 1}))
        );
        let event = loop {
            match sink.recv_timeout(CALL_TIMEOUT).expect("event") {
                Ok(event) => break event,
                Err(_) => continue,
            }
        };
        assert_eq!(event["event"], "meters");
        daemon.join().expect("fake daemon");
        assert_eq!(
            third.recv_timeout(CALL_TIMEOUT).expect("third fails"),
            Err(BridgeError::unavailable())
        );
        // Without an endpoint the bridge reports the daemon unavailable; it did not launch one
        // because it was reachable at start.
        wait_status(&sink, |status| matches!(status, Status::Unavailable { .. }));
        assert_eq!(launched.load(Ordering::Relaxed), 0);
        assert_eq!(
            bridge.call("mic.get", None),
            Err(BridgeError::unavailable())
        );
    }
}
