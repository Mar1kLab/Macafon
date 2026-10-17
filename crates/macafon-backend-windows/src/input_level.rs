//! Downward capture-level calibration. Persist the original before touching the endpoint;
//! calibration survives Quit, and explicit recovery/uninstall restores the user's level.

use crate::{Error, devices::device_by_id};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};
use windows::Win32::{Media::Audio::Endpoints::IAudioEndpointVolume, System::Com::CLSCTX_ALL};

const JOURNAL: &str = "capture-levels.json";
const LEVEL_TOLERANCE_DB: f32 = 0.01;

#[derive(Serialize, Deserialize)]
struct Entry {
    device: String,
    original_db: f32,
    ours_db: f32,
    pending_db: Option<f32>,
}

fn endpoint(device_id: &str) -> Result<IAudioEndpointVolume, Error> {
    let device = device_by_id(device_id)?;
    // SAFETY: the control/recovery thread initialises COM; the device is a live endpoint.
    Ok(unsafe { device.Activate(CLSCTX_ALL, None) }?)
}

fn read(path: &Path) -> Result<Vec<Entry>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| Error::Journal(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(io(path, error)),
    }
}

fn write(path: &Path, entries: &[Entry]) -> Result<(), Error> {
    let bytes =
        serde_json::to_vec_pretty(entries).map_err(|error| Error::Journal(error.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&temporary).map_err(|error| io(&temporary, error))?;
    file.write_all(&bytes)
        .map_err(|error| io(&temporary, error))?;
    file.sync_all().map_err(|error| io(&temporary, error))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|error| io(path, error))
}

fn io(path: &Path, error: std::io::Error) -> Error {
    Error::Journal(format!("{}: {error}", path.display()))
}

/// Lower only the selected capture endpoint by one hardware-aligned step. Returns the dB
/// change for rescaling the already captured, unclipped calibration samples. Fresh capture
/// must be checked after the endpoint settles; a dB readback can be the requested value
/// rather than the hardware's actual step (IAudioEndpointVolume::GetVolumeRange).
pub fn lower(state_dir: &Path, device_id: &str, raw_peak_dbfs: f32) -> Result<f32, Error> {
    let volume = endpoint(device_id)?;
    // SAFETY: valid COM interface on the control thread; all out pointers are local floats.
    let (current_db, target_db) = unsafe {
        let current_db = volume.GetMasterVolumeLevel()?;
        let (mut minimum_db, mut maximum_db, mut increment_db) = (0.0, 0.0, 0.0);
        volume.GetVolumeRange(&mut minimum_db, &mut maximum_db, &mut increment_db)?;
        let target_db = macafon_dsp::calibration::input_level_target_db(
            current_db,
            minimum_db,
            increment_db,
            raw_peak_dbfs,
        )
        .filter(|target| *target < current_db)
        .ok_or_else(|| {
            Error::InputLevel("microphone input level cannot be lowered further".into())
        })?;
        (current_db, target_db)
    };
    let path = state_dir.join(JOURNAL);
    let mut entries = read(&path)?;
    let index = match entries.iter().position(|entry| entry.device == device_id) {
        Some(index) => index,
        None => {
            entries.push(Entry {
                device: device_id.to_owned(),
                original_db: current_db,
                ours_db: current_db,
                pending_db: None,
            });
            entries.len() - 1
        }
    };
    // Write-ahead: either side of a failed call/crash remains recoverable. Repeated
    // calibrations never replace the original user level with an already calibrated level.
    entries[index].ours_db = current_db;
    entries[index].pending_db = Some(target_db);
    write(&path, &entries)?;
    // SAFETY: a live endpoint interface, and null means no event-context GUID.
    let actual_db = unsafe {
        volume.SetMasterVolumeLevel(target_db, std::ptr::null())?;
        volume.GetMasterVolumeLevel()?
    };
    entries[index].ours_db = actual_db;
    entries[index].pending_db = None;
    write(&path, &entries)?;
    if actual_db >= current_db - LEVEL_TOLERANCE_DB {
        return Err(Error::InputLevel(
            "microphone input level did not decrease".into(),
        ));
    }
    Ok(actual_db - current_db)
}

/// Explicit recovery only: preserve later user choices, retain unplugged/failed entries,
/// and try every device even if one fails. Ordinary daemon Quit must not call this.
pub fn restore(state_dir: &Path) -> Result<(), Error> {
    let path = state_dir.join(JOURNAL);
    let entries = read(&path)?;
    if entries.is_empty() {
        return Ok(());
    }
    let mut retained = Vec::new();
    let mut failed = None;
    for entry in entries {
        let result = (|| -> Result<(), Error> {
            let volume = endpoint(&entry.device)?;
            // SAFETY: a live endpoint on the COM-initialised recovery thread.
            unsafe {
                let current_db = volume.GetMasterVolumeLevel()?;
                let matches = |level: f32| (current_db - level).abs() <= LEVEL_TOLERANCE_DB;
                if matches(entry.ours_db) || entry.pending_db.is_some_and(matches) {
                    volume.SetMasterVolumeLevel(entry.original_db, std::ptr::null())?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            failed = Some(error);
            retained.push(entry);
        }
    }
    if retained.is_empty() {
        std::fs::remove_file(&path).map_err(|error| io(&path, error))?;
    } else {
        write(&path, &retained)?;
    }
    failed.map_or(Ok(()), Err)
}
