//! User-initiated yt-dlp jobs. All subprocess I/O and probing happen on workers.

use macafon_ipc::{
    DownloaderInfo, ErrorBody, ErrorCode, Outcome, SoundDeckDownload,
    SoundDeckDownloadState as State,
};
use macafon_sounddeck::library;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const POLL: Duration = Duration::from_millis(20);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const DETECT_TIMEOUT: Duration = Duration::from_secs(2);
const NO_PROGRESS: u32 = u32::MAX;
const LINE_BYTES: usize = 8192;

pub(super) fn validate_url(value: &str) -> Result<(), ErrorBody> {
    let valid = value.len() <= 8192
        && value.split_once("://").is_some_and(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
        })
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
        && reqwest::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    if valid {
        Ok(())
    } else {
        Err(ErrorBody::new(
            ErrorCode::InvalidParams,
            "expected an http or https URL",
        ))
    }
}

fn arguments(url: &str, output: &Path, ffmpeg: bool) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = [
        "--ignore-config",
        "--no-playlist",
        "--no-exec",
        "--no-plugin-dirs",
        "--newline",
        "--restrict-filenames",
        "--windows-filenames",
        "--no-simulate",
        "--max-downloads",
        "1",
        "--max-filesize",
        "200M",
        "--match-filter",
        "duration <= 600",
        "--socket-timeout",
        "15",
        "--retries",
        "3",
        "--encoding",
        "utf-8",
        // YouTube needs a JS runtime for its challenges; yt-dlp enables only Deno by default,
        // while Node or Bun is often what the user has (on PATH, found by yt-dlp itself).
        "--js-runtimes",
        "deno",
        "--js-runtimes",
        "node",
        "--js-runtimes",
        "bun",
        "--progress",
        "--progress-delta",
        "0.25",
        "--progress-template",
        "download:MACAFON_PROGRESS:%(progress._percent_str)s",
        "--progress-template",
        "postprocess:MACAFON_CONVERTING",
        "--print",
        "before_dl:MACAFON_TITLE:%(title)j",
        "--print",
        "after_move:MACAFON_FILE:%(filepath)j",
        "-f",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if ffmpeg {
        args.extend(["bestaudio/best", "-x", "--audio-format", "mp3"].map(Into::into));
    } else {
        args.push(
            library::SUPPORTED_EXTENSIONS
                .iter()
                .map(|ext| format!("bestaudio[ext={ext}]"))
                .collect::<Vec<_>>()
                .join("/")
                .into(),
        );
    }
    args.push("-o".into());
    // yt-dlp treats the whole output path as a template, including the user's folder.
    // Escape percent signs there while retaining the one intended extension placeholder.
    let mut template = output
        .parent()
        .unwrap_or(Path::new("."))
        .to_string_lossy()
        .replace('%', "%%");
    template.push(std::path::MAIN_SEPARATOR);
    template.push_str("media.%(ext)s");
    args.push(template.into());
    args.push("--".into());
    args.push(url.into());
    args
}

#[derive(Debug, PartialEq)]
enum Line {
    Progress(f32),
    Title(String),
    File(PathBuf),
    Converting,
}

fn parse_line(line: &str) -> Option<Line> {
    if let Some(value) = line.strip_prefix("MACAFON_PROGRESS:") {
        let pct: f32 = value.trim().trim_end_matches('%').trim().parse().ok()?;
        return pct
            .is_finite()
            .then_some(Line::Progress(pct.clamp(0.0, 100.0)));
    }
    if let Some(value) = line.strip_prefix("MACAFON_TITLE:") {
        return serde_json::from_str(value).ok().map(Line::Title);
    }
    if let Some(value) = line.strip_prefix("MACAFON_FILE:") {
        return serde_json::from_str::<String>(value)
            .ok()
            .map(|value| Line::File(value.into()));
    }
    (line == "MACAFON_CONVERTING").then_some(Line::Converting)
}

/// `--max-downloads 1` makes yt-dlp exit with 101 after the one download it allows, so 101
/// with a finished file is a success.
fn finished(code: Option<i32>, file: bool) -> bool {
    code == Some(0) || (code == Some(101) && file)
}

fn short_error(line: &str, ffmpeg: bool) -> String {
    let line = line
        .trim()
        .strip_prefix("ERROR:")
        .unwrap_or(line.trim())
        .trim();
    if !ffmpeg
        && (line.contains("Requested format is not available") || line.contains("No video formats"))
    {
        return "No supported audio format is available; install ffmpeg and re-check the downloader".into();
    }
    let clean: String = line
        .chars()
        .filter(|ch| !ch.is_control())
        .take(300)
        .collect();
    if clean.is_empty() {
        "yt-dlp failed to download this sound".into()
    } else {
        clean
    }
}

fn title_name(title: &str) -> String {
    let mut bytes = 0;
    let name: String = title
        .chars()
        .map(|ch| {
            if ch.is_control() || "/\\:*?\"<>|".contains(ch) {
                '_'
            } else {
                ch
            }
        })
        .take_while(|ch| {
            bytes += ch.len_utf8();
            bytes <= 150
        })
        .collect();
    let name = name.trim_matches(['.', ' ']);
    if name.is_empty() {
        return "Sound".into();
    }
    if library::validate_name(name).is_err() {
        format!("Sound {name}")
    } else {
        name.into()
    }
}

/// Owns only a freshly-created job directory under the library's ignored staging directory.
pub(super) struct Staged {
    directory: PathBuf,
    pub path: PathBuf,
}
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub(super) enum Update {
    Downloading,
    Title(String),
    Converting,
    Finished(Result<Staged, String>),
}

struct Job {
    status: SoundDeckDownload,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    updates: mpsc::Receiver<Update>,
    thread: Option<JoinHandle<()>>,
}
impl Job {
    fn active(&self) -> bool {
        matches!(
            self.status.state,
            State::Queued | State::Downloading | State::Converting
        )
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Detection {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<(DownloaderInfo, Option<PathBuf>)>,
    thread: Option<JoinHandle<()>>,
    reply: Option<mpsc::Sender<Outcome>>,
}
impl Drop for Detection {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub(super) struct Downloads {
    pub info: DownloaderInfo,
    ffmpeg_directory: Option<PathBuf>,
    detection: Option<Detection>,
    jobs: Vec<Job>,
    retiring: Vec<Job>,
    events: Vec<Vec<SoundDeckDownload>>,
    progress_at: Option<Instant>,
}

impl Downloads {
    pub fn active(&self) -> bool {
        self.jobs.iter().chain(&self.retiring).any(|job| {
            job.active()
                || job
                    .thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
        })
    }
    pub fn snapshot(&self) -> Vec<SoundDeckDownload> {
        self.jobs.iter().map(|job| job.status.clone()).collect()
    }
    fn changed(&mut self) {
        self.events.push(self.snapshot());
    }
    pub fn take_events(&mut self) -> Vec<Vec<SoundDeckDownload>> {
        std::mem::take(&mut self.events)
    }

    pub fn check(
        &mut self,
        path: Option<PathBuf>,
        reply: Option<mpsc::Sender<Outcome>>,
    ) -> Result<(), ErrorBody> {
        if self.detection.is_some() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "downloader detection is already running",
            ));
        }
        let (send, result) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("sounddeck-detect".into())
            .spawn(move || {
                let info = detect(path, &flag);
                let directory = crate::updates::tools::managed_ffmpeg_directory();
                let _ = send.send((info, directory));
            })
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))?;
        self.detection = Some(Detection {
            cancel,
            result,
            thread: Some(thread),
            reply,
        });
        Ok(())
    }

    pub fn poll_detection(&mut self) -> bool {
        let result = self
            .detection
            .as_ref()
            .and_then(|detection| detection.result.try_recv().ok());
        let Some((info, directory)) = result else {
            return false;
        };
        if let Some(mut detection) = self.detection.take()
            && let Some(reply) = detection.reply.take()
        {
            let _ = reply.send(Outcome::Result(serde_json::json!(&info)));
        }
        self.info = info;
        self.ffmpeg_directory = directory;
        true
    }

    pub fn start(
        &mut self,
        root: &Path,
        url: String,
        category: Option<String>,
    ) -> Result<String, ErrorBody> {
        validate_url(&url)?;
        if self.detection.is_some() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "wait for downloader detection to finish",
            ));
        }
        if let Some(category) = &category {
            library::validate_name(category).map_err(super::library_error)?;
        }
        // Cancelled processes still occupy a slot until their worker has exited.
        if self
            .jobs
            .iter()
            .chain(&self.retiring)
            .filter(|job| {
                job.active()
                    || job
                        .thread
                        .as_ref()
                        .is_some_and(|thread| !thread.is_finished())
            })
            .count()
            >= 3
        {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "at most three SoundDeck downloads can run at once",
            ));
        }
        let executable = self
            .info
            .path
            .as_ref()
            .filter(|_| self.info.available)
            .ok_or_else(|| {
                ErrorBody::new(
                    ErrorCode::NotFound,
                    "yt-dlp is unavailable; configure its path or install it on PATH and re-check",
                )
            })?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = format!(
            "download-{}-{timestamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let (send, updates) = mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU32::new(NO_PROGRESS));
        let task = Task {
            root: root.into(),
            id: id.clone(),
            url: url.clone(),
            executable: executable.into(),
            ffmpeg: self.info.ffmpeg,
            ffmpeg_directory: self.ffmpeg_directory.clone(),
            cancel: cancel.clone(),
            progress: progress.clone(),
            send,
        };
        let thread = std::thread::Builder::new()
            .name("sounddeck-download".into())
            .spawn(move || {
                let result = task.run();
                let _ = task.send.send(Update::Finished(result));
            })
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))?;
        self.jobs.push(Job {
            status: SoundDeckDownload {
                id: id.clone(),
                url,
                category,
                title: None,
                state: State::Queued,
                progress_pct: None,
                error: None,
                sound_id: None,
            },
            cancel,
            progress,
            updates,
            thread: Some(thread),
        });
        self.changed();
        Ok(id)
    }

    pub fn cancel(&mut self, id: &str) -> Result<(), ErrorBody> {
        let job = self
            .jobs
            .iter_mut()
            .find(|job| job.status.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "unknown download"))?;
        if job.active() {
            job.cancel.store(true, Ordering::Relaxed);
            job.status.state = State::Cancelled;
            job.status.progress_pct = None;
            self.changed();
        }
        Ok(())
    }

    pub fn cancel_all(&mut self) {
        let ids: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| job.active())
            .map(|job| job.status.id.clone())
            .collect();
        for id in ids {
            let _ = self.cancel(&id);
        }
    }

    pub fn dismiss(&mut self, id: &str) -> Result<(), ErrorBody> {
        let index = self
            .jobs
            .iter()
            .position(|job| job.status.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "unknown download"))?;
        let job = &self.jobs[index];
        if job.active() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "download has not finished stopping",
            ));
        }
        self.retiring.push(self.jobs.remove(index));
        self.changed();
        Ok(())
    }

    /// Poll state transitions separately so the host publishes every transition, even if a
    /// fast worker completes between two control-loop ticks. Progress occupies one atomic slot.
    pub fn poll(&mut self, now: Instant) -> Vec<(String, Staged, Option<String>)> {
        self.retiring.retain(|job| {
            job.thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
        });
        let mut completed = Vec::new();
        for index in 0..self.jobs.len() {
            while let Ok(update) = self.jobs[index].updates.try_recv() {
                let job = &mut self.jobs[index];
                if !job.active() {
                    continue;
                }
                match update {
                    Update::Downloading => job.status.state = State::Downloading,
                    Update::Title(title) => job.status.title = Some(title),
                    Update::Converting => {
                        job.status.state = State::Converting;
                        job.status.progress_pct = None;
                    }
                    Update::Finished(Ok(staged)) => {
                        completed.push((
                            job.status.id.clone(),
                            staged,
                            job.status.category.clone(),
                        ));
                        continue;
                    }
                    Update::Finished(Err(error)) => {
                        job.status.state = State::Failed;
                        job.status.error = Some(error);
                    }
                }
                self.changed();
            }
        }
        if self
            .progress_at
            .is_none_or(|at| now.saturating_duration_since(at) >= PROGRESS_INTERVAL)
        {
            let mut changed = false;
            for job in &mut self.jobs {
                if job.status.state == State::Downloading {
                    let bits = job.progress.swap(NO_PROGRESS, Ordering::Relaxed);
                    if bits != NO_PROGRESS {
                        job.status.progress_pct = Some(f32::from_bits(bits));
                        changed = true;
                    }
                }
            }
            if changed {
                self.progress_at = Some(now);
                self.changed();
            }
        }
        completed
    }

    pub fn finished(&mut self, id: &str, result: Result<String, ErrorBody>) {
        if let Some(job) = self.jobs.iter_mut().find(|job| job.status.id == id) {
            match result {
                Ok(sound_id) => {
                    job.status.state = State::Done;
                    job.status.sound_id = Some(sound_id);
                    job.status.progress_pct = Some(100.0);
                }
                Err(error) => {
                    job.status.state = State::Failed;
                    job.status.error = Some(error.message);
                }
            }
            self.changed();
        }
    }
}

impl Drop for Downloads {
    fn drop(&mut self) {
        // Signal every worker before joining any of them.
        for job in self.jobs.iter().chain(&self.retiring) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

struct Task {
    root: PathBuf,
    id: String,
    url: String,
    executable: PathBuf,
    ffmpeg: bool,
    ffmpeg_directory: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    send: mpsc::SyncSender<Update>,
}

impl Task {
    fn run(&self) -> Result<Staged, String> {
        let directory = library::upload_directory(&self.root)
            .map_err(|error| error.to_string())?
            .join(&self.id);
        std::fs::create_dir(&directory).map_err(|error| error.to_string())?;
        let mut staged = Staged {
            path: directory.join("media"),
            directory,
        };
        let output = staged.directory.join("media.%(ext)s");
        let mut command = Command::new(&self.executable);
        if let Some(directory) = &self.ffmpeg_directory {
            command.arg("--ffmpeg-location").arg(directory);
        }
        command.args(arguments(&self.url, &output, self.ffmpeg));
        let mut process = Process::spawn(command).map_err(|error| error.to_string())?;
        let _ = self.send.send(Update::Downloading);
        let stdout = process
            .child
            .stdout
            .take()
            .ok_or("yt-dlp stdout unavailable")?;
        let stderr = process
            .child
            .stderr
            .take()
            .ok_or("yt-dlp stderr unavailable")?;
        let (send, lines) = mpsc::sync_channel(32);
        let readers = [read_lines(stdout, send.clone()), read_lines(stderr, send)];
        let mut title = None;
        let mut file = None;
        let mut error = String::new();
        let mut converting = false;
        let mut status = None;
        let mut disconnected = false;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            match lines.recv_timeout(POLL) {
                Ok(line) => match parse_line(&line) {
                    Some(Line::Progress(pct)) => {
                        self.progress.store(pct.to_bits(), Ordering::Relaxed)
                    }
                    Some(Line::Title(value)) if title.is_none() => {
                        title = Some(value.clone());
                        let _ = self.send.send(Update::Title(value));
                    }
                    Some(Line::File(value)) => file = Some(value),
                    Some(Line::Converting) if !converting => {
                        converting = true;
                        let _ = self.send.send(Update::Converting);
                    }
                    Some(_) => {}
                    // Only yt-dlp's own errors explain a failure; warnings and info lines
                    // (missing JS runtime, "Deleting original file") do not.
                    None if line.trim_start().starts_with("ERROR:") => {
                        error = short_error(&line, self.ffmpeg)
                    }
                    None => {}
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if status.is_none() {
                status = process
                    .child
                    .try_wait()
                    .map_err(|error| error.to_string())?;
            }
            if status.is_some() && disconnected {
                break;
            }
            if disconnected {
                std::thread::sleep(POLL);
            }
        }
        // Kill the whole process tree before waiting for pipe readers (ffmpeg can inherit pipes).
        drop(process);
        drop(lines);
        for reader in readers.into_iter().flatten() {
            let _ = reader.join();
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err("Download cancelled".into());
        }
        if !status.is_some_and(|status| finished(status.code(), file.is_some())) {
            return Err(short_error(&error, self.ffmpeg));
        }
        let file = file.ok_or_else(|| {
            if error.is_empty() {
                "No audio downloaded (the sound must be at most 10 minutes and 200 MB)".into()
            } else {
                error
            }
        })?;
        let metadata = std::fs::symlink_metadata(&file).map_err(|error| error.to_string())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || file.parent().and_then(|path| path.canonicalize().ok())
                != staged.directory.canonicalize().ok()
            || metadata.len() > library::MAX_FILE_BYTES
        {
            return Err("Invalid downloaded audio file or file exceeds 200 MB".into());
        }
        let extension = file
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !library::SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
            return Err(
                "Unsupported audio format; install ffmpeg and re-check the downloader".into(),
            );
        }
        let duration = macafon_sounddeck::decode::probe_duration_ms(&file)
            .map_err(|error| error.to_string())?;
        if duration.is_none_or(|ms| ms > 600_000) {
            return Err("Downloaded sound exceeds 10 minutes or has no readable duration".into());
        }
        macafon_sounddeck::decode::probe_playable(&file).map_err(|error| error.to_string())?;
        staged.path = staged.directory.join(format!(
            "{}.{}",
            title_name(title.as_deref().unwrap_or("Sound")),
            extension
        ));
        if staged.path != file {
            std::fs::rename(&file, &staged.path).map_err(|error| error.to_string())?;
        }
        Ok(staged)
    }
}

/// Bound both line length and the number of queued lines, draining oversized lines completely.
fn read_lines(
    reader: impl Read + Send + 'static,
    send: mpsc::SyncSender<String>,
) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("sounddeck-tool-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut line = Vec::with_capacity(LINE_BYTES);
            while let Ok(buffer) = reader.fill_buf() {
                if buffer.is_empty() {
                    if !line.is_empty() {
                        let _ = send.send(String::from_utf8_lossy(&line).into_owned());
                    }
                    break;
                }
                let end = buffer.iter().position(|&byte| byte == b'\n');
                let count = end.map_or(buffer.len(), |index| index + 1);
                line.extend_from_slice(&buffer[..count.min(LINE_BYTES.saturating_sub(line.len()))]);
                reader.consume(count);
                if end.is_some() {
                    let value = String::from_utf8_lossy(&line)
                        .trim_end_matches(['\n', '\r'])
                        .to_owned();
                    if send.send(value).is_err() {
                        break;
                    }
                    line.clear();
                }
            }
        })
        .ok()
}

pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    let path = crate::updates::tools::search_path()?;
    std::env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| {
            let candidate = directory.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            });
            candidate.is_file().then_some(candidate)
        })
}

pub(crate) fn detect(configured: Option<PathBuf>, cancel: &AtomicBool) -> DownloaderInfo {
    let mut info = DownloaderInfo {
        configured_path: configured.as_ref().map(|path| path.display().to_string()),
        ffmpeg: crate::updates::tools::resolve_ffmpeg().is_some(),
        ..Default::default()
    };
    let Some((path, _)) = crate::updates::tools::resolve_downloader(configured.as_deref()) else {
        return info;
    };
    if let Some(version) = probe_version(
        &path,
        &["--ignore-config", "--no-plugin-dirs", "--version"],
        cancel,
    ) {
        info.available = true;
        info.path = Some(path.canonicalize().unwrap_or(path).display().to_string());
        info.version = Some(version);
    }
    info
}

pub(crate) fn probe_version(path: &Path, args: &[&str], cancel: &AtomicBool) -> Option<String> {
    let mut command = Command::new(path);
    command.args(args);
    let Ok(mut process) = Process::spawn(command) else {
        return None;
    };
    let stdout = process.child.stdout.take()?;
    let stderr = process.child.stderr.take()?;
    let (send, lines) = mpsc::sync_channel(32);
    let readers = [read_lines(stdout, send.clone()), read_lines(stderr, send)];
    let started = Instant::now();
    let mut version = None;
    let mut status = None;
    let mut disconnected = false;
    while !cancel.load(Ordering::Relaxed) && started.elapsed() < DETECT_TIMEOUT {
        match lines.recv_timeout(POLL) {
            Ok(line) => {
                if version.is_none() && version_line(&line) {
                    version = Some(
                        line.chars()
                            .filter(|ch| !ch.is_control())
                            .take(100)
                            .collect::<String>(),
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = process.child.try_wait().ok().flatten();
        }
        if disconnected && status.is_some() {
            break;
        }
        if disconnected {
            std::thread::sleep(POLL);
        }
    }
    drop(process);
    // Drain after process exit too: a short-lived tool may exit before its reader runs.
    for line in lines.try_iter() {
        if version.is_none() && version_line(&line) {
            version = Some(
                line.chars()
                    .filter(|ch| !ch.is_control())
                    .take(100)
                    .collect(),
            );
        }
    }
    drop(lines);
    for reader in readers.into_iter().flatten() {
        let _ = reader.join();
    }
    version.filter(|_| status.is_some_and(|status| status.success()))
}

fn version_line(line: &str) -> bool {
    line.chars().next().is_some_and(|ch| ch.is_ascii_digit()) || line.starts_with("ffmpeg version ")
}

/// Owns a dedicated process tree. Windows starts suspended so even a fast launcher cannot
/// create ffmpeg before being assigned to the kill-on-close Job Object.
struct Process {
    child: Child,
    #[cfg(windows)]
    job: windows::Win32::Foundation::HANDLE,
}

impl Process {
    fn spawn(mut command: Command) -> anyhow::Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::{io::AsRawHandle, process::CommandExt};
            use windows::Win32::{
                Foundation::HANDLE,
                System::{JobObjects::*, Threading::*},
            };
            command.creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0);
            let child = command.spawn()?;
            let mut process = Self {
                child,
                job: HANDLE::default(),
            };
            // SAFETY: owned handles and a correctly-sized limits structure remain live for
            // these synchronous calls. Drop kills/reaps the suspended child on any error.
            unsafe {
                process.job = CreateJobObjectW(None, None)?;
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    process.job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    std::mem::size_of_val(&limits) as u32,
                )?;
                let handle = HANDLE(process.child.as_raw_handle());
                AssignProcessToJobObject(process.job, handle)?;
                #[link(name = "ntdll")]
                unsafe extern "system" {
                    fn NtResumeProcess(process: HANDLE) -> i32;
                }
                anyhow::ensure!(NtResumeProcess(handle) >= 0, "cannot resume yt-dlp process");
            }
            Ok(process)
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child: command.spawn()?,
            })
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.job.is_invalid() {
            // SAFETY: the job handle is owned; termination kills all descendants, including
            // ffmpeg, before its handle is closed exactly once.
            unsafe {
                let _ = windows::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
                let _ = windows::Win32::Foundation::CloseHandle(self.job);
            }
        }
        #[cfg(unix)]
        {
            // SAFETY: the child was spawned in its own process group; a negative pid targets
            // only that owned group, including ffmpeg. No shell/system-wide process lookup.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
#[path = "sounddeck_download_tests.rs"]
mod tests;
