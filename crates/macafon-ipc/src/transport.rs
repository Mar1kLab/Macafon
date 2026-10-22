//! Local transport (PRD §10.4, §12.2.3): a per-user Unix socket, or on Windows the named pipe
//! `\\.\pipe\macafon-<user SID>` whose DACL admits only the current user.
//!
//! Connections come as a receive half and a send half that may be used from different
//! threads at once (one blocked reading events while another writes requests).

#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

/// A separately owned handle used to interrupt a connection without locking its writer.
#[derive(Clone)]
pub struct AbortHandle {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(unix)]
    handle: std::sync::Arc<std::os::unix::net::UnixStream>,
    #[cfg(windows)]
    handle: std::sync::Arc<std::os::windows::io::OwnedHandle>,
}

impl AbortHandle {
    pub fn new(writer: &SendHalf) -> std::io::Result<Self> {
        #[cfg(unix)]
        let handle = writer.try_clone()?;
        #[cfg(windows)]
        let handle = {
            use std::os::windows::io::AsHandle;
            let SendHalf::NamedPipe(pipe) = writer;
            pipe.as_handle().try_clone_to_owned()?
        };
        Ok(Self {
            cancelled: Default::default(),
            handle: std::sync::Arc::new(handle),
        })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Repeat while joining I/O threads: an operation can start concurrently with cancellation.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        #[cfg(unix)]
        let _ = self.handle.shutdown(std::net::Shutdown::Both);
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            // SAFETY: the duplicate owns the same pipe object and remains valid for this call.
            unsafe {
                let _ = ::windows::Win32::System::IO::CancelIoEx(
                    ::windows::Win32::Foundation::HANDLE(self.handle.as_raw_handle()),
                    None,
                );
            }
        }
    }
}

pub struct AbortableReader<R> {
    pub reader: R,
    pub abort: AbortHandle,
}
impl<R: std::io::Read> std::io::Read for AbortableReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.abort.is_cancelled() {
            return Ok(0);
        }
        self.reader.read(buffer)
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    pub type RecvHalf = UnixStream;
    pub type SendHalf = UnixStream;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Endpoint {
        path: PathBuf,
    }

    impl Endpoint {
        pub fn socket(path: impl Into<PathBuf>) -> Self {
            Self { path: path.into() }
        }

        /// `$XDG_RUNTIME_DIR/macafon/macafond.sock`.
        pub fn for_current_user() -> Option<Self> {
            crate::socket_path().map(Self::socket)
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    pub fn connect(endpoint: &Endpoint) -> std::io::Result<(RecvHalf, SendHalf)> {
        let stream = UnixStream::connect(&endpoint.path)?;
        Ok((stream.try_clone()?, stream))
    }

    impl std::fmt::Display for Endpoint {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.path.display().fmt(f)
        }
    }
}

#[cfg(windows)]
mod windows {
    use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};
    use interprocess::os::windows::local_socket::ListenerOptionsExt as _;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use std::io;

    pub type RecvHalf = interprocess::local_socket::RecvHalf;
    pub type SendHalf = interprocess::local_socket::SendHalf;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Endpoint {
        name: String,
    }

    impl Endpoint {
        /// A pipe `\\.\pipe\<name>`.
        pub fn pipe(name: impl Into<String>) -> Self {
            Self { name: name.into() }
        }

        /// `\\.\pipe\macafon-<user SID>`: one daemon per user, also across sessions.
        pub fn for_current_user() -> Option<Self> {
            super::sid::current_user_sid()
                .ok()
                .map(|sid| Self::pipe(format!("macafon-{sid}")))
        }

        pub fn name(&self) -> &str {
            &self.name
        }
    }

    impl std::fmt::Display for Endpoint {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, r"\\.\pipe\{}", self.name)
        }
    }

    pub fn connect(endpoint: &Endpoint) -> io::Result<(RecvHalf, SendHalf)> {
        let name = endpoint.name.as_str().to_ns_name::<GenericNamespaced>()?;
        Ok(Stream::connect(name)?.split())
    }

    /// Server side of the daemon pipe. Creating it fails while another process (including a
    /// squatter) owns the name.
    pub struct Listener {
        inner: interprocess::local_socket::Listener,
    }

    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
            let sid = super::sid::current_user_sid()?;
            // Protected DACL: full access for the current user only, nothing inherited.
            let sddl = widestring::U16CString::from_str(format!("D:P(A;;GA;;;{sid})"))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let descriptor = SecurityDescriptor::deserialize(&sddl)?;
            let name = endpoint.name.as_str().to_ns_name::<GenericNamespaced>()?;
            let inner = ListenerOptions::new()
                .name(name)
                .security_descriptor(descriptor)
                .create_sync()?;
            Ok(Self { inner })
        }

        /// Block until the next client connects.
        pub fn accept(&self) -> io::Result<(RecvHalf, SendHalf)> {
            let stream = self
                .inner
                .incoming()
                .next()
                .unwrap_or_else(|| Err(io::ErrorKind::BrokenPipe.into()))?;
            Ok(stream.split())
        }
    }
}

#[cfg(windows)]
mod sid {
    use std::io;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::PWSTR;

    /// String SID of the user running this process, e.g. `S-1-5-21-…-1001`.
    pub fn current_user_sid() -> io::Result<String> {
        // SAFETY: the token handle and the SID string are released below; the TOKEN_USER
        // buffer is sized by the first GetTokenInformation call and 8-byte aligned.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
            let mut size = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let info = GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                size,
                &mut size,
            );
            let _ = CloseHandle(token);
            info?;
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut string = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut string)?;
            let sid = string.to_string();
            let _ = LocalFree(Some(HLOCAL(string.0.cast())));
            sid.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{
        Client, Event, Hello, Outcome, PROTOCOL_VERSION, Platform, Request, Response,
        ServerMessage, decode_request, encode, read_protocol_line,
    };
    use std::io::{BufReader, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn abort_interrupts_blocked_pipe_io_without_waiting_for_the_peer() {
        use std::io::Read;
        use std::time::{Duration, Instant};
        let endpoint = Endpoint::pipe(format!("macafon-abort-test-{}", std::process::id()));
        let listener = Listener::bind(&endpoint).expect("bind");
        let client = std::thread::spawn(move || connect(&endpoint).expect("connect"));
        let (_server_reader, _server_writer) = listener.accept().expect("accept");
        let (mut reader, mut writer) = client.join().expect("client");
        let abort = AbortHandle::new(&writer).expect("abort handle");
        let reading = std::thread::spawn(move || reader.read(&mut [0; 1]));
        let writing = std::thread::spawn(move || writer.write_all(&vec![0; 4 * 1024 * 1024]));
        std::thread::sleep(Duration::from_millis(50));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !reading.is_finished() || !writing.is_finished() {
            abort.cancel();
            assert!(
                Instant::now() < deadline,
                "pipe operations were not cancelled"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(reading.join().expect("reader").is_err());
        assert!(writing.join().expect("writer").is_err());
    }

    #[test]
    fn named_pipe_carries_requests_while_events_flow_concurrently() {
        let endpoint = Endpoint::pipe(format!("macafon-test-{}", std::process::id()));
        let listener = Listener::bind(&endpoint).expect("bind");
        assert!(
            Listener::bind(&endpoint).is_err(),
            "a second owner of the name must be refused"
        );
        let done = Arc::new(AtomicBool::new(false));
        let server_done = done.clone();
        let server = std::thread::spawn(move || {
            let (reader, writer) = listener.accept().expect("accept");
            let writer = Arc::new(Mutex::new(writer));
            // Events are pushed from another thread while this one is blocked reading,
            // as the daemon's hub does; overlapped I/O must not serialise the two.
            let events = {
                let (writer, done) = (writer.clone(), server_done);
                std::thread::spawn(move || {
                    let line = encode(&ServerMessage::Event(Event::PresetsChanged(Vec::new())))
                        .expect("encode");
                    while !done.load(Ordering::Acquire) {
                        let sent = writer.lock().expect("writer").write_all(line.as_bytes());
                        if sent.is_err() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                })
            };
            let mut reader = BufReader::new(reader);
            while let Some(line) = read_protocol_line(&mut reader).expect("read") {
                let request = decode_request(&line).expect("request");
                let result = match request.request {
                    Request::Hello { .. } => serde_json::to_value(Hello {
                        protocol: PROTOCOL_VERSION,
                        daemon_version: "test".into(),
                        platform: Platform::current(),
                        capabilities: Platform::current().capabilities(),
                    })
                    .expect("hello"),
                    _ => serde_json::Value::Null,
                };
                let reply = ServerMessage::Response(Response {
                    id: Some(request.id),
                    outcome: Outcome::Result(result),
                });
                writer
                    .lock()
                    .expect("writer")
                    .write_all(encode(&reply).expect("encode").as_bytes())
                    .expect("write");
            }
            events.join().expect("events");
        });
        let (mut client, hello) = Client::connect(&endpoint, "test").expect("connect");
        assert_eq!(hello.daemon_version, "test");
        for _ in 0..100 {
            client.call(Request::MicGet).expect("call");
        }
        assert!(matches!(
            client.next_event().expect("event"),
            Event::PresetsChanged(_)
        ));
        done.store(true, Ordering::Release);
        drop(client);
        server.join().expect("server");
    }

    #[test]
    fn endpoint_is_per_user() {
        let endpoint = Endpoint::for_current_user().expect("sid");
        assert!(endpoint.name().starts_with("macafon-S-1-"), "{endpoint}");
    }
}
