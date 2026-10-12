//! Bounded newline framing shared by the daemon, blocking client, and desktop bridge.

use crate::MAX_LINE_BYTES;
use std::io::{BufRead, Read};

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("protocol line is too long or missing its terminating newline")]
    Incomplete,
}

/// Read one complete protocol line without buffering more than the protocol limit.
/// EOF between messages is clean; a partial final message is rejected.
pub fn read_protocol_line(reader: &mut impl BufRead) -> Result<Option<String>, FrameError> {
    let mut line = String::new();
    if reader.take(MAX_LINE_BYTES as u64).read_line(&mut line)? == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') {
        return Err(FrameError::Incomplete);
    }
    Ok(Some(line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn framing_rejects_truncated_and_oversized_messages_without_consuming_the_next_line() {
        let mut bytes = vec![b' '; MAX_LINE_BYTES - 1];
        bytes.extend_from_slice(b"\n{}\n");
        let mut reader = Cursor::new(bytes);
        assert_eq!(
            read_protocol_line(&mut reader)
                .expect("bounded line")
                .expect("line")
                .len(),
            MAX_LINE_BYTES
        );
        assert_eq!(
            read_protocol_line(&mut reader)
                .expect("next frame")
                .as_deref(),
            Some("{}\n")
        );
        assert!(
            read_protocol_line(&mut reader)
                .expect("clean EOF")
                .is_none()
        );

        for bytes in [
            b"{}".to_vec(),
            vec![b' '; MAX_LINE_BYTES],
            vec![b' '; MAX_LINE_BYTES + 1],
        ] {
            let mut reader = Cursor::new(bytes);
            assert!(matches!(
                read_protocol_line(&mut reader),
                Err(FrameError::Incomplete)
            ));
            assert!(reader.position() <= MAX_LINE_BYTES as u64);
        }
    }
}
