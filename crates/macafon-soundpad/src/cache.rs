//! Single-worker offline loading and a bounded decoded-sound LRU.
use crate::{
    Error,
    decode::{Sound, decode_file},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    modified_unix_ms: u64,
    generation: u64,
}
struct Request {
    id: String,
    key: Key,
}
struct Finished {
    request: Request,
    result: Result<Arc<Sound>, Error>,
}
struct Cached {
    key: Key,
    sound: Arc<Sound>,
    used: u64,
}
pub struct LoadResult {
    pub id: String,
    pub result: Result<Arc<Sound>, Error>,
}
pub struct SoundLoader {
    sender: Option<mpsc::Sender<Request>>,
    receiver: mpsc::Receiver<Finished>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    cache: HashMap<String, Cached>,
    pending: HashMap<String, Key>,
    failures: Vec<LoadResult>,
    budget_bytes: usize,
    used_bytes: usize,
    serial: u64,
}
impl SoundLoader {
    /// Starts one worker. The daemon normally supplies a 512 MiB budget.
    pub fn new(budget_bytes: usize) -> Self {
        let (sender, requests) = mpsc::channel::<Request>();
        let (results, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("macafon-soundpad-loader".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = decode_file(&request.key.path).map(Arc::new);
                    if results.send(Finished { request, result }).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Self {
            sender: worker.as_ref().map(|_| sender),
            receiver,
            worker,
            stop,
            cache: HashMap::new(),
            pending: HashMap::new(),
            failures: Vec::new(),
            budget_bytes,
            used_bytes: 0,
            serial: 0,
        }
    }
    pub fn get(&mut self, id: &str, path: &Path, modified_unix_ms: u64) -> Option<Arc<Sound>> {
        let cached = self.cache.get_mut(id)?;
        if cached.key.path != path || cached.key.modified_unix_ms != modified_unix_ms {
            return None;
        }
        self.serial = self.serial.wrapping_add(1);
        cached.used = self.serial;
        Some(Arc::clone(&cached.sound))
    }
    pub fn request(&mut self, id: &str, path: &Path, modified_unix_ms: u64) {
        if self.get(id, path, modified_unix_ms).is_some()
            || self
                .pending
                .get(id)
                .is_some_and(|k| k.path == path && k.modified_unix_ms == modified_unix_ms)
        {
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        let key = Key {
            path: path.into(),
            modified_unix_ms,
            generation: self.serial,
        };
        let request = Request {
            id: id.into(),
            key: key.clone(),
        };
        if self
            .sender
            .as_ref()
            .is_some_and(|s| s.send(request).is_ok())
        {
            self.pending.insert(id.into(), key);
        } else {
            self.failures.push(LoadResult {
                id: id.into(),
                result: Err(Error::WorkerUnavailable),
            });
        }
    }
    /// Returns current results; invalidated or superseded requests are discarded.
    /// A sound larger than the budget is returned but is not cached.
    pub fn poll(&mut self) -> Vec<LoadResult> {
        let mut results = std::mem::take(&mut self.failures);
        while let Ok(finished) = self.receiver.try_recv() {
            let Request { id, key } = finished.request;
            if self.pending.get(&id) != Some(&key) {
                continue;
            }
            self.pending.remove(&id);
            if let Ok(sound) = &finished.result {
                self.remove(&id);
                let bytes = sound.byte_size();
                if bytes <= self.budget_bytes {
                    while self.used_bytes > self.budget_bytes - bytes {
                        let oldest = self
                            .cache
                            .iter()
                            .min_by_key(|(_, c)| c.used)
                            .map(|(id, _)| id.clone());
                        if let Some(oldest) = oldest {
                            self.remove(&oldest);
                        } else {
                            break;
                        }
                    }
                    self.serial = self.serial.wrapping_add(1);
                    self.used_bytes += bytes;
                    self.cache.insert(
                        id.clone(),
                        Cached {
                            key,
                            sound: Arc::clone(sound),
                            used: self.serial,
                        },
                    );
                }
            }
            results.push(LoadResult {
                id,
                result: finished.result,
            });
        }
        results
    }
    fn remove(&mut self, id: &str) {
        if let Some(cached) = self.cache.remove(id) {
            self.used_bytes -= cached.sound.byte_size();
        }
    }
    pub fn invalidate(&mut self, id: &str) {
        self.remove(id);
        self.pending.remove(id);
    }
    pub fn clear(&mut self) {
        self.cache.clear();
        self.pending.clear();
        self.failures.clear();
        self.used_bytes = 0;
    }
}
impl Drop for SoundLoader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::time::{Duration, Instant};
    fn wait(loader: &mut SoundLoader) -> Vec<LoadResult> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = loader.poll();
            if !result.is_empty() {
                return result;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn load_deduplicate_invalidate_and_evict() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tone.wav");
        crate::decode::tests::wav(&path, 48_000, 480);
        let mut loader = SoundLoader::new(2 * 480 * 8);
        loader.request("a", &path, 1);
        loader.request("a", &path, 1);
        let result = wait(&mut loader);
        assert_eq!(result.len(), 1);
        assert!(result[0].result.is_ok());
        assert!(loader.get("a", &path, 1).is_some());
        assert!(loader.get("a", &path, 2).is_none());
        loader.request("b", &path, 1);
        wait(&mut loader);
        assert!(loader.get("a", &path, 1).is_some()); // Bump A ahead of B.
        loader.request("c", &path, 1);
        wait(&mut loader);
        assert!(loader.get("b", &path, 1).is_none());
        assert!(loader.get("a", &path, 1).is_some());
        assert!(loader.get("c", &path, 1).is_some());
        loader.request("d", &path, 1);
        loader.invalidate("d");
        loader.request("e", &path, 1);
        wait(&mut loader);
        assert!(loader.get("d", &path, 1).is_none());
        assert!(loader.get("e", &path, 1).is_some());
        loader.clear();
        assert!(loader.get("e", &path, 1).is_none());
    }
}
