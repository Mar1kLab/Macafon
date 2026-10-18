//! Bounded daemon logs. This writer is only used outside the audio data loop.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
// The active log and four backups together occupy at most 5 × 2 MiB.
const BACKUPS: usize = 4;

pub fn init() {
    // Decoders note every odd container atom of every probed sound: a large library would push
    // the useful lines out of the size-capped log.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,symphonia=error".into());
    let file = log_path().and_then(|path| LogWriter::open(path, MAX_BYTES));
    let file_layer = match file {
        Ok(writer) => Some(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(writer),
        ),
        Err(error) => {
            eprintln!("Cannot open daemon log; using stderr only: {error}");
            None
        }
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(io::stderr),
        )
        .with(file_layer)
        .init();
}

/// `%LOCALAPPDATA%\Macafon\macafond.log`.
#[cfg(windows)]
fn log_path() -> io::Result<PathBuf> {
    macafon_config::state_dir()
        .map(|directory| directory.join("macafond.log"))
        .map_err(io::Error::other)
}

#[cfg(not(windows))]
fn log_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .ok_or_else(|| io::Error::other("neither XDG_STATE_HOME nor HOME is set"))?;
    Ok(base.join("macafon/macafond.log"))
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<RotatingLog>>);

impl LogWriter {
    fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        let directory = path.parent().unwrap_or(Path::new("."));
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(directory)?;
        let file = open_log(&path)?;
        let bytes = file.metadata()?.len();
        Ok(Self(Arc::new(Mutex::new(RotatingLog {
            path,
            file: Some(file),
            bytes,
            max_bytes,
        }))))
    }
}

fn open_log(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    // `%LOCALAPPDATA%` is already private to the user on Windows.
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

struct RotatingLog {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
}

impl RotatingLog {
    fn backup(&self, index: usize) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        for index in (1..=BACKUPS).rev() {
            let source = if index == 1 {
                self.path.clone()
            } else {
                self.backup(index - 1)
            };
            match std::fs::rename(source, self.backup(index)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        self.bytes = 0;
        Ok(())
    }

    fn write_event(&mut self, event: &[u8]) -> io::Result<()> {
        // Bound even unusually large diagnostic messages. Keep a complete line on disk.
        let limit = self.max_bytes as usize;
        let truncated = event.len() > limit;
        let length = event.len().min(limit);
        if self.bytes.saturating_add(length as u64) > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.file = Some(open_log(&self.path)?);
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("log is closed"))?;
        // Account for partial writes too, so an I/O failure cannot defeat the size limit.
        if truncated {
            self.bytes += length as u64;
            let mut end = length.saturating_sub(1);
            while end > 0 && event[end] & 0xc0 == 0x80 {
                end -= 1;
            }
            file.write_all(&event[..end])?;
            file.write_all(b"\n")
        } else {
            self.bytes += length as u64;
            file.write_all(event)
        }
    }
}

struct EventWriter {
    log: LogWriter,
    event: Vec<u8>,
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = EventWriter;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            log: self.clone(),
            event: Vec::new(),
        }
    }
}

impl Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // Formatters can write an event in pieces; rotate once for the whole event.
        let remaining = (MAX_BYTES as usize + 1).saturating_sub(self.event.len());
        self.event
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if self.event.is_empty() {
            return;
        }
        let result = self
            .log
            .0
            .lock()
            .map_err(|_| io::Error::other("log lock is poisoned"))
            .and_then(|mut log| log.write_event(&self.event));
        if let Err(error) = result {
            eprintln!("Cannot write daemon log: {error}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn logs_append_across_restarts_and_rotate_with_a_total_size_bound() {
        let directory = std::env::temp_dir().join(format!("macafon-log-{}", std::process::id()));
        let path = directory.join("macafond.log");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, b"old\n").unwrap();
        let writer = LogWriter::open(path.clone(), 8).unwrap();
        {
            let mut event = writer.make_writer();
            event.write_all(b"new").unwrap();
            event.write_all(b"\n").unwrap();
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"old\nnew\n");
        drop(writer);
        let writer = LogWriter::open(path.clone(), 8).unwrap();
        for index in 0..6 {
            writeln!(writer.make_writer(), "event-{index}").unwrap();
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"event-5\n");
        for index in 1..=BACKUPS {
            let backup = path.with_file_name(format!("macafond.log.{index}"));
            assert_eq!(
                std::fs::read(&backup).unwrap(),
                format!("event-{}\n", 5 - index).as_bytes()
            );
        }
        assert!(!path.with_file_name("macafond.log.5").exists());
        {
            let mut log = writer.0.lock().unwrap();
            log.write_event(b"oversized diagnostic\n").unwrap();
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"oversiz\n");
        writer
            .0
            .lock()
            .unwrap()
            .write_event("abcdefé diagnostic\n".as_bytes())
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "abcdef\n");
        drop(writer);
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(LogWriter::open(path.clone(), 1024).unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(reason = "test", "Persistent diagnostic");
        });
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Persistent diagnostic"));
        assert!(text.contains("reason=\"test\""));
        assert!(!text.contains('\u{1b}'));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
