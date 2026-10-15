//! Windows audio backend (PRD §12.2, ADR-0004): WASAPI capture → `macafon-dsp` → VB-CABLE,
//! endpoint discovery, default devices and application audio sessions.
//!
//! M3.0 spike state: the pieces are usable from `examples/spike.rs`; the daemon does not use
//! them yet. Everything compiles to an empty crate on other platforms.
#![cfg(windows)]

mod com;
pub mod devices;
pub mod journal;
pub mod microphone;
pub mod mixer;
mod mixer_journal;
pub mod policy;
pub mod probe;
pub mod sessions;
pub mod wasapi;

pub use com::ComGuard;
pub use devices::{DataFlow, Endpoint, EndpointRole, VirtualCable, endpoints, find_vb_cable};
pub use microphone::{MicrophoneConfig, MicrophonePipeline, PipelineReport};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Windows audio call failed: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
    #[error("device format not supported: {0}")]
    Format(String),
    #[error("restore journal: {0}")]
    Journal(String),
    #[error("audio thread failed: {0}")]
    Thread(String),
}
