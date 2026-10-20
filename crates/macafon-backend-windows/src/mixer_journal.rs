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

#[derive(Debug, PartialEq, Serialize, Deserialize)]
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

/// The value a session of one app had before Macafon, from what it shows now. Windows keeps
/// an app's volume for its next sessions, so a session may start at a value Macafon set for
/// an earlier one: follow those records back until the value is not Macafon's. `entries` are
/// oldest first; `first` is tried first (the session's own record), then the newest, and each
/// step only goes to an older record.
fn original(entries: &[&Entry], first: Option<usize>, current: Value) -> Value {
    fn trace<T: Copy>(
        entries: &[&Entry],
        first: Option<usize>,
        mut value: T,
        ours: impl Fn(&Entry, T) -> bool,
        previous: impl Fn(&Entry) -> T,
        same: impl Fn(T, T) -> bool,
    ) -> T {
        let mut first = first;
        let mut older_than = entries.len();
        loop {
            let next = first
                .take()
                .into_iter()
                .chain((0..older_than).rev())
                .find(|&i| ours(entries[i], value) && !same(previous(entries[i]), value));
            let Some(i) = next else { return value };
            older_than = i;
            value = previous(entries[i]);
        }
    }
    Value {
        volume: trace(
            entries,
            first,
            current.volume,
            Entry::is_ours_volume,
            |entry| entry.previous.volume,
            close,
        ),
        muted: trace(
            entries,
            first,
            current.muted,
            Entry::is_ours_mute,
            |entry| entry.previous.muted,
            |a, b| a == b,
        ),
    }
}

pub(crate) struct Journal {
    path: PathBuf,
    entries: Vec<Entry>,
    /// Records not on disk yet: `commit` writes them before their COM writes.
    dirty: bool,
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
        Ok(Self {
            path,
            entries,
            dirty: false,
        })
    }

    /// Note the value about to be written to `session`; `commit` makes it durable. A new
    /// session of an app takes over the records of its ended sessions (`live` tells which
    /// session ids still exist), so the journal holds one record per live session.
    pub fn record(&mut self, session: &StoredSession, ours: Value, live: impl Fn(&str) -> bool) {
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
            let related: Vec<&Entry> = self
                .entries
                .iter()
                .filter(|entry| entry.key == session.key)
                .collect();
            let previous = original(&related, None, session.value);
            self.entries
                .retain(|entry| entry.key != session.key || live(&entry.instance));
            self.entries.push(Entry {
                key: session.key.clone(),
                instance: session.instance.clone(),
                previous,
                ours,
                pending: None,
            });
        }
        self.dirty = true;
    }

    /// Write the records noted since the last write, once for a whole batch of sessions.
    pub fn commit(&mut self) -> Result<(), Error> {
        if self.dirty {
            self.save()?;
        }
        Ok(())
    }

    pub fn restore(&mut self, store: &mut impl SessionStore) -> Result<(), Error> {
        self.restore_matching(store, None)
    }

    pub fn restore_app(&mut self, store: &mut impl SessionStore, exe: &str) -> Result<(), Error> {
        self.restore_matching(store, Some(exe))
    }

    /// Give every present session of the journaled apps its original value back, then drop
    /// their records. An absent app or endpoint keeps one record for a later start: Windows
    /// only remembers the last value Macafon set for it, the newest record's.
    fn restore_matching(
        &mut self,
        store: &mut impl SessionStore,
        exe: Option<&str>,
    ) -> Result<(), Error> {
        let sessions = store.list()?;
        let mut keys: Vec<Key> = Vec::new();
        for entry in &self.entries {
            if exe.is_none_or(|exe| entry.key.exe == exe) && !keys.contains(&entry.key) {
                keys.push(entry.key.clone());
            }
        }
        let mut completed = Vec::new();
        for key in keys {
            let present: Vec<_> = sessions.iter().filter(|s| s.key == key).collect();
            let related: Vec<&Entry> = self.entries.iter().filter(|e| e.key == key).collect();
            if present.is_empty() {
                if let [older @ .., newest] = related.as_slice()
                    && !older.is_empty()
                {
                    let previous = original(older, None, newest.previous);
                    let mut left = related.len();
                    self.entries.retain(|e| {
                        if e.key != key {
                            return true;
                        }
                        left -= 1;
                        left == 0 // The newest record of this key is its last one.
                    });
                    if let Some(kept) = self.entries.iter_mut().find(|e| e.key == key) {
                        kept.previous = previous;
                    }
                }
                continue;
            }
            for session in present {
                let own = related.iter().position(|e| e.instance == session.instance);
                let value = original(&related, own, session.value);
                if value != session.value {
                    store.set(&session.instance, value)?;
                }
            }
            completed.push(key);
        }
        self.entries.retain(|entry| !completed.contains(&entry.key));
        self.save()
    }

    fn save(&mut self) -> Result<(), Error> {
        if self.entries.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Ok(()) => {
                    self.dirty = false;
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.dirty = false;
                    Ok(())
                }
                Err(e) => Err(error(e)),
            };
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary).map_err(error)?;
        // Compact: rewritten for every batch of volume changes while a key is held.
        let bytes = serde_json::to_vec(&self.entries).map_err(error)?;
        std::io::Write::write_all(&mut file, &bytes).map_err(error)?;
        file.sync_all().map_err(error)?;
        drop(file);
        std::fs::rename(temporary, &self.path).map_err(error)?;
        self.dirty = false;
        Ok(())
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
            journal.record(session, ours, |_| true);
            journal.commit().expect("record");
            session.value = ours;
        }
        journal
            .restore_app(&mut store, "browser.exe")
            .expect("unmanage");
        assert_eq!(store.0[0].value, original);
        assert_eq!(store.0[1].value, ours);
        assert_eq!(journal.pending(), 1);

        // Re-managing starts a new journal entry from the restored session value.
        journal.record(&store.0[0], ours, |_| true);
        journal.commit().expect("manage again");
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
        journal.record(&store.0[0], ours, |_| true);
        journal.commit().expect("record before mutation");
        store.0[0].value = ours;
        journal.record(
            &store.0[0],
            Value {
                volume: 0.25,
                ..ours
            },
            |_| true,
        );
        journal.commit().expect("second change");
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
        journal.record(&store.0[0], ours, |_| true);
        journal.commit().expect("record");
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
        journal.record(&store.0[0], first, |_| true);
        journal.commit().expect("first");
        store.0[0].value = first;
        // The second write is journaled, then SetMasterVolume fails or the daemon crashes.
        let second = Value {
            volume: 0.25,
            ..first
        };
        journal.record(&store.0[0], second, |_| true);
        journal.commit().expect("second");
        let mut journal = Journal::open(&dir).expect("restart");
        journal.restore(&mut store).expect("restore");
        assert_eq!(store.0[0].value, original);
        assert_eq!(journal.pending(), 0);
    }

    #[test]
    fn relaunched_app_keeps_one_record_and_its_true_original() {
        let dir = std::env::temp_dir().join(format!("macafon-relaunch-{}", std::process::id()));
        let key = Key {
            exe: "game.exe".into(),
            endpoint: "speakers".into(),
        };
        let original = Value {
            volume: 1.0,
            muted: false,
        };
        let ours = Value {
            volume: 0.125,
            muted: false,
        };
        let mut store = Fake(vec![StoredSession {
            key: key.clone(),
            instance: "pid-0".into(),
            value: original,
        }]);
        let mut journal = Journal::open(&dir).expect("journal");
        for launch in 0..5 {
            let live = format!("pid-{launch}");
            store.0[0].instance = live.clone();
            // Windows gives a new session the volume Macafon set for the last one.
            journal.record(&store.0[0], ours, |instance| instance == live);
            journal.commit().expect("record");
            store.0[0].value = ours;
            assert_eq!(journal.pending(), 1, "one record per live session");
        }
        // A journal left by an older build: several records that disagree on the original.
        journal.entries.push(Entry {
            key: key.clone(),
            instance: "pid-old".into(),
            previous: ours,
            ours,
            pending: None,
        });
        journal.commit().expect("older records");
        let mut journal = Journal::open(&dir).expect("restart");
        store.0[0].instance = "pid-new".into();
        journal.restore(&mut store).expect("restore");
        assert_eq!(store.0[0].value, original);
        assert_eq!(journal.pending(), 0);
    }

    #[test]
    fn absent_app_keeps_only_its_newest_record_with_the_true_original() {
        let dir = std::env::temp_dir().join(format!("macafon-absent-{}", std::process::id()));
        let key = Key {
            exe: "game.exe".into(),
            endpoint: "speakers".into(),
        };
        let value = |volume| Value {
            volume,
            muted: false,
        };
        let mut journal = Journal::open(&dir).expect("journal");
        // Three launches by an older build, each starting at the volume set for the last.
        for (instance, previous, ours) in [("a", 1.0, 0.125), ("b", 0.125, 0.3), ("c", 0.3, 0.2)] {
            journal.entries.push(Entry {
                key: key.clone(),
                instance: instance.into(),
                previous: value(previous),
                ours: value(ours),
                pending: None,
            });
        }
        let mut store = Fake(Vec::new());
        journal.restore(&mut store).expect("absent");
        assert_eq!(journal.pending(), 1);
        store.0.push(StoredSession {
            key,
            instance: "d".into(),
            value: value(0.2),
        });
        let mut journal = Journal::open(&dir).expect("restart");
        journal.restore(&mut store).expect("back");
        assert_eq!(store.0[0].value, value(1.0));
        assert_eq!(journal.pending(), 0);
    }
}
