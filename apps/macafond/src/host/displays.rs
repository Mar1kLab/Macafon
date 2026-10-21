//! Display profiles on the daemon control loop (PRD §8.5).

use super::DaemonHost;
use macafon_ipc::{ErrorBody, ErrorCode, Request};

impl DaemonHost {
    /// Display profiles work where the platform has them (`Capabilities::displays`).
    pub(super) fn displays_available(&self) -> bool {
        false
    }

    /// The profile after the active one, wrapping around (DISP-003).
    pub(super) fn next_display_profile(&self) -> Option<String> {
        None
    }

    pub(super) fn respond_displays(
        &mut self,
        _request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        Err(ErrorBody::new(
            ErrorCode::NotFound,
            "display profiles are not available yet",
        ))
    }
}
