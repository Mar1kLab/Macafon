//! Journal of system defaults Macafon changed (MIC-012, WIN-MIC-005, WIN-CH-007).
//!
//! Before the first change the original default of every role is written to disk; restoring
//! (Quit, start-up after a crash, uninstall) puts it back only where the default is still the
//! one Macafon chose, so a later choice by the user wins. As on Linux the journal is never
//! overwritten while it exists: it always holds the user's original setup.

use crate::Error;
use crate::devices::{DataFlow, EndpointRole};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Current default per role and the way to change it; the system one is [`SystemDefaults`].
pub trait Defaults {
    fn get(&self, flow: DataFlow, role: EndpointRole) -> Result<Option<String>, Error>;
    fn set(&mut self, device_id: &str, role: EndpointRole) -> Result<(), Error>;
}

/// The real defaults: `IMMDeviceEnumerator` and `IPolicyConfig`.
pub struct SystemDefaults;

impl Defaults for SystemDefaults {
    fn get(&self, flow: DataFlow, role: EndpointRole) -> Result<Option<String>, Error> {
        crate::devices::default_device(flow, role)
    }

    fn set(&mut self, device_id: &str, role: EndpointRole) -> Result<(), Error> {
        crate::policy::set_default(device_id, role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    flow: Flow,
    role: Role,
    /// The user's default before Macafon changed it.
    previous: Option<String>,
    /// What Macafon set; restoring only happens while this is still the default.
    ours: String,
    /// A write-ahead target; either choice may still be current if the call fails or crashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
}

// Stable on-disk names, independent of the Rust enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Flow {
    Capture,
    Render,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Console,
    Multimedia,
    Communications,
}

impl From<DataFlow> for Flow {
    fn from(flow: DataFlow) -> Self {
        match flow {
            DataFlow::Capture => Self::Capture,
            DataFlow::Render => Self::Render,
        }
    }
}

impl From<Flow> for DataFlow {
    fn from(flow: Flow) -> Self {
        match flow {
            Flow::Capture => Self::Capture,
            Flow::Render => Self::Render,
        }
    }
}

impl From<EndpointRole> for Role {
    fn from(role: EndpointRole) -> Self {
        match role {
            EndpointRole::Console => Self::Console,
            EndpointRole::Multimedia => Self::Multimedia,
            EndpointRole::Communications => Self::Communications,
        }
    }
}

impl From<Role> for EndpointRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Console => Self::Console,
            Role::Multimedia => Self::Multimedia,
            Role::Communications => Self::Communications,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    entries: Vec<Entry>,
}

/// `%LOCALAPPDATA%\Macafon\default-capture.json` and `…\default-render.json`.
pub fn journal_path(state_dir: &Path, flow: DataFlow) -> PathBuf {
    state_dir.join(match flow {
        DataFlow::Capture => "default-capture.json",
        DataFlow::Render => "default-render.json",
    })
}

/// Make `device_id` the default of `flow` for every role, journaling the originals first.
pub fn set_default(
    defaults: &mut impl Defaults,
    path: &Path,
    flow: DataFlow,
    device_id: &str,
) -> Result<(), Error> {
    let mut document = read(path)?.unwrap_or_default();
    for role in EndpointRole::ALL {
        let key = (Flow::from(flow), Role::from(role));
        match document
            .entries
            .iter_mut()
            .find(|entry| (entry.flow, entry.role) == key)
        {
            // Already ours from an earlier change: keep the original `previous`.
            Some(entry) => {
                if entry.pending.as_deref() == defaults.get(flow, role)?.as_deref() {
                    if let Some(pending) = entry.pending.take() {
                        entry.ours = pending;
                    }
                }
                entry.pending = Some(device_id.to_owned());
            }
            None => document.entries.push(Entry {
                flow: key.0,
                role: key.1,
                previous: defaults.get(flow, role)?,
                ours: device_id.to_owned(),
                pending: None,
            }),
        }
    }
    // Durable before the system changes, so a crash right after still restores.
    write(path, &document)?;
    for role in EndpointRole::ALL {
        defaults.set(device_id, role)?;
    }
    for entry in &mut document.entries {
        if let Some(pending) = entry.pending.take() {
            entry.ours = pending;
        }
    }
    write(path, &document)?;
    Ok(())
}

/// The original default Macafon replaced for `flow`/`role`, if a change is journaled.
pub fn previous(path: &Path, flow: DataFlow, role: EndpointRole) -> Option<String> {
    let key = (Flow::from(flow), Role::from(role));
    read(path)
        .ok()
        .flatten()?
        .entries
        .into_iter()
        .find(|entry| (entry.flow, entry.role) == key)?
        .previous
}

/// Restore every journaled role that still points at Macafon's choice, then delete the
/// journal. Roles the user changed in the meantime are left alone. A missing journal is a
/// no-op, so this is safe at every start-up.
pub fn restore(defaults: &mut impl Defaults, path: &Path) -> Result<(), Error> {
    let Some(document) = read(path)? else {
        return Ok(());
    };
    let mut failed = None;
    for entry in &document.entries {
        let current = defaults.get(entry.flow.into(), entry.role.into())?;
        let Some(previous) = &entry.previous else {
            continue;
        };
        if current.as_deref() == Some(entry.ours.as_str())
            || entry
                .pending
                .as_deref()
                .is_some_and(|id| current.as_deref() == Some(id))
        {
            // The previous device may be unplugged now; keep trying the other roles and keep
            // the journal so the next start can finish the job.
            if let Err(error) = defaults.set(previous, entry.role.into()) {
                failed = Some(error);
            }
        }
    }
    if let Some(error) = failed {
        return Err(error);
    }
    std::fs::remove_file(path).map_err(|error| io(path, error))
}

fn read(path: &Path) -> Result<Option<Document>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| Error::Journal(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io(path, error)),
    }
}

fn write(path: &Path, document: &Document) -> Result<(), Error> {
    let bytes =
        serde_json::to_vec_pretty(document).map_err(|error| Error::Journal(error.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&temporary).map_err(|error| io(&temporary, error))?;
    file.write_all(&bytes).map_err(|error| io(&temporary, error))?;
    file.sync_all().map_err(|error| io(&temporary, error))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|error| io(path, error))
}

fn io(path: &Path, error: std::io::Error) -> Error {
    Error::Journal(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        current: HashMap<(Flow, Role), String>,
        unplugged: Vec<String>,
    }

    impl Defaults for Fake {
        fn get(&self, flow: DataFlow, role: EndpointRole) -> Result<Option<String>, Error> {
            Ok(self.current.get(&(flow.into(), role.into())).cloned())
        }
        fn set(&mut self, device_id: &str, role: EndpointRole) -> Result<(), Error> {
            if self.unplugged.iter().any(|id| id == device_id) {
                return Err(Error::DeviceNotFound(device_id.into()));
            }
            // Capture IDs start with {0.0.1, render with {0.0.0.
            let flow = if device_id.starts_with("mic") || device_id.starts_with("cable") {
                Flow::Capture
            } else {
                Flow::Render
            };
            self.current.insert((flow, role.into()), device_id.into());
            Ok(())
        }
    }

    fn fake(mic: &str, comms: &str) -> Fake {
        let mut fake = Fake::default();
        for role in [Role::Console, Role::Multimedia] {
            fake.current.insert((Flow::Capture, role), mic.into());
        }
        fake.current
            .insert((Flow::Capture, Role::Communications), comms.into());
        fake
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("macafon-journal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        journal_path(&dir, DataFlow::Capture)
    }

    #[test]
    fn restores_originals_after_a_crash_but_respects_later_user_choices() {
        let path = temp("crash");
        let mut system = fake("mic-usb", "mic-headset");
        set_default(&mut system, &path, DataFlow::Capture, "cable-output").expect("set");
        // A second change (e.g. after a daemon restart) must not journal the cable as
        // the "previous" default.
        set_default(&mut system, &path, DataFlow::Capture, "cable-output").expect("again");
        assert_eq!(
            previous(&path, DataFlow::Capture, EndpointRole::Communications).as_deref(),
            Some("mic-headset")
        );
        // The daemon dies; meanwhile the user picks another microphone for one role.
        system
            .set("mic-webcam", EndpointRole::Multimedia)
            .expect("user");
        restore(&mut system, &path).expect("restore");
        let get = |role| system.get(DataFlow::Capture, role).expect("get");
        assert_eq!(get(EndpointRole::Console).as_deref(), Some("mic-usb"));
        assert_eq!(get(EndpointRole::Multimedia).as_deref(), Some("mic-webcam"));
        assert_eq!(
            get(EndpointRole::Communications).as_deref(),
            Some("mic-headset")
        );
        assert!(!path.exists());
        restore(&mut system, &path).expect("restore without a journal is a no-op");
    }

    #[test]
    fn keeps_the_journal_while_the_original_device_is_unplugged() {
        let path = temp("unplugged");
        let mut system = fake("mic-usb", "mic-usb");
        set_default(&mut system, &path, DataFlow::Capture, "cable-output").expect("set");
        system.unplugged.push("mic-usb".into());
        assert!(restore(&mut system, &path).is_err());
        assert!(path.exists(), "the next start retries");
        system.unplugged.clear();
        restore(&mut system, &path).expect("restore");
        assert_eq!(
            system
                .get(DataFlow::Capture, EndpointRole::Console)
                .expect("get")
                .as_deref(),
            Some("mic-usb")
        );
    }

    #[test]
    fn failed_retarget_keeps_the_previous_macafon_choice_restorable() {
        let path = temp("retarget");
        let mut system = fake("mic-usb", "mic-headset");
        set_default(&mut system, &path, DataFlow::Capture, "cable-output").expect("first choice");
        system.unplugged.push("cable-new".into());
        assert!(set_default(&mut system, &path, DataFlow::Capture, "cable-new").is_err());
        // Simulate a partial switch before the failure/crash: both old and new choices
        // must be recovered, while an unrelated manual choice must be preserved.
        system
            .current
            .insert((Flow::Capture, Role::Console), "cable-new".into());
        system
            .current
            .insert((Flow::Capture, Role::Multimedia), "mic-webcam".into());
        restore(&mut system, &path).expect("restore after failed retarget");
        assert_eq!(system.current[&(Flow::Capture, Role::Console)], "mic-usb");
        assert_eq!(
            system.current[&(Flow::Capture, Role::Multimedia)],
            "mic-webcam"
        );
        assert_eq!(
            system.current[&(Flow::Capture, Role::Communications)],
            "mic-headset"
        );
        assert!(!path.exists());
    }
}
