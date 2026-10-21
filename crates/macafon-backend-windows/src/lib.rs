//! Windows audio backend (PRD §12.2, ADR-0004): WASAPI capture → `macafon-dsp` → VB-CABLE,
//! endpoint discovery, default devices and application audio sessions.
//!
//! `daemon` drives the daemon's host on Windows; `examples/spike.rs` exercises the pieces by
//! hand. Everything compiles to an empty crate on other platforms.
#![cfg(windows)]

mod com;
pub mod daemon;
pub mod devices;
pub mod diagnostics;
pub mod displays;
mod groups;
pub mod input_level;
pub mod journal;
pub mod macaron;
pub mod microphone;
pub mod mixer;
mod mixer_journal;
mod monitor;
pub mod output;
mod player;
pub mod policy;
pub mod probe;
pub mod sessions;
pub mod wasapi;

pub use com::ComGuard;
pub use daemon::{
    ControlReceiver, control_channel, fix_cable_default_output, output_devices, restore_all,
    restore_default_microphone, run_microphone, set_default_microphone,
};
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
    #[error("microphone level control failed: {0}")]
    InputLevel(String),
    #[error("restore journal: {0}")]
    Journal(String),
    #[error("audio thread failed: {0}")]
    Thread(String),
}
