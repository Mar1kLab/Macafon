//! Bounded offline library inventory and duration probing; never runs on the control/audio loop.
use macafon_soundpad::library::{self, SoundEntry};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

pub type Durations = HashMap<String, (u64, Option<u32>)>;
pub struct Inventory {
    pub entries: Vec<SoundEntry>,
    pub categories: Vec<String>,
    pub problems: Vec<(SoundEntry, String)>,
}
pub enum Update {
    Inventory(Inventory),
    Durations(Durations),
    Complete,
}

pub struct Worker {
    requests: Option<mpsc::SyncSender<(u64, PathBuf, bool)>>,
    #[cfg(test)]
    thread: Option<thread::JoinHandle<()>>,
    pub results: mpsc::Receiver<(u64, Update)>,
    revision: Arc<AtomicU64>,
}
impl Worker {
    pub fn new() -> Self {
        let (requests, receiver) = mpsc::sync_channel::<(u64, PathBuf, bool)>(1);
        let (sender, results) = mpsc::sync_channel(2);
        let revision = Arc::new(AtomicU64::new(0));
        let current_revision = Arc::clone(&revision);
        // Dropping the channels cancels the worker without joining slow filesystem I/O on the
        // control loop. Only one scan and at most two result messages can be outstanding.
        let spawned = thread::Builder::new()
            .name("macafon-soundpad-library".into())
            .spawn(move || run(receiver, sender, current_revision));
        if let Err(error) = &spawned {
            tracing::warn!(%error, "Cannot start SoundDeck library worker");
        }
        Self {
            requests: Some(requests),
            #[cfg(test)]
            thread: spawned.ok(),
            results,
            revision,
        }
    }
    pub fn cancel_before(&self, revision: u64) {
        self.revision.store(revision, Ordering::Relaxed);
    }
    pub fn request(&self, revision: u64, folder: PathBuf, force: bool) -> bool {
        self.requests
            .as_ref()
            .is_some_and(|requests| requests.try_send((revision, folder, force)).is_ok())
    }
}

fn run(
    receiver: mpsc::Receiver<(u64, PathBuf, bool)>,
    sender: mpsc::SyncSender<(u64, Update)>,
    current_revision: Arc<AtomicU64>,
) {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        // SAFETY: background mode applies only to this dedicated inventory thread.
        if let Err(error) =
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) }
        {
            tracing::debug!(%error, "Cannot lower library worker CPU/I/O priority");
        }
    }
    let mut folder = PathBuf::new();
    let mut known: HashMap<String, (u64, u64, Option<u32>)> = HashMap::new();
    #[cfg(windows)]
    let mut watcher: Option<Watcher> = None;
    while let Ok((revision, root, force)) = receiver.recv() {
        if root != folder {
            known.clear();
            folder = root.clone();
            #[cfg(windows)]
            {
                let _ = std::fs::create_dir_all(&root);
                watcher = Watcher::new(&root);
            }
        }
        #[cfg(windows)]
        let changed = watcher.as_ref().is_none_or(Watcher::changed);
        #[cfg(windows)]
        if !force && !changed {
            if sender.send((revision, Update::Complete)).is_err() {
                break;
            }
            continue;
        }
        #[cfg(not(windows))]
        let _ = force;
        match library::scan_playable_library(&root) {
            Ok(library::PlayableInventory {
                entries,
                categories,
                problems,
            }) => {
                let probes = entries.clone();
                if sender
                    .send((
                        revision,
                        Update::Inventory(Inventory {
                            entries,
                            categories,
                            problems,
                        }),
                    ))
                    .is_err()
                {
                    break;
                }
                let mut durations = HashMap::with_capacity(probes.len());
                let mut next = HashMap::with_capacity(probes.len());
                for entry in probes {
                    if current_revision.load(Ordering::Relaxed) != revision {
                        break;
                    }
                    let duration = match known.get(&entry.id) {
                        Some(&(modified, size, duration))
                            if modified == entry.modified_unix_ms && size == entry.size_bytes =>
                        {
                            duration
                        }
                        _ => macafon_soundpad::decode::probe_duration_ms(&entry.path)
                            .ok()
                            .flatten(),
                    };
                    durations.insert(entry.id.clone(), (entry.modified_unix_ms, duration));
                    next.insert(
                        entry.id,
                        (entry.modified_unix_ms, entry.size_bytes, duration),
                    );
                }
                known = next;
                if sender
                    .send((revision, Update::Durations(durations)))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                tracing::debug!(%error, folder = %root.display(), "Cannot scan the SoundDeck folder")
            }
        }
        if sender.send((revision, Update::Complete)).is_err() {
            break;
        }
    }
}

#[cfg(windows)]
struct Watcher(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Watcher {
    fn new(root: &std::path::Path) -> Option<Self> {
        use std::os::windows::ffi::OsStrExt;
        use windows::{Win32::Storage::FileSystem::*, core::PCWSTR};
        let root: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: terminated path is live for the call; this worker owns the returned handle.
        unsafe {
            FindFirstChangeNotificationW(
                PCWSTR(root.as_ptr()),
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES
                    | FILE_NOTIFY_CHANGE_CREATION,
            )
        }
        .ok()
        .map(Self)
    }
    fn changed(&self) -> bool {
        use windows::Win32::{
            Foundation::WAIT_TIMEOUT, Storage::FileSystem::FindNextChangeNotification,
            System::Threading::WaitForSingleObject,
        };
        // SAFETY: the worker owns a valid notification handle; zero timeout never waits.
        if unsafe { WaitForSingleObject(self.0, 0) } == WAIT_TIMEOUT {
            return false;
        }
        // SAFETY: rearm the worker-owned notification. A failure keeps fallback scanning active.
        let _ = unsafe { FindNextChangeNotification(self.0) };
        true
    }
}
#[cfg(windows)]
impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: called once for the handle owned by this notification object.
        let _ = unsafe { windows::Win32::Storage::FileSystem::FindCloseChangeNotification(self.0) };
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.revision.store(u64::MAX, Ordering::Relaxed);
        self.requests.take();
        #[cfg(test)]
        {
            // Fixture directories must not be removed while the worker still reads/moves files.
            // Drain the bounded result channel before joining so a pending send cannot wait forever.
            if let Some(thread) = self.thread.take() {
                while !thread.is_finished() {
                    let _ = self
                        .results
                        .recv_timeout(std::time::Duration::from_millis(10));
                }
                let _ = thread.join();
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::Duration;
    fn inventory(worker: &Worker) -> Option<Inventory> {
        let mut inventory = None;
        loop {
            match worker
                .results
                .recv_timeout(Duration::from_secs(5))
                .expect("worker result")
                .1
            {
                Update::Inventory(found) => inventory = Some(found),
                Update::Complete => return inventory,
                Update::Durations(_) => {}
            }
        }
    }
    #[test]
    fn native_notification_skips_idle_walks_and_detects_empty_folder_changes() {
        let root = std::env::temp_dir().join(format!("macafon-watch-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("root");
        let worker = Worker::new();
        worker.cancel_before(1);
        assert!(worker.request(1, root.clone(), true));
        assert!(
            inventory(&worker)
                .expect("initial scan")
                .categories
                .is_empty()
        );
        assert!(worker.request(1, root.clone(), false));
        assert!(
            inventory(&worker).is_none(),
            "idle check walked the library"
        );
        std::fs::create_dir(root.join("Empty")).expect("Explorer create");
        assert!(worker.request(1, root.clone(), false));
        assert_eq!(
            inventory(&worker).expect("created folder scan").categories,
            ["Empty"]
        );
        std::fs::rename(root.join("Empty"), root.join("Renamed")).expect("Explorer rename");
        assert!(worker.request(1, root.clone(), false));
        assert_eq!(
            inventory(&worker).expect("renamed folder scan").categories,
            ["Renamed"]
        );
        std::fs::remove_dir(root.join("Renamed")).expect("Explorer delete");
        assert!(worker.request(1, root.clone(), false));
        assert!(
            inventory(&worker)
                .expect("deleted folder scan")
                .categories
                .is_empty()
        );
        drop(worker);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
