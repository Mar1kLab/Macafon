//! Linux integration follows the Windows control-loop safety flow; the native API stays separate.
use super::super::{ControlMessage, reply};
use super::*;
use macafon_backend::ControlSender;
use macafon_displays_linux as backend;
use macafon_ipc::{DisplayProfileInfo, DisplaysPending, DisplaysState, Event};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub(crate) struct Runtime {
    monitors: Vec<DisplayMonitor>,
    previous: Option<backend::Snapshot>,
    pending: Option<Pending>,
    generation: u64,
    control: Option<ControlSender<ControlMessage>>,
    watcher: Option<backend::Watcher>,
}
struct Pending {
    state: DisplaysPending,
    deadline: Instant,
    generation: u64,
    set_key: String,
    output_device: Option<String>,
    applied_monitors: Vec<ProfileMonitor>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
fn display_error(error: backend::DisplayError) -> ErrorBody {
    let code = match error {
        backend::DisplayError::Unavailable(_) => ErrorCode::Internal,
        backend::DisplayError::Rejected(_) | backend::DisplayError::NothingToTurnOn => {
            ErrorCode::InvalidParams
        }
    };
    ErrorBody::new(code, error.to_string())
}
fn not_found() -> ErrorBody {
    ErrorBody::new(ErrorCode::NotFound, "display profile not found")
}

impl DaemonHost {
    pub(crate) fn start_displays(&mut self, control: ControlSender<ControlMessage>) {
        self.displays.control = Some(control.clone());
        if !self.displays_available() {
            return;
        }
        self.refresh_displays();
        match backend::watch(move || {
            let _ = control.send(ControlMessage::DisplaysChanged);
        }) {
            Ok(watcher) => self.displays.watcher = Some(watcher),
            Err(error) => tracing::warn!(%error, "Cannot watch displays"),
        }
    }
    pub(crate) fn stop_displays(&mut self) {
        if self.displays.pending.is_some()
            && let Err(error) = self.confirm_displays(false)
        {
            tracing::warn!(%error, "Cannot restore unconfirmed displays during shutdown");
        }
        self.displays.watcher = None;
    }
    pub(crate) fn refresh_displays(&mut self) {
        match backend::monitors() {
            Ok(monitors) => self.displays.monitors = monitors,
            Err(error) => tracing::warn!(%error, "Cannot refresh displays"),
        }
        self.displays_changed();
    }
    pub(crate) fn displays_state(&self) -> DisplaysState {
        let profiles = &self.config.displays.profiles;
        DisplaysState {
            monitors: self.displays.monitors.clone(),
            profiles: profiles
                .iter()
                .map(|profile| DisplayProfileInfo {
                    missing: backend::missing_monitors(&profile.monitors, &self.displays.monitors),
                    profile: profile.clone(),
                })
                .collect(),
            active: profiles
                .iter()
                .find(|profile| backend::matches(&profile.monitors, &self.displays.monitors))
                .map(|profile| profile.id.clone()),
            pending: self
                .displays
                .pending
                .as_ref()
                .map(|pending| pending.state.clone()),
            can_revert: self.displays.previous.is_some(),
        }
    }
    fn displays_changed(&mut self) {
        self.hub
            .broadcast(&Event::DisplaysChanged(self.displays_state()));
        self.update_tray();
    }
    fn displays_config_changed(&mut self) {
        self.changed_at = Some(Instant::now());
        self.rebind_hotkeys();
        self.hotkeys_changed();
        self.displays_changed();
    }
    pub(crate) fn tick_displays(&mut self) {
        if let Some(pending) = &self.displays.pending
            && Instant::now() >= pending.deadline
        {
            let generation = pending.generation;
            self.display_decision(generation, false);
        }
    }
    pub(crate) fn display_decision(&mut self, generation: u64, keep: bool) {
        if self
            .displays
            .pending
            .as_ref()
            .is_none_or(|p| p.generation != generation)
        {
            return;
        }
        if let Err(error) = self.confirm_displays(keep) {
            tracing::warn!(%error, "Cannot resolve display confirmation");
        }
    }
    fn display_output(&mut self, device: Option<String>) -> Result<(), ErrorBody> {
        if let Some(device) = device {
            self.respond_mixer(Request::MixerSetOutput {
                device: Some(device),
            })?;
        }
        Ok(())
    }
    fn confirm_displays(&mut self, keep: bool) -> Result<(), ErrorBody> {
        if keep
            && self
                .displays
                .pending
                .as_ref()
                .is_some_and(|p| Instant::now() < p.deadline)
        {
            backend::keep().map_err(display_error)?;
        }
        let Some(pending) = self.displays.pending.take() else {
            return Ok(());
        };
        // A late UI/native Keep can never override the timeout.
        let keep = keep && Instant::now() < pending.deadline;
        if keep {
            if let Some(id) = &pending.state.profile
                && self
                    .config
                    .displays
                    .profile(id)
                    .is_some_and(|profile| profile.monitors == pending.applied_monitors)
            {
                let sets = self
                    .config
                    .displays
                    .confirmed
                    .entry(id.clone())
                    .or_default();
                if !sets.contains(&pending.set_key) {
                    sets.push(pending.set_key.clone());
                }
                self.changed_at = Some(Instant::now());
            }
            let result = self.display_output(pending.output_device.clone());
            self.displays_changed();
            result
        } else {
            self.restore_displays()
        }
    }
    fn restore_displays(&mut self) -> Result<(), ErrorBody> {
        let snapshot = self
            .displays
            .previous
            .as_ref()
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "no previous display layout"))?;
        // Retain the snapshot on failure, so the safe fallback remains available.
        let result = backend::restore(snapshot).map_err(display_error);
        if result.is_ok() {
            self.displays.previous = None;
            self.displays.pending = None;
        }
        self.refresh_displays();
        result
    }
    fn apply_display_profile(&mut self, id: &str) -> Result<(), ErrorBody> {
        let profile = self
            .config
            .displays
            .profile(id)
            .cloned()
            .ok_or_else(not_found)?;
        let snapshot = backend::snapshot().map_err(display_error)?;
        // Replacing a pending apply retains the layout before the first apply.
        if self.displays.pending.is_none() {
            self.displays.previous = Some(snapshot);
        }
        self.displays.pending = None;
        let result = apply_with_rollback(
            &mut self.displays.previous,
            || backend::apply(&profile.monitors).map(|_| ()),
            |snapshot| {
                let result = backend::restore(snapshot);
                if let Err(error) = &result {
                    tracing::warn!(%error, "Cannot restore partially applied display profile");
                }
                result
            },
        );
        if let Err(error) = result {
            self.refresh_displays();
            return Err(display_error(error));
        }
        match backend::monitors() {
            Ok(monitors) => self.displays.monitors = monitors,
            Err(error) => {
                // Never trust a cached monitor set to bypass confirmation after an apply.
                let error = display_error(error);
                if let Err(restore_error) = self.restore_displays() {
                    tracing::warn!(%restore_error, "Cannot restore displays after enumeration failed");
                }
                return Err(error);
            }
        }
        let set_key = macafon_config::displays::monitor_set_key(
            self.displays.monitors.iter().map(|m| &m.identity),
        );
        if needs_confirmation(&self.config.displays, id, &set_key) {
            self.displays.generation += 1;
            let generation = self.displays.generation;
            let cancelled = Arc::new(AtomicBool::new(false));
            let deadline = Instant::now() + Duration::from_secs(15);
            let revert_at_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX - 15_000)) as u64
                + 15_000;
            self.displays.pending = Some(Pending {
                state: DisplaysPending {
                    profile: Some(id.to_owned()),
                    revert_at_unix_ms,
                },
                deadline,
                generation,
                set_key,
                output_device: profile.output_device,
                applied_monitors: profile.monitors,
                cancelled: cancelled.clone(),
            });
            if let Some(control) = &self.displays.control {
                super::linux_native::show(control.clone(), generation, deadline, cancelled);
            }
            self.displays_changed();
            Ok(())
        } else {
            backend::keep().map_err(display_error)?;
            let result = self.display_output(profile.output_device);
            self.displays_changed();
            result
        }
    }
    pub(in crate::host) fn respond_displays(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        if !self.displays_available() {
            return Err(ErrorBody::new(
                ErrorCode::NotFound,
                "no supported Linux display backend detected",
            ));
        }
        match request {
            Request::DisplaysGet => return reply(self.displays_state()),
            Request::DisplaysSuggestions => {
                let mut config = self.config.displays.clone();
                let suggestions: Vec<_> = backend::suggestions(&self.displays.monitors)
                    .into_iter()
                    .map(|(name, icon, monitors)| {
                        let profile = DisplayProfile {
                            id: config.next_id(&name),
                            name,
                            icon: icon.into(),
                            monitors,
                            output_device: None,
                        };
                        config.profiles.push(profile.clone());
                        profile
                    })
                    .collect();
                return reply(suggestions);
            }
            Request::DisplaysSave { name, icon } => {
                let mut profile = DisplayProfile {
                    id: String::new(),
                    name,
                    icon,
                    monitors: Vec::new(),
                    output_device: None,
                };
                validate_profile(&mut profile, &self.config.displays.profiles, false)?;
                profile.monitors = backend::capture().map_err(display_error)?;
                profile.id = self.config.displays.next_id(&profile.name);
                self.config.displays.profiles.push(profile);
            }
            Request::DisplaysCreate { mut profile } => {
                validate_profile(&mut profile, &self.config.displays.profiles, false)?;
                profile.id = self.config.displays.next_id(&profile.name);
                self.config.displays.profiles.push(profile);
            }
            Request::DisplaysUpdate { mut profile } => {
                let index = self
                    .config
                    .displays
                    .profiles
                    .iter()
                    .position(|p| p.id == profile.id)
                    .ok_or_else(not_found)?;
                validate_profile(&mut profile, &self.config.displays.profiles, true)?;
                // A changed layout must be confirmed again, even on a familiar monitor set.
                if profile.monitors != self.config.displays.profiles[index].monitors {
                    self.config.displays.confirmed.remove(&profile.id);
                }
                self.config.displays.profiles[index] = profile;
            }
            Request::DisplaysRecapture { id } => {
                let index = self
                    .config
                    .displays
                    .profiles
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or_else(not_found)?;
                let monitors = backend::capture().map_err(display_error)?;
                self.config.displays.profiles[index].monitors = monitors;
                self.config.displays.confirmed.remove(&id);
            }
            Request::DisplaysDelete { id } => {
                if self.config.displays.profile(&id).is_none() {
                    return Err(not_found());
                }
                if self
                    .displays
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.state.profile.as_deref() == Some(&id))
                {
                    self.confirm_displays(false)?;
                }
                delete_profile(&mut self.config, &id);
            }
            Request::DisplaysReorder { ids } => {
                let profiles = &self.config.displays.profiles;
                let mut ordered = Vec::with_capacity(profiles.len());
                for id in &ids {
                    let profile = profiles
                        .iter()
                        .find(|p| &p.id == id)
                        .ok_or_else(not_found)?;
                    if ordered.iter().any(|p: &DisplayProfile| p.id == *id) {
                        return Err(ErrorBody::new(
                            ErrorCode::InvalidParams,
                            "duplicate display profile id",
                        ));
                    }
                    ordered.push(profile.clone());
                }
                if ordered.len() != profiles.len() {
                    return Err(ErrorBody::new(
                        ErrorCode::InvalidParams,
                        "reorder must include every display profile",
                    ));
                }
                self.config.displays.profiles = ordered;
            }
            Request::DisplaysApply { id } => {
                self.apply_display_profile(&id)?;
                return reply(self.displays_state());
            }
            Request::DisplaysConfirm { keep } => {
                self.confirm_displays(keep)?;
                return reply(self.displays_state());
            }
            Request::DisplaysRevert => {
                self.restore_displays()?;
                return reply(self.displays_state());
            }
            Request::DisplaysAllOn => {
                let snapshot = backend::snapshot().map_err(display_error)?;
                if self.displays.pending.is_none() {
                    self.displays.previous = Some(snapshot);
                }
                self.displays.pending = None;
                let result = apply_with_rollback(
                    &mut self.displays.previous,
                    backend::all_on,
                    backend::restore,
                );
                if let Err(error) = result {
                    self.refresh_displays();
                    return Err(display_error(error));
                }
                // The recovery action must never open another compositor confirmation.
                self.refresh_displays();
                return reply(self.displays_state());
            }
            _ => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "not a display request",
                ));
            }
        }
        self.displays_config_changed();
        reply(self.displays_state())
    }
}
