//! Durable, conditional recovery of session settings. No COM dependency in the journal.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Key {
    pub exe: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Value {
    pub volume: f32,
    pub muted: bool,
}

#[derive(Clone)]
pub(crate) struct StoredSession {
    pub key: Key,
    pub instance: String,
    pub value: Value,
}

pub(crate) trait SessionStore {
    fn list(&self) -> Result<Vec<StoredSession>, Error>;
    fn set(&mut self, instance: &str, value: Value) -> Result<(), Error>;
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    key: Key,
    // A hint for live sessions, never the recovery key after a process restart.
    instance: String,
    previous: Value,
    ours: Value,
    /// A write-ahead value: the COM call may have failed or crashed, so either `ours` or this
    /// can be current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Value>,
}

impl Entry {
    fn is_ours_volume(&self, volume: f32) -> bool {
        close(volume, self.ours.volume) || self.pending.is_some_and(|p| close(volume, p.volume))
    }
    fn is_ours_mute(&self, muted: bool) -> bool {
        muted == self.ours.muted || self.pending.is_some_and(|p| muted == p.muted)
    }
}

pub(crate) struct Journal {
    path: PathBuf,
    entries: Vec<Entry>,
}

impl Journal {
    pub fn pending(&self) -> usize {
        self.entries.len()
    }

    pub fn open(state_dir: &Path) -> Result<Self, Error> {
        let path = state_dir.join("session-volumes.json");
        let entries = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(error)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(error(e)),
        };
        Ok(Self { path, entries })
    }

    pub fn record(&mut self, session: &StoredSession, ours: Value) -> Result<(), Error> {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.key == session.key && entry.instance == session.instance)
        {
            // `session.value` is current: it confirms the last pending write if that landed;
            // otherwise that write is not on the session and needs no recovery.
            if entry.pending == Some(session.value) {
                entry.ours = session.value;
            }
            entry.pending = Some(ours);
        } else {
            self.entries.push(Entry {
                key: session.key.clone(),
                instance: session.instance.clone(),
                previous: session.value,
                ours,
                pending: None,
            });
        }
        self.save()
    }

    pub fn restore(&mut self, store: &mut impl SessionStore) -> Result<(), Error> {
        self.restore_matching(store, None)
    }

    pub fn restore_app(&mut self, store: &mut impl SessionStore, exe: &str) -> Result<(), Error> {
        self.restore_matching(store, Some(exe))
    }

    fn restore_matching(
        &mut self,
        store: &mut impl SessionStore,
        exe: Option<&str>,
    ) -> Result<(), Error> {
        let sessions = store.list()?;
        let mut completed = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if exe.is_some_and(|exe| entry.key.exe != exe) {
                continue;
            }
            let exact = sessions
                .iter()
                .any(|s| s.key == entry.key && s.instance == entry.instance);
            let candidates: Vec<_> = sessions
                .iter()
                .filter(|s| s.key == entry.key && (!exact || s.instance == entry.instance))
                .collect();
            if candidates.is_empty() {
                continue; // An absent endpoint/app may return on the next start.
            }
            // Several original values for one executable cannot safely be assigned to new
            // session IDs. Keep those records for explicit recovery instead of guessing.
            if !exact
                && self
                    .entries
                    .iter()
                    .any(|other| other.key == entry.key && other.previous != entry.previous)
            {
                continue;
            }
            for session in candidates {
                let mut value = session.value;
                if entry.is_ours_volume(value.volume) {
                    value.volume = entry.previous.volume;
                }
                if entry.is_ours_mute(value.muted) {
                    value.muted = entry.previous.muted;
                }
                if value != session.value {
                    store.set(&session.instance, value)?;
                }
            }
            completed.push(index);
        }
        for index in completed.into_iter().rev() {
            self.entries.remove(index);
        }
        self.save()
    }

    fn save(&self) -> Result<(), Error> {
        if self.entries.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(error(e)),
            };
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary).map_err(error)?;
        serde_json::to_writer_pretty(&mut file, &self.entries).map_err(error)?;
        file.sync_all().map_err(error)?;
        drop(file);
        std::fs::rename(temporary, &self.path).map_err(error)
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.00001
}
fn error(e: impl std::fmt::Display) -> Error {
    Error::Journal(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<StoredSession>);
    impl SessionStore for Fake {
        fn list(&self) -> Result<Vec<StoredSession>, Error> {
            Ok(self.0.clone())
        }
        fn set(&mut self, instance: &str, value: Value) -> Result<(), Error> {
            self.0
                .iter_mut()
                .find(|s| s.instance == instance)
                .expect("session")
                .value = value;
            Ok(())
        }
    }

    #[test]
    fn unmanaged_app_restores_only_its_sessions_and_can_be_managed_again() {
        let dir = std::env::temp_dir().join(format!("macafon-unmanaged-{}", std::process::id()));
        let original = Value {
            volume: 0.8,
            muted: false,
        };
        let ours = Value {
            volume: 0.125,
            muted: true,
        };
        let mut store = Fake(
            ["browser.exe", "player.exe"]
                .into_iter()
                .enumerate()
                .map(|(i, exe)| StoredSession {
                    key: Key {
                        exe: exe.into(),
                        endpoint: "speakers".into(),
                    },
                    instance: i.to_string(),
                    value: original,
                })
                .collect(),
        );
        let mut journal = Journal::open(&dir).expect("journal");
        for session in &mut store.0 {
            journal.record(session, ours).expect("record");
            session.value = ours;
        }
        journal
            .restore_app(&mut store, "browser.exe")
            .expect("unmanage");
        assert_eq!(store.0[0].value, original);
        assert_eq!(store.0[1].value, ours);
        assert_eq!(journal.pending(), 1);

        // Re-managing starts a new journal entry from the restored session value.
        journal.record(&store.0[0], ours).expect("manage again");
        store.0[0].value = ours;
        // A subsequent external volume change must survive; our mute is restored.
        store.0[0].value.volume = 0.6;
        journal
            .restore_app(&mut store, "browser.exe")
            .expect("unmanage again");
        assert_eq!(
            store.0[0].value,
            Value {
                volume: 0.6,
                muted: false
            }
        );
        assert_eq!(store.0[1].value, ours);
        journal.restore(&mut store).expect("cleanup");
        assert_eq!(store.0[1].value, original);
        assert_eq!(journal.pending(), 0);
    }

    #[test]
    fn crash_recovery_uses_exe_endpoint_and_preserves_foreign_changes() {
        let dir =
            std::env::temp_dir().join(format!("macafon-mixer-journal-{}", std::process::id()));
        let key = Key {
            exe: "browser.exe".into(),
            endpoint: "speakers".into(),
        };
        let original = Value {
            volume: 0.8,
            muted: false,
        };
        let ours = Value {
            volume: 0.125,
            muted: true,
        };
        let mut store = Fake(vec![StoredSession {
            key,
            instance: "old-pid".into(),
            value: original,
        }]);
        let mut journal = Journal::open(&dir).expect("open");
        journal
            .record(&store.0[0], ours)
            .expect("record before mutation");
        store.0[0].value = ours;
        journal
            .record(
                &store.0[0],
                Value {
                    volume: 0.25,
                    ..ours
                },
            )
            .expect("second change");
        store.0[0].instance = "new-pid".into();
        store.0[0].value = Value {
            volume: 0.6,
            ..ours
        }; // External volume wins; mute is still ours.
        let mut journal = Journal::open(&dir).expect("restart");
        journal.restore(&mut store).expect("restore");
        assert_eq!(
            store.0[0].value,
            Value {
                volume: 0.6,
                muted: false
            }
        );
        store.0[0].value = original;
        journal.record(&store.0[0], ours).expect("record");
        store.0[0].value = ours;
        let missing = store.0.pop().expect("session");
        journal.restore(&mut store).expect("absent");
        assert!(dir.join("session-volumes.json").exists());
        store.0.push(missing);
        journal.restore(&mut store).expect("reconnected");
        assert_eq!(store.0[0].value, original);
        assert!(!dir.join("session-volumes.json").exists());
    }

    #[test]
    fn failed_rewrite_keeps_the_earlier_value_restorable() {
        let dir = std::env::temp_dir().join(format!("macafon-rewrite-{}", std::process::id()));
        let original = Value {
            volume: 0.8,
            muted: false,
        };
        let first = Value {
            volume: 0.125,
            muted: false,
        };
        let mut store = Fake(vec![StoredSession {
            key: Key {
                exe: "browser.exe".into(),
                endpoint: "speakers".into(),
            },
            instance: "1".into(),
            value: original,
        }]);
        let mut journal = Journal::open(&dir).expect("journal");
        journal.record(&store.0[0], first).expect("first");
        store.0[0].value = first;
        // The second write is journaled, then SetMasterVolume fails or the daemon crashes.
        let second = Value {
            volume: 0.25,
            ..first
        };
        journal.record(&store.0[0], second).expect("second");
        let mut journal = Journal::open(&dir).expect("restart");
        journal.restore(&mut store).expect("restore");
        assert_eq!(store.0[0].value, original);
        assert_eq!(journal.pending(), 0);
    }
}
