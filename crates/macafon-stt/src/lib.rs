//! Local speech to text for dictation (PRD §8.4): the worker protocol between `macafond` and
//! the `macafon-stt` worker process, the language policy and, behind features, the engines.
//! No dependency on the daemon or the audio backends.

pub mod manifest;
pub mod protocol;

/// Sample rate of the audio the worker receives: mono f32 at 16 kHz.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
