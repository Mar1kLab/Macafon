//! Launch the packaged sibling executable, falling back to PATH.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;

pub(super) fn run(name: &str, args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .filter(|path| path.is_file());
    let program = sibling.unwrap_or_else(|| name.into());
    let error = std::process::Command::new(&program).args(args).exec();
    Err(error).with_context(|| format!("cannot start {}", program.display()))
}
