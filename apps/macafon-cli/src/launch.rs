//! Launch the packaged sibling executable, falling back to PATH.

use anyhow::{Context, Result};
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub(super) fn run(name: &str, args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(&file)))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| file.into());
    let mut command = std::process::Command::new(&program);
    command.args(args);
    #[cfg(unix)]
    {
        let error = command.exec();
        Err(error).with_context(|| format!("cannot start {}", program.display()))
    }
    // No exec on Windows: run it and pass its exit code on.
    #[cfg(windows)]
    {
        let status = command
            .status()
            .with_context(|| format!("cannot start {}", program.display()))?;
        std::process::exit(status.code().unwrap_or(1))
    }
}
