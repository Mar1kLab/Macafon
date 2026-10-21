//! Display profile state is owned by the daemon control loop.

use super::DaemonHost;
use macafon_ipc::{ErrorBody, ErrorCode, Request};

impl DaemonHost {
    pub(super) fn displays_available(&self) -> bool {
        cfg!(windows)
    }

    pub(super) fn next_display_profile(&self) -> Option<String> {
        #[cfg(windows)]
        {
            next_profile(
                &self.config.displays.profiles,
                self.displays_state().active.as_deref(),
            )
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    #[cfg(not(windows))]
    pub(super) fn respond_displays(
        &mut self,
        _request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        Err(ErrorBody::new(
            ErrorCode::NotFound,
            "display profiles are not available on this platform",
        ))
    }
}

#[cfg(any(windows, test))]
use macafon_ipc::{DisplayMonitor, DisplayProfile, ProfileMonitor};

#[cfg(any(windows, test))]
fn next_profile(profiles: &[DisplayProfile], active: Option<&str>) -> Option<String> {
    if profiles.is_empty() {
        return None;
    }
    let index = active
        .and_then(|id| profiles.iter().position(|profile| profile.id == id))
        .map_or(0, |index| (index + 1) % profiles.len());
    Some(profiles[index].id.clone())
}

/// ADR-0006 identity precedence; ambiguous EDIDs must never select an arbitrary monitor.
#[cfg(any(windows, test))]
fn missing_monitors(profile: &[ProfileMonitor], connected: &[DisplayMonitor]) -> Vec<String> {
    profile
        .iter()
        .filter(|saved| {
            let id = &saved.identity;
            let path = !id.path.is_empty()
                && connected
                    .iter()
                    .any(|m| m.identity.path.eq_ignore_ascii_case(&id.path));
            let serial = id.serial.as_ref().is_some_and(|serial| {
                connected.iter().any(|m| {
                    m.identity.edid.eq_ignore_ascii_case(&id.edid)
                        && m.identity.serial.as_ref() == Some(serial)
                })
            });
            let unique = connected
                .iter()
                .filter(|m| m.identity.edid.eq_ignore_ascii_case(&id.edid))
                .count()
                == 1;
            !(path || serial || unique)
        })
        .map(|saved| saved.name.clone())
        .collect()
}

#[cfg(any(windows, test))]
fn validate_profile(
    profile: &mut DisplayProfile,
    profiles: &[DisplayProfile],
    updating: bool,
) -> Result<(), ErrorBody> {
    profile.name = profile.name.trim().to_owned();
    if profile.name.is_empty() || profile.name.chars().count() > 40 {
        return Err(ErrorBody::new(
            ErrorCode::InvalidParams,
            "display profile name must contain 1 to 40 characters",
        ));
    }
    if profiles.iter().any(|other| {
        (!updating || other.id != profile.id)
            && other.name.to_lowercase() == profile.name.to_lowercase()
    }) {
        return Err(ErrorBody::new(
            ErrorCode::InvalidParams,
            "display profile name is already used",
        ));
    }
    if !macafon_ipc::DISPLAY_PROFILE_ICONS.contains(&profile.icon.as_str()) {
        return Err(ErrorBody::new(
            ErrorCode::InvalidParams,
            "unknown display profile icon",
        ));
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn delete_profile(config: &mut macafon_config::Config, id: &str) {
    config.displays.profiles.retain(|profile| profile.id != id);
    config.displays.confirmed.remove(id);
    let action = format!("display_profile.{id}");
    config.hotkeys.remove(&action);
    config.hotkeys.remove(&format!("{action}#secondary"));
}

#[cfg(any(windows, test))]
fn needs_confirmation(config: &macafon_config::DisplaysConfig, id: &str, set_key: &str) -> bool {
    !config.is_confirmed(id, set_key)
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
mod windows {
    use super::super::{ControlMessage, reply};
    use super::*;
    use macafon_backend::ControlSender;
    use macafon_backend_windows::displays as backend;
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
            backend::DisplayError::Windows(_) => ErrorCode::Internal,
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
                        missing: missing_monitors(&profile.monitors, &self.displays.monitors),
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
            let snapshot =
                self.displays.previous.as_ref().ok_or_else(|| {
                    ErrorBody::new(ErrorCode::NotFound, "no previous display layout")
                })?;
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
            if let Err(error) = backend::apply(&profile.monitors) {
                self.refresh_displays();
                return Err(display_error(error));
            }
            // Replacing a pending apply retains the layout before the first apply.
            if self.displays.pending.is_none() {
                self.displays.previous = Some(snapshot);
            }
            self.displays.pending = None;
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
                    .min(u128::from(u64::MAX - 15_000))
                    as u64
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
                    super::native::show(control.clone(), generation, deadline, cancelled);
                }
                self.displays_changed();
                Ok(())
            } else {
                let result = self.display_output(profile.output_device);
                self.displays_changed();
                result
            }
        }
        pub(in crate::host) fn respond_displays(
            &mut self,
            request: Request,
        ) -> Result<serde_json::Value, ErrorBody> {
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
                    if let Err(error) = backend::all_on() {
                        self.refresh_displays();
                        return Err(display_error(error));
                    }
                    if self.displays.pending.is_none() {
                        self.displays.previous = Some(snapshot);
                    }
                    self.displays.pending = None;
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
}
#[cfg(windows)]
pub(crate) use windows::Runtime;

#[cfg(test)]
mod tests {
    use super::*;
    use macafon_ipc::DisplayIdentity;

    fn profile(id: &str) -> DisplayProfile {
        DisplayProfile {
            id: id.into(),
            name: id.into(),
            icon: "monitor".into(),
            monitors: Vec::new(),
            output_device: None,
        }
    }
    fn monitor(edid: &str, serial: Option<&str>, path: &str) -> DisplayMonitor {
        DisplayMonitor {
            identity: DisplayIdentity {
                edid: edid.into(),
                serial: serial.map(str::to_owned),
                path: path.into(),
            },
            name: path.into(),
            built_in: false,
            layout: None,
            modes: Vec::new(),
            hdr_supported: false,
        }
    }
    #[test]
    fn confirmation_is_specific_to_profile_and_monitor_set() {
        let mut config = macafon_config::DisplaysConfig::default();
        config.confirmed.insert("work".into(), vec!["a,b".into()]);
        assert!(!needs_confirmation(&config, "work", "a,b"));
        assert!(needs_confirmation(&config, "work", "a"));
        assert!(needs_confirmation(&config, "play", "a,b"));
    }
    #[test]
    fn cycling_starts_at_first_and_wraps_after_active() {
        let profiles = [profile("a"), profile("b")];
        assert_eq!(next_profile(&[], None), None);
        assert_eq!(next_profile(&profiles, None).as_deref(), Some("a"));
        assert_eq!(
            next_profile(&profiles, Some("unknown")).as_deref(),
            Some("a")
        );
        assert_eq!(next_profile(&profiles, Some("a")).as_deref(), Some("b"));
        assert_eq!(next_profile(&profiles, Some("b")).as_deref(), Some("a"));
    }
    #[test]
    fn missing_names_resolve_paths_serials_and_only_unique_edids() {
        let connected = [
            monitor("same", Some("one"), "port1"),
            monitor("same", Some("two"), "port2"),
            monitor("unique", None, "port3"),
        ];
        let saved = [
            (monitor("changed", None, "PORT1"), "path"),
            (monitor("same", Some("two"), "old"), "serial"),
            (monitor("unique", None, "old"), "unique"),
            (monitor("same", None, "old"), "ambiguous"),
            (monitor("missing", None, "old"), "unplugged"),
        ]
        .into_iter()
        .map(|(monitor, name)| ProfileMonitor {
            identity: monitor.identity,
            name: name.into(),
            layout: None,
        })
        .collect::<Vec<_>>();
        assert_eq!(
            missing_monitors(&saved, &connected),
            ["ambiguous", "unplugged"]
        );
    }
    #[test]
    fn deleting_removes_both_hotkeys_and_confirmation_only_for_that_profile() {
        let mut config = macafon_config::Config::default();
        config.displays.profiles = vec![profile("a"), profile("b")];
        for id in ["a", "b"] {
            config
                .displays
                .confirmed
                .insert(id.into(), vec!["set".into()]);
            config
                .hotkeys
                .insert(format!("display_profile.{id}"), "F7".into());
            config
                .hotkeys
                .insert(format!("display_profile.{id}#secondary"), "F8".into());
        }
        delete_profile(&mut config, "a");
        assert_eq!(config.displays.profiles, [profile("b")]);
        assert!(!config.displays.confirmed.contains_key("a"));
        assert!(!config.hotkeys.contains_key("display_profile.a"));
        assert!(!config.hotkeys.contains_key("display_profile.a#secondary"));
        assert!(config.hotkeys.contains_key("display_profile.b"));
        assert!(config.displays.confirmed.contains_key("b"));
    }
    #[test]
    fn names_are_trimmed_bounded_and_case_insensitively_unique_and_icons_validated() {
        let mut candidate = profile("new");
        candidate.name = "  Work  ".into();
        validate_profile(&mut candidate, &[], false).expect("valid profile name");
        assert_eq!(candidate.name, "Work");
        let existing = [candidate.clone()];
        assert!(validate_profile(&mut candidate, &existing, true).is_ok());
        candidate.id = "other".into();
        candidate.name = "wOrK".into();
        assert!(validate_profile(&mut candidate, &existing, false).is_err());
        for name in [" ".to_owned(), "e".repeat(41)] {
            candidate.name = name;
            assert!(validate_profile(&mut candidate, &[], false).is_err());
        }
        candidate.name = "e".repeat(40);
        assert!(validate_profile(&mut candidate, &[], false).is_ok());
        candidate.icon = "invalid".into();
        assert!(validate_profile(&mut candidate, &[], false).is_err());
    }
}
