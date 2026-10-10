//! Bridge between the WebView and the daemon socket (PRD §11.1). The UI never touches audio:
//! it sends protocol requests through `call` and receives events and connection status.
//!
//! One background thread owns the connection: it connects (starting `macafond` once if the
//! socket is missing, §9.7), performs the `hello` handshake, reads replies and events, and
//! reconnects after the daemon restarts (systemd brings it back within 2 s, NFR-S6).

use macafon_ipc::{MAX_LINE_BYTES, PROTOCOL_VERSION};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
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
    writer: UnixStream,
    pending: HashMap<u64, Reply>,
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
    socket: PathBuf,
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
    /// Start the connector thread. `launcher` starts the daemon when the socket is missing.
    pub fn start(
        socket: PathBuf,
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
            socket,
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
        let id = connection.next_id;
        connection.next_id += 1;
        let mut request = json!({ "id": id, "method": method });
        if let Some(params) = params.filter(|params| !params.is_null()) {
            request["params"] = params;
        }
        let mut line = request.to_string();
        line.push('\n');
        match connection.writer.write_all(line.as_bytes()) {
            Ok(()) => {
                connection.pending.insert(id, reply);
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

fn connector(shared: &Shared) {
    // Start the daemon at most once per attempt; a lost connection waits for systemd first.
    let mut may_launch = true;
    loop {
        shared.set_status(Status::Connecting);
        let deadline = Instant::now() + CONNECT_WINDOW;
        let mut launch_error = None;
        let stream = loop {
            match UnixStream::connect(&shared.socket) {
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
            Some(stream) => {
                let result = serve(shared, stream);
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
                let reason = launch_error
                    .unwrap_or_else(|| format!("no daemon socket at {}", shared.socket.display()));
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

fn read_line(reader: &mut BufReader<UnixStream>) -> Result<Option<Value>, String> {
    let mut line = String::new();
    let read = reader
        .take(MAX_LINE_BYTES as u64)
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if read == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|error| format!("invalid message from macafond: {error}"))
}

/// Handshake, then dispatch replies and events until the daemon goes away.
fn serve(shared: &Shared, stream: UnixStream) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut writer = stream;
    let hello = json!({
        "id": 0,
        "method": "hello",
        "params": { "protocol": PROTOCOL_VERSION, "client": "macafon-ui" },
    });
    writer
        .write_all(format!("{hello}\n").as_bytes())
        .map_err(|error| error.to_string())?;
    let version = loop {
        let message = read_line(&mut reader)?.ok_or("macafond closed during the handshake")?;
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
    let result = dispatch(shared, &mut reader);
    // Fail whatever was still waiting for this connection.
    if let Some(connection) = shared.lock().connection.take() {
        for (_, reply) in connection.pending {
            let _ = reply.send(Err(BridgeError::unavailable()));
        }
    }
    result
}

fn dispatch(shared: &Shared, reader: &mut BufReader<UnixStream>) -> Result<(), String> {
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
        let Some(reply) = reply else { continue };
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
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("macafond")))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| Path::new("macafond").to_path_buf());
    let mut child = Command::new(&program)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
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

    fn socket_path() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("macafon-bridge-{}-{n}.sock", std::process::id()))
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
        let path = socket_path();
        let listener = UnixListener::bind(&path).expect("bind the fake daemon");
        let launched = Arc::new(AtomicUsize::new(0));
        let daemon = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("bridge connects");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut writer = stream;
            let mut line = String::new();
            let mut read = |reader: &mut BufReader<UnixStream>| -> Value {
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
            third["id"].as_u64().expect("numeric id")
        });
        let (events, sink) = mpsc::channel();
        let bridge = Bridge::start(path.clone(), Channel(Mutex::new(events)), {
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
        drop(std::fs::remove_file(&path));
        assert_eq!(
            third.recv_timeout(CALL_TIMEOUT).expect("third fails"),
            Err(BridgeError::unavailable())
        );
        // Without a socket the bridge reports the daemon unavailable; it did not launch one
        // because it was reachable at start.
        wait_status(&sink, |status| matches!(status, Status::Unavailable { .. }));
        assert_eq!(launched.load(Ordering::Relaxed), 0);
        assert_eq!(
            bridge.call("mic.get", None),
            Err(BridgeError::unavailable())
        );
    }
}
