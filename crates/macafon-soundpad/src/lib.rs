//! Portable sound library, background decoder, and two-output real-time playback.
pub mod library;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid file or category name: {0}")]
    InvalidName(String),
    #[error("invalid sound id: {0}")]
    InvalidId(String),
    #[error("unsupported audio format")]
    UnsupportedFormat,
    #[error("file exceeds the 200 MiB limit")]
    TooLarge,
    #[error("sound exceeds the 600 second limit")]
    TooLong,
    #[error("audio decode failed: {0}")]
    Decode(String),
    #[error("sound loader worker is unavailable")]
    WorkerUnavailable,
}

#[cfg(test)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
