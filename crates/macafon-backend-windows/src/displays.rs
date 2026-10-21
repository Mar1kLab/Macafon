//! Display profiles (PRD §8.5, ADR-0006): monitor layouts through the Windows CCD API
//! (`QueryDisplayConfig` / `SetDisplayConfig`). No administrator rights are needed.
//!
//! Profiles store `DisplayIdentity` (EDID + device path), never adapter LUIDs or target ids:
//! those change after a reboot or a driver update. Every apply resolves the profile against
//! the monitors connected now.

use macafon_ipc::{DisplayMonitor, ProfileMonitor};

#[derive(Debug, thiserror::Error)]
pub enum DisplayError {
    #[error("Windows display call failed: {0}")]
    Windows(#[from] windows::core::Error),
    /// `SDC_VALIDATE` rejected the layout, such as a mode the monitor or cable cannot do.
    #[error("Windows rejected this layout: {0}")]
    Rejected(String),
    /// None of the profile's monitors that would be on are connected (DISP-007).
    #[error("none of this profile's monitors are connected")]
    NothingToTurnOn,
}

/// The whole display configuration as it was, to go back to (DISP-004/005).
#[derive(Debug, Clone)]
pub struct Snapshot {
    _private: (),
}

/// What `apply` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Names of profile monitors that are not connected and were skipped (DISP-007).
    pub missing: Vec<String>,
}

/// Every connected monitor, active or not.
pub fn monitors() -> Result<Vec<DisplayMonitor>, DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}

/// The current layout of every connected monitor, as a profile would store it.
pub fn capture() -> Result<Vec<ProfileMonitor>, DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}

pub fn snapshot() -> Result<Snapshot, DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}

pub fn restore(_snapshot: &Snapshot) -> Result<(), DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}

/// Validate, then apply a profile's monitors; connected monitors not in the profile turn off.
pub fn apply(_monitors: &[ProfileMonitor]) -> Result<Applied, DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}

/// Every connected monitor on, extended, at its preferred mode (DISP-005).
pub fn all_on() -> Result<(), DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}

/// Whether the connected monitors are laid out as `profile` says: its connected monitors in
/// their layouts, every other connected monitor off. Missing monitors are ignored.
pub fn matches(_profile: &[ProfileMonitor], _monitors: &[DisplayMonitor]) -> bool {
    false
}

/// First-run profiles (§9.10): "All monitors" (extended, left to right), then each monitor
/// alone as the primary at its preferred mode. `(name, icon, monitors)`.
pub fn suggestions(
    _monitors: &[DisplayMonitor],
) -> Vec<(String, &'static str, Vec<ProfileMonitor>)> {
    Vec::new()
}

/// Calls `on_change` from its own thread when the layout or the connected monitors change
/// (`WM_DISPLAYCHANGE`, device arrival/removal); stops when dropped.
pub struct Watcher {
    _private: (),
}

pub fn watch(_on_change: impl Fn() + Send + 'static) -> Result<Watcher, DisplayError> {
    Err(DisplayError::Rejected("not implemented yet".into()))
}
