//! Desktop display control, independent of the PipeWire audio backend.
//! Runtime probing never treats Xwayland's read-only outputs as physical monitors.
#![cfg(target_os = "linux")]

mod cli;
mod identity;
mod logic;
mod mutter;
mod watcher;

pub use logic::suggestions;
use macafon_ipc::{DisplayMonitor, ProfileMonitor};
use std::sync::OnceLock;
pub use watcher::{Watcher, watch};

#[derive(Debug, thiserror::Error)]
pub enum DisplayError {
    #[error("display backend unavailable: {0}")]
    Unavailable(String),
    #[error("display configuration rejected: {0}")]
    Rejected(String),
    #[error("no connected monitor would be enabled")]
    NothingToTurnOn,
}
impl From<zbus::Error> for DisplayError {
    fn from(error: zbus::Error) -> Self {
        Self::Unavailable(error.to_string())
    }
}
impl From<std::io::Error> for DisplayError {
    fn from(error: std::io::Error) -> Self {
        Self::Unavailable(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Mutter,
    Kscreen,
    WlrRandr,
    Xrandr,
}
static BACKEND: OnceLock<Option<Backend>> = OnceLock::new();

/// Probe the actual session API, not just an installed executable. Cached for this daemon run.
pub fn detected_backend() -> Option<Backend> {
    *BACKEND.get_or_init(|| {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland");
        let kde = std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|s| s.split(':').any(|s| s.eq_ignore_ascii_case("kde")));
        let candidates = [
            Some(Backend::Mutter),
            kde.then_some(Backend::Kscreen),
            wayland.then_some(Backend::WlrRandr),
            (!wayland).then_some(Backend::Xrandr),
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|backend| backend.inventory().is_ok_and(|i| !i.monitors.is_empty()))
    })
}
fn backend() -> Result<Backend, DisplayError> {
    detected_backend()
        .ok_or_else(|| DisplayError::Unavailable("no supported desktop session".into()))
}

pub(crate) struct Inventory {
    monitors: Vec<DisplayMonitor>,
    /// Backend mode ids, aligned with each monitor's modes. Never persisted in a profile.
    mode_ids: Vec<Vec<String>>,
    /// Existing scales are preserved; profiles intentionally do not contain scaling.
    scales: Vec<f64>,
    preferred: Vec<Option<macafon_ipc::DisplayMode>>,
    mirror_supported: bool,
}
impl Backend {
    fn inventory(self) -> Result<Inventory, DisplayError> {
        let mut inventory = match self {
            Self::Mutter => mutter::current()?.inventory()?,
            _ => cli::inventory(self)?,
        };
        identity::enrich(&mut inventory.monitors);
        Ok(inventory)
    }
}

pub fn monitors() -> Result<Vec<DisplayMonitor>, DisplayError> {
    Ok(backend()?.inventory()?.monitors)
}
pub fn capture() -> Result<Vec<ProfileMonitor>, DisplayError> {
    Ok(as_profile(&monitors()?))
}
fn as_profile(monitors: &[DisplayMonitor]) -> Vec<ProfileMonitor> {
    monitors
        .iter()
        .map(|m| ProfileMonitor {
            identity: m.identity.clone(),
            name: m.name.clone(),
            layout: m.layout.clone(),
        })
        .collect()
}
pub struct Snapshot {
    backend: Backend,
    monitors: Vec<ProfileMonitor>,
    /// Raw Mutter state preserves scale, transform, color and layout mode for rollback.
    mutter: Option<mutter::State>,
    scales: Vec<f64>,
}
pub fn snapshot() -> Result<Snapshot, DisplayError> {
    let backend = backend()?;
    let state = if backend == Backend::Mutter {
        Some(mutter::current()?)
    } else {
        None
    };
    let inventory = match &state {
        Some(s) => s.inventory()?,
        None => backend.inventory()?,
    };
    Ok(Snapshot {
        backend,
        monitors: as_profile(&inventory.monitors),
        scales: inventory.scales,
        mutter: state,
    })
}
pub fn restore(snapshot: &Snapshot) -> Result<(), DisplayError> {
    if let Some(state) = &snapshot.mutter {
        return mutter::restore(state, &snapshot.monitors, &snapshot.scales);
    }
    let mut inventory = snapshot.backend.inventory()?;
    for (saved, scale) in snapshot.monitors.iter().zip(&snapshot.scales) {
        if let Some(index) = inventory
            .monitors
            .iter()
            .position(|m| m.identity.path == saved.identity.path)
        {
            inventory.scales[index] = *scale;
        }
    }
    let plan = plan(&snapshot.monitors, &inventory)?;
    cli::apply(snapshot.backend, &inventory, &plan, true)?;
    cli::apply(snapshot.backend, &inventory, &plan, false)
}

pub struct Applied {
    pub missing: Vec<String>,
}
struct Plan {
    layouts: Vec<Option<macafon_ipc::DisplayLayout>>,
    groups: Vec<Vec<usize>>,
    missing: Vec<String>,
}
fn plan(profile: &[ProfileMonitor], inventory: &Inventory) -> Result<Plan, DisplayError> {
    let resolved = logic::resolve(profile, &inventory.monitors);
    let connected =
        logic::connected(profile, &resolved).map_err(|e| DisplayError::Rejected(e.into()))?;
    let mut layouts = vec![None; inventory.monitors.len()];
    let mut missing = Vec::new();
    for (i, saved) in profile.iter().enumerate() {
        let Some(j) = resolved[i] else {
            missing.push(saved.name.clone());
            continue;
        };
        if let Some(mut layout) = connected.layouts[i].clone() {
            let mode = inventory.monitors[j]
                .modes
                .iter()
                .filter(|m| logic::mode_matches(**m, layout.mode))
                .min_by_key(|m| m.refresh_mhz.abs_diff(layout.mode.refresh_mhz))
                .ok_or_else(|| {
                    DisplayError::Rejected(format!("unsupported mode on {}", saved.name))
                })?;
            layout.mode = *mode;
            if let Some(path) = &layout.mirror_of {
                layout.mirror_of = profile
                    .iter()
                    .position(|p| p.identity.path.eq_ignore_ascii_case(path))
                    .and_then(|i| resolved[i])
                    .map(|j| inventory.monitors[j].identity.path.clone());
            }
            layout.hdr &= inventory.monitors[j].hdr_supported;
            layouts[j] = Some(layout);
        }
    }
    if layouts.iter().all(Option::is_none) {
        return Err(DisplayError::NothingToTurnOn);
    }
    let groups = connected
        .groups
        .into_iter()
        .map(|g| g.into_iter().filter_map(|i| resolved[i]).collect())
        .collect();
    Ok(Plan {
        layouts,
        groups,
        missing,
    })
}
fn configure(profile: &[ProfileMonitor], verify: bool) -> Result<Applied, DisplayError> {
    let backend = backend()?;
    if backend == Backend::Mutter {
        return mutter::configure(profile, verify);
    }
    let inventory = backend.inventory()?;
    let plan = plan(profile, &inventory)?;
    if !verify {
        cli::apply(backend, &inventory, &plan, true)?;
    }
    cli::apply(backend, &inventory, &plan, verify)?;
    Ok(Applied {
        missing: plan.missing,
    })
}
pub fn validate(profile: &[ProfileMonitor]) -> Result<Applied, DisplayError> {
    configure(profile, true)
}
pub fn apply(profile: &[ProfileMonitor]) -> Result<Applied, DisplayError> {
    configure(profile, false)
}
pub fn all_on() -> Result<(), DisplayError> {
    let inventory = backend()?.inventory()?;
    let profile = all_on_profile(&inventory)?;
    apply(&profile).map(|_| ())
}
fn all_on_profile(inventory: &Inventory) -> Result<Vec<ProfileMonitor>, DisplayError> {
    let mut x = 0i32;
    let mut profile = Vec::new();
    let primary = inventory
        .monitors
        .iter()
        .position(|m| m.layout.as_ref().is_some_and(|l| l.primary))
        .unwrap_or(0);
    for (i, m) in inventory.monitors.iter().enumerate() {
        let mode = inventory.preferred[i]
            .or_else(|| m.modes.first().copied())
            .ok_or_else(|| DisplayError::Rejected(format!("no supported mode on {}", m.name)))?;
        let rotation = m
            .layout
            .as_ref()
            .map_or(macafon_ipc::DisplayRotation::None, |l| l.rotation);
        let width = if matches!(
            rotation,
            macafon_ipc::DisplayRotation::Rotate90 | macafon_ipc::DisplayRotation::Rotate270
        ) {
            mode.height
        } else {
            mode.width
        };
        let logical_width = (f64::from(width) / inventory.scales[i]).ceil();
        if !logical_width.is_finite() || logical_width > f64::from(i32::MAX) {
            return Err(DisplayError::Rejected("desktop width overflow".into()));
        }
        profile.push(ProfileMonitor {
            identity: m.identity.clone(),
            name: m.name.clone(),
            layout: Some(macafon_ipc::DisplayLayout {
                x,
                y: 0,
                mode,
                rotation,
                primary: i == primary,
                mirror_of: None,
                hdr: m.layout.as_ref().is_some_and(|l| l.hdr),
            }),
        });
        x = x
            .checked_add(logical_width as i32)
            .ok_or_else(|| DisplayError::Rejected("desktop width overflow".into()))?;
    }
    if profile.is_empty() {
        return Err(DisplayError::NothingToTurnOn);
    }
    Ok(profile)
}

/// Report the same one-to-one resolution used by apply, including connector-only identities.
pub fn missing_monitors(profile: &[ProfileMonitor], monitors: &[DisplayMonitor]) -> Vec<String> {
    logic::resolve(profile, monitors)
        .iter()
        .zip(profile)
        .filter(|(resolved, _)| resolved.is_none())
        .map(|(_, saved)| saved.name.clone())
        .collect()
}
pub fn matches(profile: &[ProfileMonitor], monitors: &[DisplayMonitor]) -> bool {
    matches_on(detected_backend(), profile, monitors)
}
fn matches_on(
    backend: Option<Backend>,
    profile: &[ProfileMonitor],
    monitors: &[DisplayMonitor],
) -> bool {
    let mut profile = profile.to_vec();
    let mut live = monitors.to_vec();
    // wlroots has no primary or mirror concept; overlapping outputs do not mirror content.
    // Older KScreen versions have no replication setter. Ignore unsupported mirror requests.
    match backend {
        Some(Backend::WlrRandr) => {
            for l in profile.iter_mut().filter_map(|m| m.layout.as_mut()) {
                l.primary = false;
                l.mirror_of = None;
            }
            for l in live.iter_mut().filter_map(|m| m.layout.as_mut()) {
                l.primary = false;
                l.mirror_of = None;
            }
            // Choose the same normalization anchor even if the compositor reorders outputs.
            let anchor = live
                .iter()
                .enumerate()
                .filter(|(_, m)| m.layout.is_some())
                .min_by_key(|(_, m)| m.identity.path.to_lowercase())
                .map(|(i, _)| i);
            if let Some(anchor) = anchor {
                let resolved = logic::resolve(&profile, &live);
                if let Some(saved) = resolved.iter().position(|i| *i == Some(anchor))
                    && let Some(layout) = &mut profile[saved].layout
                {
                    layout.primary = true;
                }
                if let Some(layout) = &mut live[anchor].layout {
                    layout.primary = true;
                }
            }
        }
        Some(Backend::Kscreen) if !cli::kscreen_mirroring() => {
            for l in profile.iter_mut().filter_map(|m| m.layout.as_mut()) {
                l.mirror_of = None;
            }
            for l in live.iter_mut().filter_map(|m| m.layout.as_mut()) {
                l.mirror_of = None;
            }
        }
        _ => {}
    }
    logic::matches(&profile, &live)
}

/// Persist a kept Mutter layout. Other backends already apply without a compositor countdown.
pub fn keep() -> Result<(), DisplayError> {
    if backend()? == Backend::Mutter {
        mutter::keep()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wlroots_matching_ignores_primary_and_inventory_order_but_not_position() {
        let mode = macafon_ipc::DisplayMode {
            width: 1920,
            height: 1080,
            refresh_mhz: 60000,
        };
        let monitor = |path: &str, x| DisplayMonitor {
            identity: macafon_ipc::DisplayIdentity {
                path: path.into(),
                edid: path.into(),
                serial: None,
            },
            name: path.into(),
            built_in: false,
            hdr_supported: false,
            modes: vec![mode],
            layout: Some(macafon_ipc::DisplayLayout {
                x,
                y: 0,
                mode,
                rotation: macafon_ipc::DisplayRotation::None,
                primary: false,
                mirror_of: None,
                hdr: false,
            }),
        };
        let mut live = vec![monitor("DP-1", 0), monitor("DP-2", 1920)];
        let mut saved = as_profile(&live);
        saved.reverse();
        saved[0].layout.as_mut().expect("layout").primary = true;
        assert!(matches_on(Some(Backend::WlrRandr), &saved, &live));
        live[0].layout.as_mut().expect("layout").x = 10;
        assert!(!matches_on(Some(Backend::WlrRandr), &saved, &live));
        let missing = [ProfileMonitor {
            identity: macafon_ipc::DisplayIdentity {
                path: "missing".into(),
                edid: String::new(),
                serial: None,
            },
            name: "Unplugged".into(),
            layout: None,
        }];
        live[0].identity.edid.clear();
        assert_eq!(missing_monitors(&missing, &live), ["Unplugged"]);
    }
}
