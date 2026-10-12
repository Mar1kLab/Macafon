//! Blocking client for `macafon ctl` and tests.

use crate::{
    ErrorBody, Event, FrameError, Hello, Outcome, PROTOCOL_VERSION, Request, RequestEnvelope,
    ServerMessage, encode, read_protocol_line,
};
use serde::de::DeserializeOwned;
use std::collections::VecDeque;
use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot reach macafond: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid message from macafond: {0}")]
    Protocol(#[from] serde_json::Error),
    #[error("invalid framing from macafond: {0}")]
    Framing(FrameError),
    #[error("macafond closed the connection")]
    Closed,
    #[error("macafond rejected the request: {0}")]
    Rejected(#[from] ErrorBody),
}

impl From<FrameError> for ClientError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Io(error) => Self::Io(error),
            FrameError::Incomplete => Self::Framing(error),
        }
    }
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    events: VecDeque<Event>,
}

impl Client {
    /// Connect and perform the protocol handshake.
    pub fn connect(path: &Path, name: &str) -> Result<(Self, Hello), ClientError> {
        let stream = UnixStream::connect(path)?;
        let mut client = Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
            events: VecDeque::new(),
        };
        let hello = client.call_as(Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: name.to_owned(),
        })?;
        Ok((client, hello))
    }

    pub fn call(&mut self, request: Request) -> Result<serde_json::Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        self.writer
            .write_all(encode(&RequestEnvelope { id, request })?.as_bytes())?;
        loop {
            match self.read()? {
                ServerMessage::Response(response) if response.id == Some(id) => {
                    return match response.outcome {
                        Outcome::Result(value) => Ok(value),
                        Outcome::Error(error) => Err(error.into()),
                    };
                }
                ServerMessage::Response(response) => match response.outcome {
                    // A reply without our id means the daemon could not parse the request.
                    Outcome::Error(error) if response.id.is_none() => return Err(error.into()),
                    _ => {}
                },
                ServerMessage::Event(event) => self.events.push_back(event),
            }
        }
    }

    pub fn call_as<T: DeserializeOwned>(&mut self, request: Request) -> Result<T, ClientError> {
        Ok(serde_json::from_value(self.call(request)?)?)
    }

    /// Next event, including ones that arrived while waiting for a reply.
    pub fn next_event(&mut self) -> Result<Event, ClientError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        loop {
            if let ServerMessage::Event(event) = self.read()? {
                return Ok(event);
            }
        }
    }

    fn read(&mut self) -> Result<ServerMessage, ClientError> {
        let line = read_protocol_line(&mut self.reader)?.ok_or(ClientError::Closed)?;
        Ok(serde_json::from_str(&line)?)
    }
}
