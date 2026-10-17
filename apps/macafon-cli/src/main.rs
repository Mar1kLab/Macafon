//! Open Macafon or control its daemon from scripts and desktop hotkeys.

mod args;
mod control;
mod launch;
mod output;

use anyhow::Result;
use args::{Cli, Top};
use clap::Parser;

fn main() -> Result<()> {
    let (json, command) = match Cli::parse().command {
        None => return launch::run("macafon-ui", []),
        Some(Top::Daemon { args }) => {
            return launch::run("macafond", std::iter::once("run".into()).chain(args));
        }
        Some(Top::RestoreDefault) => return restore_defaults(),
        Some(Top::QuickPanel) => {
            return launch::run("macafon-ui", [macafon_ipc::QUICK_PANEL_ARG.into()]);
        }
        Some(Top::Ctl { json, command }) => (json, command),
    };
    control::run(json, command)
}

#[cfg(target_os = "linux")]
fn restore_defaults() -> Result<()> {
    let journal = macafon_backend_pipewire::default_source_journal()?;
    macafon_backend_pipewire::restore_default_source(&journal)?;
    let journal = macafon_backend_pipewire::default_sink_journal()?;
    macafon_backend_pipewire::restore_default_sink(&journal)?;
    Ok(())
}

/// The default microphone from the journal (WIN-MIC-005); outputs are not changed on Windows
/// yet.
#[cfg(windows)]
fn restore_defaults() -> Result<()> {
    let _com = macafon_backend_windows::ComGuard::new()?;
    macafon_backend_windows::restore_default_microphone(&macafon_config::state_dir()?)?;
    Ok(())
}
