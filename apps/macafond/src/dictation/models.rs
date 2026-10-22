//! Explicit, cancellable model operations. All file and network I/O runs off the host loop.
use anyhow::{Context, Result, bail, ensure};
use macafon_ipc::{
    DictationEngine, DictationModel, DictationModelStatus, DictationMove, DictationStorage,
};
use macafon_stt::manifest::{self, ModelSpec};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

pub enum Update {
    Progress { id: String, bytes: u64, speed: u64 },
    Moving { to: PathBuf, done: u64, total: u64 },
    Scanned(Vec<(String, bool)>, Option<u64>, bool),
    Complete(Result<Option<PathBuf>, String>),
}

pub struct Manager {
    pub folder: PathBuf,
    pub models: Vec<DictationModel>,
    pub storage: DictationStorage,
    pub error: Option<String>,
    pub previous_folder: Option<PathBuf>,
    receiver: Option<mpsc::Receiver<Update>>,
    cancel: Arc<AtomicBool>,
    active_model: Option<String>,
    scanned: bool,
}

pub fn default_folder() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| macafon_config::state_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Macafon/models/stt")
}

pub fn spec(id: &str) -> Result<&'static ModelSpec> {
    manifest::models()
        .iter()
        .find(|m| m.id == id)
        .context("unknown model")
}

/// Redirect hosts are normalized by `Url`; dots delimit subdomains, never string suffixes.
fn allowed_model_host(host: &str) -> bool {
    host == "huggingface.co"
        || host.ends_with(".huggingface.co")
        || host == "hf.co"
        || host.ends_with(".hf.co")
}

impl Manager {
    pub fn new(folder: &str) -> Self {
        let folder = if folder.is_empty() {
            default_folder()
        } else {
            PathBuf::from(folder)
        };
        let storage = DictationStorage {
            folder: folder.to_string_lossy().into_owned(),
            default_folder: folder == default_folder(),
            free_bytes: None,
            used_bytes: 0,
            rotational: false,
            moving: None,
        };
        let models = manifest::models()
            .iter()
            .map(|m| DictationModel {
                id: m.id.into(),
                engine: match m.engine {
                    macafon_stt::protocol::Engine::Whisper => DictationEngine::Whisper,
                    macafon_stt::protocol::Engine::Parakeet => DictationEngine::Parakeet,
                },
                name: m.name.into(),
                size_bytes: m.size_bytes(),
                quality: m.quality,
                speed: m.speed,
                recommended: m.recommended,
                status: DictationModelStatus::Missing,
                downloaded_bytes: 0,
                bytes_per_sec: None,
                error: None,
            })
            .collect();
        Self {
            folder,
            models,
            storage,
            error: None,
            previous_folder: None,
            receiver: None,
            cancel: Arc::default(),
            active_model: None,
            scanned: false,
        }
    }

    fn spawn(
        &mut self,
        task: impl FnOnce(mpsc::Sender<Update>, Arc<AtomicBool>) -> Result<Option<PathBuf>>
        + Send
        + 'static,
    ) -> Result<()> {
        ensure!(
            self.receiver.is_none(),
            "another model operation is running"
        );
        self.cancel = Arc::default();
        self.error = None;
        let folder = self.folder.clone();
        let cancel = self.cancel.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("dictation-models".into())
            .spawn(move || {
                background_priority();
                let result =
                    task(sender.clone(), cancel.clone()).map_err(|error| error.to_string());
                let scan_folder = result
                    .as_ref()
                    .ok()
                    .and_then(|p| p.as_ref())
                    .unwrap_or(&folder);
                let checks = manifest::models()
                    .iter()
                    .map(|m| (m.id.to_owned(), verified_model(scan_folder, m, &cancel)))
                    .collect();
                let _ = sender.send(Update::Scanned(
                    checks,
                    free_bytes(scan_folder),
                    rotational(scan_folder),
                ));
                let _ = sender.send(Update::Complete(result));
            })?;
        self.receiver = Some(receiver);
        Ok(())
    }

    pub fn ensure_scanned(&mut self) {
        if self.scanned {
            return;
        }
        self.scanned = true;
        self.scan();
    }

    fn scan(&mut self) {
        let folder = self.folder.clone();
        let _ = self.spawn(move |sender, cancel| {
            for model in manifest::models() {
                if verified_model(&folder, model, &cancel) {
                    let _ = sender.send(Update::Progress {
                        id: model.id.into(),
                        bytes: model.size_bytes(),
                        speed: 0,
                    });
                }
            }
            Ok(None)
        });
    }

    pub fn ready(&self, id: &str) -> bool {
        self.models
            .iter()
            .any(|m| m.id == id && m.status == DictationModelStatus::Ready)
    }

    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }

    /// Called only after the new config was saved. Failure leaves the old source intact.
    pub fn commit_move(&mut self, saved: bool) {
        let Some(source) = self.previous_folder.take() else {
            return;
        };
        if !saved {
            self.folder = source;
            self.storage.folder = self.folder.to_string_lossy().into_owned();
            self.storage.default_folder = self.folder == default_folder();
            self.error = Some("cannot persist models folder; original folder retained".into());
            return;
        }
        let target = self.folder.clone();
        let _ = self.spawn(move |_, cancel| {
            for model in manifest::models() {
                for entry in model.files {
                    cancelled(&cancel)?;
                    let from = manifest::model_dir(&source, model).join(entry.file_name);
                    if !from.exists() {
                        continue;
                    }
                    reject_links(&from)?;
                    let to = manifest::model_dir(&target, model).join(entry.file_name);
                    // Never delete the only copy, whatever path spelling led here.
                    ensure!(!same_path(&from, &to), "models folder is unchanged");
                    verify(&from, entry.size_bytes, entry.sha256, &cancel)?;
                    verify(&to, entry.size_bytes, entry.sha256, &cancel)?;
                    fs::remove_file(from)?;
                }
            }
            Ok(None)
        });
    }

    pub fn download(&mut self, id: &str) -> Result<()> {
        let model = spec(id)?;
        let folder = self.folder.clone();
        self.spawn(move |sender, cancel| {
            validate_drive(&folder)?;
            reject_links(&folder)?;
            fs::create_dir_all(&folder)?;
            check_space(&folder, model.size_bytes())?;
            let client = reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(30))
                // The pinned origin and the owner-authorized Hugging Face CDN only.
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.url().scheme() == "https"
                        && attempt.url().host_str().is_some_and(allowed_model_host)
                        && attempt.previous().len() <= 5
                    {
                        attempt.follow()
                    } else {
                        attempt
                            .error("model redirect outside the allowed Hugging Face hosts refused")
                    }
                }))
                .build()?;
            let dir = manifest::model_dir(&folder, model);
            reject_links(&dir)?;
            fs::create_dir_all(&dir)?;
            let started = Instant::now();
            let mut tick = Instant::now() - Duration::from_secs(1);
            let mut base = 0;
            for entry in model.files {
                cancelled(&cancel)?;
                let path = dir.join(entry.file_name);
                if verify(&path, entry.size_bytes, entry.sha256, &cancel).is_ok() {
                    base += entry.size_bytes;
                    continue;
                }
                if path.exists() {
                    reject_links(&path)?;
                    fs::remove_file(&path)?;
                }
                let part = path.with_extension(format!(
                    "{}.part",
                    path.extension().and_then(|e| e.to_str()).unwrap_or("")
                ));
                let mut offset = fs::metadata(&part).map_or(0, |m| m.len());
                if offset > entry.size_bytes {
                    fs::remove_file(&part)?;
                    offset = 0;
                }
                while offset < entry.size_bytes {
                    let requested_offset = offset;
                    let mut request = client.get(entry.url());
                    if offset > 0 {
                        request =
                            request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
                    }
                    let mut response = request.send()?.error_for_status()?;
                    offset = resume_offset(
                        response.status().as_u16(),
                        response
                            .headers()
                            .get(reqwest::header::CONTENT_RANGE)
                            .and_then(|h| h.to_str().ok()),
                        offset,
                        entry.size_bytes,
                    )?;
                    let mut output = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(offset == 0)
                        .append(offset != 0)
                        .open(&part)?;
                    let mut buf = [0u8; 64 * 1024];
                    loop {
                        cancelled(&cancel)?;
                        let n = match response.read(&mut buf) {
                            Ok(n) => n,
                            Err(_) if offset > requested_offset => break,
                            Err(error) => return Err(error.into()),
                        };
                        if n == 0 {
                            ensure!(offset == entry.size_bytes, "truncated model response");
                            break;
                        }
                        ensure!(
                            offset + n as u64 <= entry.size_bytes,
                            "model exceeds manifest size"
                        );
                        output.write_all(&buf[..n])?;
                        offset += n as u64;
                        if tick.elapsed() >= Duration::from_millis(250) {
                            let bytes = base + offset;
                            let _ = sender.send(Update::Progress {
                                id: model.id.into(),
                                bytes,
                                speed: (bytes as f64 / started.elapsed().as_secs_f64().max(0.001))
                                    as u64,
                            });
                            tick = Instant::now();
                        }
                    }
                    output.sync_all()?;
                }
                if let Err(error) = verify(&part, entry.size_bytes, entry.sha256, &cancel) {
                    if !cancel.load(Ordering::Relaxed) {
                        let _ = fs::remove_file(&part);
                    }
                    return Err(error);
                }
                cancelled(&cancel)?;
                fs::rename(&part, &path)?;
                base += entry.size_bytes;
            }
            Ok(None)
        })?;
        self.active_model = Some(id.into());
        if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
            m.status = DictationModelStatus::Downloading;
            m.error = None;
        }
        Ok(())
    }

    pub fn cancel_download(&self, id: &str) -> Result<()> {
        ensure!(
            self.active_model.as_deref() == Some(id),
            "model is not downloading"
        );
        self.cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn cancel_move(&self) {
        if self.active_model.is_some() {
            return;
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.receiver.is_none(),
            "another model operation is running"
        );
        let model = spec(id)?;
        let folder = self.folder.clone();
        self.spawn(move |_, cancel| {
            // Never remove user files or directories recursively.
            for entry in model.files {
                cancelled(&cancel)?;
                let path = manifest::model_dir(&folder, model).join(entry.file_name);
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(None)
        })?;
        if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
            m.status = DictationModelStatus::Missing;
        }
        Ok(())
    }

    pub fn import(&mut self, path: PathBuf) -> Result<()> {
        let folder = self.folder.clone();
        self.spawn(move |_, cancel| {
            validate_drive(&folder)?;
            reject_links(&folder)?;
            let size = fs::metadata(&path)?.len();
            let digest = hash(&path, &cancel)?;
            let mut matched = false;
            for model in manifest::models() {
                for entry in model
                    .files
                    .iter()
                    .filter(|entry| entry.size_bytes == size && entry.sha256 == digest)
                {
                    matched = true;
                    let dir = manifest::model_dir(&folder, model);
                    reject_links(&dir)?;
                    fs::create_dir_all(&dir)?;
                    check_space(&dir, size)?;
                    let dest = dir.join(entry.file_name);
                    let part = dest.with_extension("import.part");
                    copy(&path, &part, &cancel, |_| {})?;
                    verify(&part, size, entry.sha256, &cancel)?;
                    fs::rename(part, dest)?;
                }
            }
            ensure!(matched, "file does not match any manifest hash");
            Ok(None)
        })
    }

    pub fn move_to(&mut self, target: PathBuf) -> Result<()> {
        ensure!(target.is_absolute(), "models folder must be absolute");
        let source = self.folder.clone();
        ensure!(target != source, "models folder is unchanged");
        self.spawn(move |sender, cancel| {
            validate_drive(&target)?;
            reject_links(&target)?;
            fs::create_dir_all(&target)?;
            // `..`, letter case or another spelling of the same folder: copying would replace
            // each model with itself and the cleanup would then delete it.
            ensure!(!same_path(&source, &target), "models folder is unchanged");
            let mut files = Vec::new();
            for model in manifest::models() {
                for entry in model.files {
                    let from = manifest::model_dir(&source, model).join(entry.file_name);
                    if from.is_file() {
                        files.push((model, entry, from));
                    }
                }
            }
            let total = files.iter().map(|(_, e, _)| e.size_bytes).sum();
            check_space(&target, total)?;
            let mut done = 0;
            let mut tick = Instant::now() - Duration::from_secs(1);
            for (model, entry, from) in &files {
                cancelled(&cancel)?;
                let dir = manifest::model_dir(&target, model);
                fs::create_dir_all(&dir)?;
                let part = dir.join(format!("{}.move.part", entry.file_name));
                copy(from, &part, &cancel, |bytes| {
                    if tick.elapsed() >= Duration::from_millis(250) {
                        let _ = sender.send(Update::Moving {
                            to: target.clone(),
                            done: done + bytes,
                            total,
                        });
                        tick = Instant::now();
                    }
                })?;
                verify(&part, entry.size_bytes, entry.sha256, &cancel)?;
                fs::rename(part, dir.join(entry.file_name))?;
                done += entry.size_bytes;
            }
            cancelled(&cancel)?;
            // All target copies are verified before any source is deleted. Persisting the
            // new folder is the host's commit point; source cleanup happens afterwards.
            Ok(Some(target))
        })
    }

    /// Returns a completed folder change for persistence, and whether visible state changed.
    pub fn poll(&mut self) -> (bool, Option<PathBuf>) {
        let mut changed = false;
        let mut moved = None;
        let mut complete = None;
        if let Some(receiver) = &self.receiver {
            while let Ok(update) = receiver.try_recv() {
                changed = true;
                match update {
                    Update::Scanned(checks, free, rotational) => {
                        self.storage.free_bytes = free;
                        self.storage.rotational = rotational;
                        for (id, ready) in checks {
                            if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
                                if ready {
                                    m.status = DictationModelStatus::Ready;
                                    m.downloaded_bytes = m.size_bytes;
                                    m.error = None;
                                } else if m.status != DictationModelStatus::Error {
                                    m.status = DictationModelStatus::Missing;
                                    m.downloaded_bytes = 0;
                                }
                            }
                        }
                    }
                    Update::Progress { id, bytes, speed } => {
                        if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
                            m.downloaded_bytes = bytes;
                            m.bytes_per_sec = (speed > 0).then_some(speed);
                            if speed == 0 {
                                m.status = DictationModelStatus::Ready;
                            }
                        }
                    }
                    Update::Moving { to, done, total } => {
                        self.storage.moving = Some(DictationMove {
                            to: to.to_string_lossy().into_owned(),
                            done_bytes: done,
                            total_bytes: total,
                        })
                    }
                    Update::Complete(result) => complete = Some(result),
                }
            }
        }
        if let Some(result) = complete {
            self.error = result.as_ref().err().cloned();
            self.receiver = None;
            self.storage.moving = None;
            if let Some(id) = self.active_model.take() {
                if let Some(m) = self.models.iter_mut().find(|m| m.id == id) {
                    match &result {
                        Ok(_) => {
                            m.status = DictationModelStatus::Ready;
                            m.downloaded_bytes = m.size_bytes;
                        }
                        Err(e) => {
                            m.status = DictationModelStatus::Error;
                            m.error = Some(e.clone());
                        }
                    }
                    m.bytes_per_sec = None;
                }
            }
            if let Ok(Some(folder)) = result {
                self.previous_folder = Some(std::mem::replace(&mut self.folder, folder.clone()));
                self.storage.folder = folder.to_string_lossy().into_owned();
                self.storage.default_folder = folder == default_folder();
                moved = Some(folder);
            }
            self.storage.used_bytes = self
                .models
                .iter()
                .filter(|m| m.status == DictationModelStatus::Ready)
                .map(|m| m.size_bytes)
                .sum();
            // Import/delete/move require a fresh hash scan; do not loop after a scan.
        }
        (changed, moved)
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "operation cancelled");
    Ok(())
}

fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "model paths cannot contain symbolic links"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "model paths cannot contain reparse points"
                );
            }
        }
    }
    Ok(())
}

fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        let mut ancestor = path;
        while !ancestor.exists() {
            ancestor = ancestor.parent()?;
        }
        let text = windows::core::HSTRING::from(ancestor.as_os_str());
        let mut available = 0;
        // SAFETY: path and local output are valid for the synchronous query.
        unsafe {
            windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                &text,
                Some(&mut available),
                None,
                None,
            )
            .ok()?;
        }
        Some(available)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

fn rotational(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::*,
            Storage::FileSystem::*,
            System::{IO::DeviceIoControl, Ioctl::*},
        };
        let Some(prefix) = path.components().next() else {
            return false;
        };
        let text = prefix.as_os_str().to_string_lossy();
        if text.len() != 2 || !text.ends_with(':') {
            return false;
        }
        let name = windows::core::HSTRING::from(format!(r"\\.\{text}"));
        // SAFETY: querying a volume with no write access; buffers are initialized and sized.
        unsafe {
            let Ok(handle) = CreateFileW(
                &name,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            ) else {
                return false;
            };
            let query = STORAGE_PROPERTY_QUERY {
                PropertyId: StorageDeviceSeekPenaltyProperty,
                QueryType: PropertyStandardQuery,
                ..Default::default()
            };
            let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
            let mut bytes = 0;
            let result = DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some(&query as *const _ as _),
                std::mem::size_of_val(&query) as u32,
                Some(&mut descriptor as *mut _ as _),
                std::mem::size_of_val(&descriptor) as u32,
                Some(&mut bytes),
                None,
            );
            let _ = CloseHandle(handle);
            result.is_ok() && descriptor.IncursSeekPenalty
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

pub fn hash(path: &Path, cancel: &AtomicBool) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify(path: &Path, size: u64, digest: &str, cancel: &AtomicBool) -> Result<()> {
    ensure!(fs::metadata(path)?.len() == size, "model size mismatch");
    ensure!(hash(path, cancel)? == digest, "model SHA-256 mismatch");
    Ok(())
}

fn verified_model(folder: &Path, model: &ModelSpec, cancel: &AtomicBool) -> bool {
    model.files.iter().all(|e| {
        verify(
            &manifest::model_dir(folder, model).join(e.file_name),
            e.size_bytes,
            e.sha256,
            cancel,
        )
        .is_ok()
    })
}

fn copy(from: &Path, to: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<()> {
    let mut input = File::open(from)?;
    let mut output = File::create(to)?;
    let mut buf = [0; 64 * 1024];
    let mut done = 0;
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
        done += n as u64;
        progress(done);
    }
    output.sync_all()?;
    Ok(())
}

fn resume_offset(status: u16, range: Option<&str>, offset: u64, total: u64) -> Result<u64> {
    match status {
        200 => Ok(0),
        206 => {
            let expected = format!("bytes {offset}-");
            ensure!(
                range
                    .is_some_and(|s| s.starts_with(&expected) && s.ends_with(&format!("/{total}"))),
                "invalid resume Content-Range"
            );
            Ok(offset)
        }
        _ => bail!("unexpected model HTTP status {status}"),
    }
}

#[cfg(windows)]
fn check_space(path: &Path, bytes: u64) -> Result<()> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let path = windows::core::HSTRING::from(path.as_os_str());
    let mut available = 0;
    // SAFETY: the path and output pointer remain valid for the synchronous query.
    unsafe {
        GetDiskFreeSpaceExW(&path, Some(&mut available), None, None)?;
    }
    ensure!(
        available >= bytes.saturating_add(bytes / 10),
        "not enough free space (10 percent reserve required)"
    );
    Ok(())
}
#[cfg(not(windows))]
fn check_space(_: &Path, _: u64) -> Result<()> {
    bail!("dictation is available on Windows only")
}

#[cfg(windows)]
fn validate_drive(path: &Path) -> Result<()> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
    let text = windows::core::HSTRING::from(path.as_os_str());
    ensure!(
        !path.to_string_lossy().starts_with(r"\\"),
        "network drives are not supported"
    );
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor.parent().context("drive not found")?;
    }
    let existing = windows::core::HSTRING::from(ancestor.as_os_str());
    let mut root = [0u16; 32768];
    // SAFETY: output buffer has the documented length and is NUL terminated by the API.
    unsafe {
        GetVolumePathNameW(&existing, &mut root)?;
        ensure!(
            GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) == 3,
            "only fixed local drives are supported"
        );
    }
    let _ = text;
    Ok(())
}
#[cfg(not(windows))]
fn validate_drive(_: &Path) -> Result<()> {
    bail!("dictation is available on Windows only")
}

fn background_priority() {
    #[cfg(windows)]
    // SAFETY: applies background scheduling only to this model I/O thread.
    unsafe {
        let _ = windows::Win32::System::Threading::SetThreadPriority(
            windows::Win32::System::Threading::GetCurrentThread(),
            windows::Win32::System::Threading::THREAD_MODE_BACKGROUND_BEGIN,
        );
    }
}

/// Both paths exist and resolve to the same file or folder.
fn same_path(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "explicitly downloads Small and exercises the local worker"]
    fn dictation_download_worker_smoke() {
        let folder =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.artifacts/dictation-models");
        let mut manager = Manager::new(&folder.to_string_lossy());
        manager
            .download("whisper-small-q5_1")
            .expect("explicit download");
        let deadline = Instant::now() + Duration::from_secs(180);
        while manager.busy() {
            manager.poll();
            assert!(Instant::now() < deadline, "download timed out");
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            manager.ready("whisper-small-q5_1"),
            "download failed: {:?}",
            manager.error
        );
        use crate::dictation::worker::{Input, Worker};
        use macafon_stt::protocol::*;
        let worker = Worker::spawn().expect("worker with Job and anonymous pipes");
        let spec = spec("whisper-small-q5_1").expect("manifest");
        worker
            .send(WorkerRequest::Load {
                engine: Engine::Whisper,
                model_dir: manifest::model_dir(&folder, spec),
                model_id: spec.id.into(),
                threads: 4,
                gpu: false,
                efficiency: true,
            })
            .expect("load");
        worker
            .send(WorkerRequest::Begin)
            .expect("begin while loading");
        worker
            .input
            .try_send(Input::Audio(vec![0.0; 8000]))
            .expect("silence");
        worker
            .send(WorkerRequest::Finish {
                language: LanguageInput {
                    policy: LanguagePolicy::Fixed,
                    fixed: Language::from_code("en").expect("supported language"),
                    layout: None,
                    last_cyrillic: None,
                    layout_languages: macafon_ipc::installed_dictation_languages()
                        .iter()
                        .filter_map(|info| Language::from_code(&info.code))
                        .collect(),
                },
                vocabulary: vec![],
            })
            .expect("finish");
        let mut ready = false;
        loop {
            match worker
                .events
                .recv_timeout(Duration::from_secs(60))
                .expect("worker response")
                .expect("worker frame")
            {
                WorkerEvent::Loading => {}
                WorkerEvent::Ready { .. } => ready = true,
                WorkerEvent::NoSpeech { .. } => {
                    assert!(ready);
                    break;
                }
                _ => panic!("unexpected worker response to silence"),
            }
        }
    }
    #[test]
    fn another_spelling_of_the_models_folder_is_the_same_folder() {
        let root = std::env::temp_dir().join(format!("macafon-same-path-{}", std::process::id()));
        let folder = root.join("Models");
        fs::create_dir_all(&folder).expect("folder");
        assert!(same_path(&folder, &folder.join("..").join("Models")));
        #[cfg(windows)]
        assert!(same_path(&folder, &root.join("MODELS")));
        assert!(!same_path(&folder, &root));
        assert!(!same_path(&folder, &root.join("missing")));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn model_redirect_hosts_are_limited_to_hugging_face_domains() {
        for host in [
            "huggingface.co",
            "hf.co",
            "us.aws.cdn.hf.co",
            "cdn-lfs.huggingface.co",
            "cdn-lfs-us-1.huggingface.co",
        ] {
            assert!(allowed_model_host(host), "{host}");
        }
        for host in [
            "evil-hf.co",
            "nothuggingface.co",
            "hf.co.evil.test",
            "huggingface.co.evil.test",
            "example.com",
            "",
            "https://hf.co",
            "hf.co@evil.test",
        ] {
            assert!(!allowed_model_host(host), "{host}");
        }
    }
    #[test]
    fn fixture_hash_and_resume_validation() {
        let folder =
            std::env::temp_dir().join(format!("macafon-model-test-{}", std::process::id()));
        fs::create_dir_all(&folder).expect("folder");
        let path = folder.join("fixture");
        fs::write(&path, b"abc").expect("fixture");
        let cancel = AtomicBool::new(false);
        assert!(
            verify(
                &path,
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                &cancel
            )
            .is_ok()
        );
        assert!(verify(&path, 3, "bad", &cancel).is_err());
        assert_eq!(
            resume_offset(206, Some("bytes 3-9/10"), 3, 10).expect("resume"),
            3
        );
        assert_eq!(resume_offset(200, None, 3, 10).expect("restart"), 0);
        assert!(resume_offset(206, Some("bytes 0-9/10"), 3, 10).is_err());
        fs::remove_file(path).expect("cleanup");
        fs::remove_dir(folder).expect("cleanup");
    }
}
