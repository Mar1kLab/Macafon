//! Bounded length-prefixed frames shared by the daemon and worker.
use crate::protocol::{KIND_AUDIO, KIND_JSON, MAX_FRAME_BYTES};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

#[derive(Debug, PartialEq)]
pub enum Frame<T> {
    Json(T),
    Audio(Vec<f32>),
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<Option<Frame<T>>> {
    let mut length = [0; 4];
    // Distinguish clean EOF from a truncated header.
    loop {
        match reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_le_bytes(length);
    if !(1..=MAX_FRAME_BYTES).contains(&length) {
        return Err(invalid("frame length outside allowed bounds"));
    }
    let mut body = vec![0; length as usize];
    reader.read_exact(&mut body)?;
    match body[0] {
        KIND_JSON => serde_json::from_slice(&body[1..])
            .map(Frame::Json)
            .map(Some)
            .map_err(|e| invalid(e.to_string())),
        KIND_AUDIO if (length - 1) % 4 == 0 => {
            let samples = body[1..]
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect::<Vec<_>>();
            if samples.iter().any(|sample| !sample.is_finite()) {
                return Err(invalid("non-finite audio sample"));
            }
            Ok(Some(Frame::Audio(samples)))
        }
        _ => Err(invalid("unknown frame kind or malformed audio")),
    }
}

fn write_body(writer: &mut impl Write, kind: u8, body: &[u8]) -> io::Result<()> {
    if body.len() >= MAX_FRAME_BYTES as usize {
        return Err(invalid("frame too large"));
    }
    writer.write_all(&((body.len() + 1) as u32).to_le_bytes())?;
    writer.write_all(&[kind])?;
    writer.write_all(body)?;
    writer.flush()
}

pub fn write_json(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?;
    write_body(writer, KIND_JSON, &body)
}

pub fn write_audio(writer: &mut impl Write, samples: &[f32]) -> io::Result<()> {
    if samples.len() > (MAX_FRAME_BYTES as usize - 1) / 4 || samples.iter().any(|s| !s.is_finite())
    {
        return Err(invalid("invalid audio frame"));
    }
    let body: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    write_body(writer, KIND_AUDIO, &body)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::protocol::WorkerRequest;

    #[test]
    fn round_trip_and_reject_truncated_or_oversized_frames() {
        let mut bytes = Vec::new();
        write_json(&mut bytes, &WorkerRequest::Begin).unwrap();
        write_audio(&mut bytes, &[0.2, -0.4, 0.0]).unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(
            read_frame::<WorkerRequest>(&mut reader).unwrap(),
            Some(Frame::Json(WorkerRequest::Begin))
        );
        assert_eq!(
            read_frame::<WorkerRequest>(&mut reader).unwrap(),
            Some(Frame::Audio(vec![0.2, -0.4, 0.0]))
        );
        assert_eq!(read_frame::<WorkerRequest>(&mut reader).unwrap(), None);
        assert!(read_frame::<WorkerRequest>(&mut &bytes[..2]).is_err());
        assert!(
            read_frame::<WorkerRequest>(&mut &(MAX_FRAME_BYTES + 1).to_le_bytes()[..]).is_err()
        );
    }
}
