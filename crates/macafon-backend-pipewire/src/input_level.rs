//! Downward capture-level calibration on Linux (CAL-005). WirePlumber's mixer API (`wpctl`)
//! owns source volumes: it adjusts the device route (hardware volume where available) and
//! persists it like the desktop's own slider. The original level is journaled before any
//! change; calibration survives Quit, and explicit recovery restores the user's level.

use crate::{Connection, Error};
use pipewire::types::ObjectType;
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{
    cell::Cell,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    time::{Duration, Instant},
};

const JOURNAL: &str = "macafon/capture-levels.json";
/// `wpctl` prints volumes with two decimals.
const LEVEL_TOLERANCE: f32 = 0.006;
/// WirePlumber's volume scale is cubic: linear amplitude = volume³.
const CUBIC_DB: f32 = 60.0;
/// The lowest level calibration may set (−60 dB amplitude).
const MINIMUM_DB: f32 = -60.0;
/// The software grid for targets; the readback reports what the device actually took.
const INCREMENT_DB: f32 = 0.5;
/// How long a volume change may take to show up in WirePlumber's readback.
const SETTLE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Entry {
    device: String,
    original: f32,
    ours: f32,
    pending: Option<f32>,
}

/// Stable user-local journal shared by the daemon and the recovery CLI.
pub fn capture_level_journal() -> Result<PathBuf, Error> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| Error::Message("HOME or XDG_STATE_HOME is required".into()))?;
    Ok(base.join(JOURNAL))
}

fn read(path: &Path) -> Result<Vec<Entry>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| Error::Message(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(io(path, error)),
    }
}

fn write(path: &Path, entries: &[Entry]) -> Result<(), Error> {
    let bytes =
        serde_json::to_vec_pretty(entries).map_err(|error| Error::Message(error.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|error| io(parent, error))?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| io(&temporary, error))?;
    file.write_all(&bytes)
        .map_err(|error| io(&temporary, error))?;
    file.sync_all().map_err(|error| io(&temporary, error))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|error| io(path, error))
}

fn io(path: &Path, error: std::io::Error) -> Error {
    Error::Message(format!("{}: {error}", path.display()))
}

/// The registry id of a node; `wpctl` addresses nodes by id only.
fn node_id(node_name: &str) -> Result<u32, Error> {
    let connection = Connection::new()?;
    let registry = connection.core.get_registry()?;
    let found = Rc::new(Cell::new(None));
    let collected = found.clone();
    let wanted = node_name.to_owned();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Node
                && global
                    .props
                    .is_some_and(|props| props.get("node.name") == Some(wanted.as_str()))
            {
                collected.set(Some(global.id));
            }
        })
        .register();
    connection.roundtrip()?;
    found
        .get()
        .ok_or_else(|| Error::Message(format!("microphone {node_name} is not available")))
}

fn wpctl(args: &[&str]) -> Result<String, Error> {
    let output = Command::new("wpctl")
        .args(args)
        .output()
        .map_err(|error| Error::Message(format!("cannot run wpctl: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Message(format!(
            "wpctl {}: {}",
            args[0],
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `Volume: 0.74` or `Volume: 0.74 [MUTED]`.
fn parse_volume(output: &str) -> Option<f32> {
    output
        .trim()
        .strip_prefix("Volume:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
        .filter(|volume: &f32| volume.is_finite() && *volume >= 0.0)
}

fn get_volume(id: u32) -> Result<f32, Error> {
    let output = wpctl(&["get-volume", &id.to_string()])?;
    parse_volume(&output)
        .ok_or_else(|| Error::Message(format!("unexpected wpctl output: {}", output.trim())))
}

fn set_volume(id: u32, volume: f32) -> Result<(), Error> {
    wpctl(&["set-volume", &id.to_string(), &format!("{volume:.4}")]).map(drop)
}

/// Targets live on `wpctl`'s two-decimal grid and move at least one step; otherwise a small
/// correction reads back as the old level and looks like a device that refused the change.
fn wpctl_step_down(current: f32, target: f32) -> f32 {
    let current = (current * 100.0).round() / 100.0;
    ((target * 100.0).floor() / 100.0).min(current - 0.01)
}

/// WirePlumber applies a new volume asynchronously; an immediate readback can still report
/// the old level, so wait briefly for it to move.
fn settled_volume(id: u32, previous: f32) -> Result<f32, Error> {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    loop {
        let volume = get_volume(id)?;
        if (volume - previous).abs() > LEVEL_TOLERANCE / 2.0 || Instant::now() >= deadline {
            return Ok(volume);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn volume_db(volume: f32) -> f32 {
    CUBIC_DB * volume.max(1e-6).log10()
}

fn db_volume(db: f32) -> f32 {
    10.0f32.powf(db / CUBIC_DB)
}

/// Lower only the selected microphone by one step. Returns the dB change for rescaling the
/// already captured, unclipped calibration samples; fresh capture must be checked after the
/// device settles.
pub fn lower(node_name: &str, raw_peak_dbfs: f32) -> Result<f32, Error> {
    lower_with(&capture_level_journal()?, node_name, raw_peak_dbfs)
}

fn lower_with(journal: &Path, node_name: &str, raw_peak_dbfs: f32) -> Result<f32, Error> {
    let id = node_id(node_name)?;
    let current = get_volume(id)?;
    let current_db = volume_db(current);
    let target_db = macafon_dsp::calibration::input_level_target_db(
        current_db,
        MINIMUM_DB,
        INCREMENT_DB,
        raw_peak_dbfs,
    )
    .filter(|target| *target < current_db)
    .ok_or_else(|| Error::Message("microphone input level cannot be lowered further".into()))?;
    let target = wpctl_step_down(current, db_volume(target_db));
    if target < db_volume(MINIMUM_DB) {
        return Err(Error::Message(
            "microphone input level cannot be lowered further".into(),
        ));
    }
    let mut entries = read(journal)?;
    let index = match entries.iter().position(|entry| entry.device == node_name) {
        Some(index) => index,
        None => {
            entries.push(Entry {
                device: node_name.to_owned(),
                original: current,
                ours: current,
                pending: None,
            });
            entries.len() - 1
        }
    };
    // Write-ahead: either side of a failed call/crash remains recoverable. Repeated
    // calibrations never replace the original user level with an already calibrated level.
    entries[index].ours = current;
    entries[index].pending = Some(target);
    write(journal, &entries)?;
    set_volume(id, target)?;
    let actual = settled_volume(id, current)?;
    entries[index].ours = actual;
    entries[index].pending = None;
    write(journal, &entries)?;
    if actual >= current - LEVEL_TOLERANCE / 2.0 {
        return Err(Error::Message(
            "microphone input level did not decrease".into(),
        ));
    }
    Ok(volume_db(actual) - current_db)
}

/// Explicit recovery only (`macafon restore-default`): preserve later user choices, retain
/// unplugged/failed entries, and try every device even if one fails. Quit must not call this.
pub fn restore_capture_levels(journal: &Path) -> Result<(), Error> {
    let entries = read(journal)?;
    if entries.is_empty() {
        return Ok(());
    }
    let mut retained = Vec::new();
    let mut failed = None;
    for entry in entries {
        let result = (|| -> Result<(), Error> {
            let id = node_id(&entry.device)?;
            let current = get_volume(id)?;
            if should_restore(&entry, current) {
                set_volume(id, entry.original)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            failed = Some(error);
            retained.push(entry);
        }
    }
    if retained.is_empty() {
        std::fs::remove_file(journal).map_err(|error| io(journal, error))?;
    } else {
        write(journal, &retained)?;
    }
    failed.map_or(Ok(()), Err)
}

/// A level the user changed after calibration is theirs and stays.
fn should_restore(entry: &Entry, current: f32) -> bool {
    let matches = |level: f32| (current - level).abs() <= LEVEL_TOLERANCE;
    matches(entry.ours) || entry.pending.is_some_and(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_volume_output() {
        assert_eq!(parse_volume("Volume: 1.00\n"), Some(1.0));
        assert_eq!(parse_volume("Volume: 0.74 [MUTED]\n"), Some(0.74));
        assert_eq!(parse_volume("Error: 'x' is not a valid number"), None);
    }

    #[test]
    fn cubic_volume_maps_to_amplitude_db() {
        assert!((volume_db(1.0)).abs() < 1e-4);
        assert!((volume_db(db_volume(-6.0)) + 6.0).abs() < 1e-3);
        // wpctl 0.5 is 0.125 linear amplitude, about −18 dB.
        assert!((volume_db(0.5) + 18.06).abs() < 0.01);
    }

    #[test]
    fn a_small_correction_still_moves_one_visible_step() {
        // 0.66 → −0.1 dB would read back as 0.66 again.
        assert!((wpctl_step_down(0.66, 0.6554) - 0.65).abs() < 1e-6);
        assert!((wpctl_step_down(1.0, 0.794) - 0.79).abs() < 1e-6);
    }

    #[test]
    fn restore_keeps_a_level_the_user_changed_later() {
        let entry = Entry {
            device: "mic".into(),
            original: 1.0,
            ours: 0.79,
            pending: None,
        };
        assert!(should_restore(&entry, 0.79));
        assert!(!should_restore(&entry, 0.5));
        let pending = Entry {
            pending: Some(0.63),
            ..entry
        };
        assert!(should_restore(&pending, 0.63));
    }
}
