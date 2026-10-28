//! Stream-friendly gain staging for volume groups. Discord, OBS and other application
//! capture take an app's audio after its session volume but before the device volume, so the
//! common attenuation goes to the device: the loudest playing app's session is lifted to full
//! scale and the default output is turned down by the same amount. What the user hears stays
//! the same; the lift moves in small steps so a change of the loudest app is never a jump.
//! No COM here: the mixer worker owns the device and the sessions.
use crate::Error;
use macafon_dsp::channel::volume_gain;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Largest lift change per step. The device and the sessions are written one after the
/// other, so for a moment they disagree by at most this much (always quieter, never louder).
pub(crate) const STEP_DB: f32 = 1.0;
/// Time between steps while the lift moves: up to 50 dB per second.
pub(crate) const STEP_EVERY: Duration = Duration::from_millis(20);
/// A louder app that went quiet keeps its headroom this long before the lift grows again,
/// so short sounds and restarts do not move the device volume back and forth.
pub(crate) const RAISE_HOLD: Duration = Duration::from_secs(3);
/// Changes below this are not written: device volume steps and float noise.
const EPSILON_DB: f32 = 0.05;
/// A device volume this far from what Macafon last wrote was changed by someone else.
pub(crate) const FOREIGN_DB: f32 = 0.1;

/// The Master fader as device gain in dB: the shared cubic curve (CH-004), `-inf` at 0%.
pub(crate) fn master_db(master_pct: f32) -> f32 {
    20.0 * volume_gain(master_pct).log10()
}

/// The Master fader for a device gain in dB.
pub(crate) fn master_pct(db: f32) -> f32 {
    if !db.is_finite() {
        return if db > 0.0 { 100.0 } else { 0.0 };
    }
    (10f32.powf(db / 20.0).cbrt() * 100.0).clamp(0.0, 100.0)
}

pub(crate) fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The lift that brings the loudest audible app (`reference`, its linear session gain before
/// the lift) to full scale, limited by how far the device can go down. `None` holds the lift.
pub(crate) fn target_lift(reference: Option<f32>, room_db: f32) -> Option<f32> {
    let reference = reference.filter(|gain| *gain > 0.0 && gain.is_finite())?;
    Some((-20.0 * reference.min(1.0).log10()).clamp(0.0, room_db.max(0.0)))
}

/// Where the lift is and where it goes, one step at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ramp {
    pub lift_db: f32,
    /// A larger target is held until then.
    raise_after: Option<Instant>,
    /// No step before then: the loop also wakes for commands and callbacks.
    step_after: Option<Instant>,
}

impl Ramp {
    pub fn new(lift_db: f32) -> Self {
        Self {
            lift_db: lift_db.max(0.0),
            raise_after: None,
            step_after: None,
        }
    }

    /// The next lift toward `target`, or `None` when it stays. A smaller lift (a louder app
    /// started) is taken at once; a larger one waits for [`RAISE_HOLD`]. Steps are at least
    /// [`STEP_EVERY`] apart.
    pub fn next(&mut self, target: Option<f32>, now: Instant) -> Option<f32> {
        let Some(target) = target else {
            // Nothing plays: a hold starts again with the next app.
            self.raise_after = None;
            return None;
        };
        let delta = target - self.lift_db;
        if delta.abs() < EPSILON_DB {
            self.raise_after = None;
            return None;
        }
        if delta > 0.0 {
            let after = *self.raise_after.get_or_insert(now + RAISE_HOLD);
            if now < after {
                return None;
            }
        } else {
            self.raise_after = None;
        }
        if self.step_after.is_some_and(|after| now < after) {
            return None;
        }
        self.step_after = Some(now + STEP_EVERY);
        Some(self.lift_db + delta.clamp(-STEP_DB, STEP_DB))
    }

    /// When the loop should look again: a step is due, a hold ends, or nothing to do.
    pub fn wake_in(&self, target: Option<f32>, now: Instant) -> Option<Duration> {
        let target = target?;
        let delta = target - self.lift_db;
        if delta.abs() < EPSILON_DB {
            return None;
        }
        let hold = self.raise_after.filter(|after| delta > 0.0 && now < *after);
        let at = hold.into_iter().chain(self.step_after).max();
        Some(at.map_or(Duration::ZERO, |at| at.saturating_duration_since(now)))
    }
}

/// What a restart needs to know about a device volume Macafon left lowered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Saved {
    pub endpoint: String,
    /// The Master fader in device dB; the device minus the lift.
    pub master_db: f32,
    /// What the device was set to.
    pub written_db: f32,
    /// Left on purpose at logoff/shutdown so the next logon starts as quiet as it ended;
    /// otherwise the daemon stopped without giving the volume back.
    #[serde(default)]
    pub kept: bool,
}

/// Whether the last daemon kept the volumes at logoff/shutdown.
pub(crate) fn kept(state_dir: &Path) -> bool {
    Store::new(state_dir)
        .load()
        .is_ok_and(|saved| saved.iter().any(|saved| saved.kept))
}

/// Device volume records next to the session journal, one per lowered device: a record is
/// dropped only once its device has its volume back.
pub(crate) struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join("system-volume.json"),
        }
    }

    pub fn load(&self) -> Result<Vec<Saved>, Error> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(error),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(error(e)),
        }
    }

    /// Replace the record of `saved.endpoint`. Written before the device: a crash leaves the
    /// record. Rewritten every step, so no `fsync`: a process crash keeps it, a power loss
    /// costs at most the volume of one ramp.
    pub fn save(&self, saved: &Saved) -> Result<(), Error> {
        let mut all = self.load().unwrap_or_default();
        all.retain(|known| known.endpoint != saved.endpoint);
        all.push(saved.clone());
        self.write(&all)
    }

    /// The device has its volume back.
    pub fn clear(&self, endpoint: &str) -> Result<(), Error> {
        let mut all = self.load()?;
        let before = all.len();
        all.retain(|known| known.endpoint != endpoint);
        if all.len() == before {
            return Ok(());
        }
        self.write(&all)
    }

    fn write(&self, all: &[Saved]) -> Result<(), Error> {
        if all.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(error(e)),
                _ => Ok(()),
            };
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(all).map_err(error)?).map_err(error)?;
        std::fs::rename(temporary, &self.path).map_err(error)
    }
}

fn error(error: impl std::fmt::Display) -> Error {
    Error::Journal(format!("system volume: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_curve_round_trips_through_device_db() {
        assert_eq!(master_db(100.0), 0.0);
        assert!((master_db(50.0) + 18.06).abs() < 0.01);
        for pct in [1.0, 25.0, 50.0, 73.0, 100.0] {
            assert!((master_pct(master_db(pct)) - pct).abs() < 0.01, "{pct}");
        }
        assert_eq!(master_pct(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn the_loudest_app_is_lifted_to_full_scale_within_the_device_range() {
        // Voice 30% (Discord) is the loudest: 0.027 → 31.4 dB of lift.
        let lift = target_lift(Some(volume_gain(30.0)), 60.0).expect("lift");
        assert!((lift - 31.37).abs() < 0.01);
        assert_eq!(target_lift(Some(volume_gain(30.0)), 20.0), Some(20.0));
        assert_eq!(target_lift(Some(1.0), 60.0), Some(0.0));
        assert_eq!(target_lift(None, 60.0), None);
        assert_eq!(target_lift(Some(0.0), 60.0), None);
    }

    #[test]
    fn a_louder_app_lowers_the_lift_at_once_in_steps() {
        let now = Instant::now();
        let mut ramp = Ramp::new(10.0);
        let mut steps = 0;
        let mut at = now;
        while let Some(next) = ramp.next(Some(2.5), at) {
            assert!(ramp.lift_db - next <= STEP_DB + 1e-6);
            ramp.lift_db = next;
            steps += 1;
            // Woken early by a command: no extra step.
            assert_eq!(ramp.next(Some(2.5), at + STEP_EVERY / 2), None);
            at += STEP_EVERY;
        }
        assert_eq!(steps, 8);
        assert!((ramp.lift_db - 2.5).abs() < 1e-6);
    }

    #[test]
    fn a_larger_lift_waits_for_the_hold_then_steps() {
        let now = Instant::now();
        let mut ramp = Ramp::new(0.0);
        assert_eq!(ramp.next(Some(5.0), now), None);
        assert_eq!(ramp.wake_in(Some(5.0), now), Some(RAISE_HOLD));
        // A louder app came back during the hold: the hold restarts later.
        assert_eq!(ramp.next(Some(0.0), now), None);
        assert_eq!(ramp.next(Some(5.0), now + RAISE_HOLD), None);
        let later = now + RAISE_HOLD * 2;
        assert_eq!(ramp.next(Some(5.0), later), Some(1.0));
        assert_eq!(ramp.wake_in(Some(5.0), later), Some(STEP_EVERY));
        assert_eq!(ramp.next(Some(5.0), later + STEP_EVERY / 2), None);
        assert_eq!(ramp.next(None, later), None);
    }

    #[test]
    fn a_lowered_device_keeps_its_record_until_it_has_its_volume_back() {
        let dir = std::env::temp_dir().join(format!("macafon-lift-{}", std::process::id()));
        let store = Store::new(&dir);
        let saved = |endpoint: &str, kept| Saved {
            endpoint: endpoint.into(),
            master_db: -10.0,
            written_db: -40.0,
            kept,
        };
        store.save(&saved("old", false)).expect("old");
        // Switching outputs records the new device and keeps the old one's record.
        store.save(&saved("new", false)).expect("new");
        store.save(&saved("new", true)).expect("kept");
        assert_eq!(store.load().expect("load").len(), 2);
        assert!(kept(&dir));
        store.clear("new").expect("clear");
        assert_eq!(store.load().expect("load"), vec![saved("old", false)]);
        assert!(!kept(&dir));
        store.clear("old").expect("clear");
        assert!(!dir.join("system-volume.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
