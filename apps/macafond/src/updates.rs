//! GitHub release operations. Workers own all network, filesystem and subprocess work.
pub(crate) mod tools;
#[cfg(windows)]
mod windows;

use anyhow::{Context, Result, bail, ensure};
use macafon_ipc::{
    ComponentUpdate, Outcome, ToolSource, UpdateComponent as Component, UpdateStage as Stage,
    UpdatesState,
};
use serde::Deserialize;
#[cfg(any(windows, test))]
use sha2::{Digest, Sha256};
#[cfg(any(windows, test))]
use std::fs::{self, File};
use std::io::Read;
#[cfg(windows)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::JoinHandle;
#[cfg(windows)]
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const COMPONENTS: [Component; 3] = [Component::App, Component::YtDlp, Component::Ffmpeg];
const DAY_SECONDS: u64 = 24 * 60 * 60;
const METADATA_LIMIT: u64 = 2 * 1024 * 1024;

fn index(component: Component) -> usize {
    match component {
        Component::App => 0,
        Component::YtDlp => 1,
        Component::Ffmpeg => 2,
    }
}
fn repository(component: Component) -> &'static str {
    match component {
        Component::App => "Mar1kLab/Macafon",
        Component::YtDlp => "yt-dlp/yt-dlp",
        Component::Ffmpeg => "yt-dlp/FFmpeg-Builds",
    }
}
fn item(state: &mut UpdatesState, component: Component) -> &mut ComponentUpdate {
    if component == Component::App {
        &mut state.app
    } else {
        &mut state.tools[index(component) - 1]
    }
}
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn version(value: &str) -> Option<[u64; 3]> {
    let parts = value
        .strip_prefix('v')
        .unwrap_or(value)
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<u64>, _>>()
        .ok()?;
    parts.try_into().ok()
}
fn newer(component: Component, installed: Option<&str>, latest: &str) -> bool {
    if component == Component::App {
        matches!((installed.and_then(version), version(latest)), (Some(old), Some(new)) if new > old)
    } else {
        installed != Some(latest)
    }
}
fn availability(value: &mut ComponentUpdate) {
    value.update_available = value
        .latest_version
        .as_deref()
        .is_some_and(|latest| newer(value.component, value.installed_version.as_deref(), latest));
}

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    #[cfg(windows)]
    browser_download_url: String,
    #[cfg(windows)]
    digest: Option<String>,
}
#[derive(Clone, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
impl Release {
    fn asset(&self, name: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .context("release asset is missing")
    }
    fn download_asset(&self, component: Component) -> Result<&Asset> {
        let name = match component {
            Component::App => {
                ensure!(
                    version(&self.tag_name).is_some(),
                    "invalid app release version"
                );
                format!(
                    "Macafon_{}_x64-setup.exe",
                    self.tag_name.trim_start_matches('v')
                )
            }
            Component::YtDlp => "yt-dlp.exe".into(),
            Component::Ffmpeg => "ffmpeg-master-latest-win64-gpl.zip".into(),
        };
        self.asset(&name)
    }
}

#[derive(Clone)]
struct Ready {
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    sha256: String,
    tag: String,
}

fn describe_release(value: &mut ComponentUpdate, release: &Release) {
    value.latest_version = Some(if value.component == Component::App {
        release.tag_name.trim_start_matches('v').to_owned()
    } else {
        release.tag_name.clone()
    });
    value.release_url = Some(release.html_url.clone());
    value.download_bytes = release
        .download_asset(value.component)
        .ok()
        .map(|asset| asset.size);
    availability(value);
}
pub enum Action {
    Startup,
    Refresh,
    Check,
    Install(Component),
    Remove(Component),
    Open(Component),
}
impl Action {
    fn component(&self) -> Option<Component> {
        match self {
            Self::Install(c) | Self::Remove(c) | Self::Open(c) => Some(*c),
            _ => None,
        }
    }
}
struct Completion {
    releases: [Option<Release>; 3],
    ready: Option<Ready>,
    quit: bool,
    tools_changed: bool,
    error: Option<String>,
}
enum Message {
    State(UpdatesState),
    Complete(Box<Completion>),
}
struct Worker {
    receiver: mpsc::Receiver<Message>,
    flags: [Arc<AtomicBool>; 3],
    thread: Option<JoinHandle<()>>,
    reply: Option<mpsc::Sender<Outcome>>,
    tools_busy: bool,
}
impl Drop for Worker {
    fn drop(&mut self) {
        for flag in &self.flags {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub struct Manager {
    pub state: UpdatesState,
    releases: [Option<Release>; 3],
    ready: Option<Ready>,
    worker: Option<Worker>,
    refresh_requested: bool,
}
#[derive(Default)]
pub struct Poll {
    pub states: Vec<UpdatesState>,
    pub tools_changed: bool,
    pub quit: bool,
}

impl Manager {
    pub fn new(config: &macafon_config::UpdatesConfig) -> Self {
        let component = |component| ComponentUpdate {
            component,
            installable: cfg!(windows),
            source: if component == Component::App {
                ToolSource::Bundled
            } else {
                ToolSource::Missing
            },
            path: None,
            installed_version: (component == Component::App)
                .then(|| env!("CARGO_PKG_VERSION").to_owned()),
            latest_version: None,
            update_available: false,
            release_url: None,
            download_bytes: None,
            stage: Stage::Idle,
            progress_pct: None,
            error: None,
        };
        Self {
            state: UpdatesState {
                check_automatically: config.check_automatically,
                last_checked_unix: config.last_checked_unix,
                app: component(Component::App),
                tools: vec![component(Component::YtDlp), component(Component::Ffmpeg)],
            },
            releases: [None, None, None],
            ready: None,
            worker: None,
            refresh_requested: false,
        }
    }
    pub fn busy_tools(&self) -> bool {
        self.worker.as_ref().is_some_and(|worker| worker.tools_busy)
    }
    pub fn refresh(&mut self) {
        self.refresh_requested = true;
    }
    pub fn automatic_due(&self) -> bool {
        self.worker.is_none()
            && self.state.check_automatically
            && self
                .state
                .last_checked_unix
                .is_none_or(|last| unix_now().saturating_sub(last) >= DAY_SECONDS)
    }
    pub fn cancel(&mut self, component: Component) {
        if let Some(worker) = &self.worker {
            worker.flags[index(component)].store(true, Ordering::Relaxed);
        }
    }
    pub fn shutdown(&mut self) {
        self.worker.take();
    }
    pub fn start(
        &mut self,
        action: Action,
        configured: Option<PathBuf>,
        reply: Option<mpsc::Sender<Outcome>>,
    ) -> Result<()> {
        ensure!(self.worker.is_none(), "another update operation is running");
        if matches!(action, Action::Install(_) | Action::Remove(_)) {
            ensure!(cfg!(windows), "use your package manager to install updates");
        }
        if matches!(action, Action::Remove(Component::App)) {
            bail!("Macafon itself cannot be removed here");
        }
        if let Action::Install(component) = action {
            if component == Component::App {
                ensure!(self.releases[0].is_some(), "check for updates first");
                ensure!(
                    self.state.app.update_available,
                    "Macafon is already up to date"
                );
            }
        }
        let tools_busy = matches!(
            action,
            Action::Install(Component::YtDlp | Component::Ffmpeg) | Action::Remove(_)
        );
        let old_state = self.state.clone();
        if matches!(action, Action::Check) {
            for component in COMPONENTS {
                let value = item(&mut self.state, component);
                value.stage = Stage::Checking;
                value.error = None;
                value.progress_pct = None;
            }
        } else if let Some(component) = action.component() {
            let ready = component == Component::App
                && self.ready.as_ref().is_some_and(|ready| {
                    self.releases[0]
                        .as_ref()
                        .is_some_and(|release| release.tag_name == ready.tag)
                });
            let value = item(&mut self.state, component);
            value.error = None;
            value.progress_pct = None;
            value.stage = match action {
                Action::Open(_) => value.stage,
                Action::Install(_) if !ready => Stage::Downloading,
                _ => Stage::Installing,
            };
        }
        let mut session = Session {
            state: self.state.clone(),
            releases: self.releases.clone(),
            ready: self.ready.clone(),
            quit: false,
            tools_changed: false,
        };
        let flags = std::array::from_fn(|_| Arc::new(AtomicBool::new(false)));
        let worker_flags = flags.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("updates".into())
            .spawn(move || {
                let result = session.run(&action, configured.as_deref(), &sender, &worker_flags);
                let error = result.err().map(|error| short_error(&error));
                if let Some(reason) = &error {
                    tracing::warn!(%reason, "Update operation failed");
                }
                if let Some(error) = &error {
                    if let Some(component) = action.component() {
                        let value = item(&mut session.state, component);
                        value.stage = Stage::Failed;
                        value.error = Some(error.clone());
                        value.progress_pct = None;
                    } else {
                        for component in COMPONENTS {
                            let value = item(&mut session.state, component);
                            if value.stage == Stage::Checking {
                                value.stage = Stage::Failed;
                                value.error = Some(error.clone());
                            }
                        }
                    }
                }
                let _ = sender.send(Message::State(session.state));
                let _ = sender.send(Message::Complete(Box::new(Completion {
                    releases: session.releases,
                    ready: session.ready,
                    quit: session.quit,
                    tools_changed: session.tools_changed,
                    error,
                })));
            });
        let thread = match spawned {
            Ok(thread) => thread,
            Err(error) => {
                self.state = old_state;
                return Err(error.into());
            }
        };
        self.worker = Some(Worker {
            receiver,
            flags,
            thread: Some(thread),
            reply,
            tools_busy,
        });
        Ok(())
    }
    pub fn poll(&mut self, configured: Option<PathBuf>) -> Poll {
        let mut result = Poll::default();
        let messages: Vec<_> = self
            .worker
            .as_ref()
            .map(|worker| worker.receiver.try_iter().collect())
            .unwrap_or_default();
        for message in messages {
            match message {
                Message::State(mut state) => {
                    state.check_automatically = self.state.check_automatically;
                    if self.state != state {
                        self.state = state;
                        result.states.push(self.state.clone());
                    }
                }
                Message::Complete(done) => {
                    self.releases = done.releases;
                    self.ready = done.ready;
                    result.quit |= done.quit;
                    result.tools_changed |= done.tools_changed;
                    if let Some(mut worker) = self.worker.take()
                        && let Some(reply) = worker.reply.take()
                    {
                        let outcome = if let Some(error) = done.error {
                            Outcome::Error(macafon_ipc::ErrorBody::new(
                                macafon_ipc::ErrorCode::Internal,
                                error,
                            ))
                        } else {
                            Outcome::Result(serde_json::Value::Null)
                        };
                        let _ = reply.send(outcome);
                    }
                }
            }
        }
        if self.worker.is_none() && self.refresh_requested {
            self.refresh_requested = false;
            if let Err(error) = self.start(Action::Refresh, configured, None) {
                tracing::warn!(reason = %short_error(&error), "Cannot refresh update tools");
            }
        }
        result
    }
}

struct Session {
    state: UpdatesState,
    releases: [Option<Release>; 3],
    ready: Option<Ready>,
    quit: bool,
    tools_changed: bool,
}
impl Session {
    fn publish(&self, sender: &mpsc::Sender<Message>) {
        let _ = sender.send(Message::State(self.state.clone()));
    }
    fn run(
        &mut self,
        action: &Action,
        configured: Option<&Path>,
        sender: &mpsc::Sender<Message>,
        flags: &[Arc<AtomicBool>; 3],
    ) -> Result<()> {
        match *action {
            Action::Startup | Action::Refresh => {
                #[cfg(windows)]
                if matches!(action, Action::Startup) {
                    windows::cleanup_installers()?;
                }
                tools::scan(&mut self.state, configured, &flags[0]);
            }
            Action::Check => {
                tools::scan(&mut self.state, configured, &flags[0]);
                let client = client()?;
                for component in COMPONENTS {
                    let result = check_release(&client, component, &flags[index(component)]);
                    let value = item(&mut self.state, component);
                    match result {
                        Ok(release) => {
                            describe_release(value, &release);
                            value.stage = if component == Component::App
                                && self
                                    .ready
                                    .as_ref()
                                    .is_some_and(|ready| ready.tag == release.tag_name)
                            {
                                Stage::Ready
                            } else {
                                Stage::Idle
                            };
                            self.releases[index(component)] = Some(release);
                        }
                        Err(error) => {
                            value.stage = Stage::Failed;
                            value.error = Some(short_error(&error));
                        }
                    }
                    self.publish(sender);
                }
                self.state.last_checked_unix = Some(unix_now());
            }
            Action::Open(component) => {
                let url = self.releases[index(component)]
                    .as_ref()
                    .map(|release| release.html_url.clone())
                    .unwrap_or_else(|| {
                        format!(
                            "https://github.com/{}/releases/latest",
                            repository(component)
                        )
                    });
                validate_url(&url)?;
                crate::sounddeck::open_folder(Path::new(&url))?;
            }
            Action::Install(component) => {
                #[cfg(windows)]
                {
                    let cancel = &flags[index(component)];
                    let release = if let Some(release) = &self.releases[index(component)] {
                        release.clone()
                    } else {
                        // Install is already an explicit user action. Resolve a missing tool's
                        // release here so it does not require a separate Check now first.
                        let release = check_release(&client()?, component, cancel)?;
                        describe_release(item(&mut self.state, component), &release);
                        self.releases[index(component)] = Some(release.clone());
                        self.publish(sender);
                        release
                    };
                    if component == Component::App
                        && let Some(ready) = self
                            .ready
                            .clone()
                            .filter(|ready| ready.tag == release.tag_name)
                    {
                        if let Err(error) = verify_file(&ready.path, &ready.sha256, None, cancel) {
                            self.ready = None;
                            return Err(error);
                        }
                        windows::launch_installer(&ready.path)?;
                        self.quit = true;
                        return Ok(());
                    }
                    let client = client()?;
                    let directory = tools::root().context("LOCALAPPDATA is unavailable")?.join(
                        if component == Component::App {
                            "updates"
                        } else {
                            "tools"
                        },
                    );
                    windows::reject_links(&directory)?;
                    fs::create_dir_all(&directory).context("cannot create update directory")?;
                    let asset = release.download_asset(component)?;
                    validate_size(component, asset.size)?;
                    windows::check_space(
                        &directory,
                        asset
                            .size
                            .saturating_mul(if component == Component::Ffmpeg { 5 } else { 2 }),
                    )?;
                    let part = directory.join(format!("{}.part", asset.name));
                    let downloaded: Result<()> = (|| {
                        let expected =
                            expected_checksum(&client, &release, component, asset, cancel)?;
                        download(&client, asset, component, &part, cancel, |pct| {
                            item(&mut self.state, component).progress_pct = Some(pct);
                            self.publish(sender);
                        })?;
                        item(&mut self.state, component).stage = Stage::Verifying;
                        self.publish(sender);
                        verify_file(&part, &expected, asset.digest.as_deref(), cancel)?;
                        cancelled(cancel)?;
                        if component == Component::App {
                            let path = directory.join(&asset.name);
                            windows::replace_file(&part, &path)?;
                            self.ready = Some(Ready {
                                path,
                                sha256: expected,
                                tag: release.tag_name.clone(),
                            });
                            item(&mut self.state, component).stage = Stage::Ready;
                        } else {
                            item(&mut self.state, component).stage = Stage::Installing;
                            self.publish(sender);
                            self.tools_changed = true;
                            windows::install_tool(component, &part, &release.tag_name, cancel)?;
                            tools::scan(&mut self.state, configured, cancel);
                            item(&mut self.state, component).stage = Stage::Idle;
                        }
                        item(&mut self.state, component).progress_pct = None;
                        Ok(())
                    })();
                    let _ = fs::remove_file(&part);
                    downloaded?;
                }
                #[cfg(not(windows))]
                {
                    let _ = component;
                    bail!("use your package manager to install updates");
                }
            }
            Action::Remove(component) => {
                #[cfg(windows)]
                {
                    self.tools_changed = true;
                    windows::remove_tool(component, &flags[index(component)])?;
                    tools::scan(&mut self.state, configured, &flags[index(component)]);
                    item(&mut self.state, component).stage = Stage::Idle;
                }
                #[cfg(not(windows))]
                {
                    let _ = component;
                    bail!("use your package manager to remove tools");
                }
            }
        }
        Ok(())
    }
}

fn short_error(error: &anyhow::Error) -> String {
    // Never expose reqwest's signed redirect URLs (including query tokens) to logs or clients.
    if error.chain().any(|cause| cause.is::<reqwest::Error>()) {
        "GitHub request failed; check your connection and try again".into()
    } else {
        error
            .to_string()
            .chars()
            .filter(|ch| !ch.is_control())
            .take(180)
            .collect()
    }
}
fn allowed_host(host: &str) -> bool {
    matches!(
        host,
        "github.com"
            | "api.github.com"
            | "objects.githubusercontent.com"
            | "release-assets.githubusercontent.com"
            | "codeload.github.com"
    )
}
fn validate_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).context("invalid GitHub URL")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some_and(allowed_host)
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443),
        "URL outside the allowed GitHub hosts refused"
    );
    Ok(())
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("Macafon/", env!("CARGO_PKG_VERSION")))
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
            );
            headers
        })
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if validate_url(attempt.url().as_str()).is_ok() && attempt.previous().len() < 6 {
                attempt.follow()
            } else {
                attempt.error("redirect outside the allowed GitHub hosts refused")
            }
        }))
        .build()?)
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "update cancelled");
    Ok(())
}
fn read_metadata(
    client: &reqwest::blocking::Client,
    url: &str,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    cancelled(cancel)?;
    validate_url(url)?;
    let mut response = client.get(url).send()?.error_for_status()?;
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= METADATA_LIMIT),
        "release metadata is too large"
    );
    let mut bytes = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        cancelled(cancel)?;
        let size = response
            .read(&mut buffer)
            .context("cannot read release metadata")?;
        if size == 0 {
            break;
        }
        ensure!(
            bytes.len() as u64 + size as u64 <= METADATA_LIMIT,
            "release metadata is too large"
        );
        bytes.extend_from_slice(&buffer[..size]);
    }
    Ok(bytes)
}
fn check_release(
    client: &reqwest::blocking::Client,
    component: Component,
    cancel: &AtomicBool,
) -> Result<Release> {
    let bytes = read_metadata(
        client,
        &format!(
            "https://api.github.com/repos/{}/releases/latest",
            repository(component)
        ),
        cancel,
    )?;
    let release: Release =
        serde_json::from_slice(&bytes).context("invalid GitHub release metadata")?;
    ensure!(
        !release.draft && !release.prerelease,
        "no stable release is available"
    );
    ensure!(
        !release.tag_name.is_empty() && release.tag_name.len() < 100,
        "invalid release tag"
    );
    validate_url(&release.html_url)?;
    if component == Component::App {
        ensure!(
            version(&release.tag_name).is_some(),
            "invalid app release version"
        );
    }
    Ok(release)
}
#[cfg(any(windows, test))]
fn cap(component: Component) -> u64 {
    (match component {
        Component::App => 200,
        Component::YtDlp => 50,
        Component::Ffmpeg => 300,
    }) * 1024
        * 1024
}
#[cfg(any(windows, test))]
fn validate_size(component: Component, bytes: u64) -> Result<()> {
    ensure!(
        bytes > 0 && bytes <= cap(component),
        "download exceeds the component size limit"
    );
    Ok(())
}
#[cfg(any(windows, test))]
fn checksum(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && valid_hash(hash))
            .then(|| hash.to_ascii_lowercase())
    })
}
#[cfg(any(windows, test))]
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|ch| ch.is_ascii_hexdigit())
}
#[cfg(windows)]
fn expected_checksum(
    client: &reqwest::blocking::Client,
    release: &Release,
    component: Component,
    asset: &Asset,
    cancel: &AtomicBool,
) -> Result<String> {
    let name = match component {
        Component::App => "SHA256SUMS.txt",
        Component::YtDlp => "SHA2-256SUMS",
        Component::Ffmpeg => "checksums.sha256",
    };
    let sums = release.assets.iter().find(|a| a.name == name).or_else(|| {
        (component == Component::Ffmpeg)
            .then(|| {
                release.assets.iter().find(|a| {
                    matches!(
                        a.name.as_str(),
                        "SHA256SUMS" | "SHA256SUMS.txt" | "sha256sums.txt"
                    )
                })
            })
            .flatten()
    });
    if let Some(sums) = sums {
        let bytes = read_metadata(client, &sums.browser_download_url, cancel)?;
        return checksum(
            std::str::from_utf8(&bytes).context("invalid checksum file")?,
            &asset.name,
        )
        .context("release checksum is missing");
    }
    if component == Component::Ffmpeg
        && let Some(hash) = asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .filter(|hash| valid_hash(hash))
    {
        return Ok(hash.to_ascii_lowercase());
    }
    bail!("release checksum is missing")
}
#[cfg(any(windows, test))]
fn verify_file(
    path: &Path,
    expected: &str,
    digest: Option<&str>,
    cancel: &AtomicBool,
) -> Result<()> {
    let result = (|| {
        ensure!(valid_hash(expected), "release checksum is missing");
        let mut file = File::open(path).context("cannot open downloaded file")?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            cancelled(cancel)?;
            let size = file
                .read(&mut buffer)
                .context("cannot verify downloaded file")?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        let actual = format!("{:x}", hash.finalize());
        ensure!(
            actual.eq_ignore_ascii_case(expected),
            "SHA-256 checksum mismatch"
        );
        if let Some(digest) = digest {
            let hash = digest
                .strip_prefix("sha256:")
                .context("unsupported GitHub asset digest")?;
            ensure!(
                valid_hash(hash) && actual.eq_ignore_ascii_case(hash),
                "GitHub asset digest mismatch"
            );
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}
#[cfg(windows)]
fn download(
    client: &reqwest::blocking::Client,
    asset: &Asset,
    component: Component,
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    validate_size(component, asset.size)?;
    validate_url(&asset.browser_download_url)?;
    cancelled(cancel)?;
    let mut response = client
        .get(&asset.browser_download_url)
        .send()?
        .error_for_status()?;
    if let Some(length) = response.content_length() {
        validate_size(component, length)?;
        ensure!(
            length == asset.size,
            "download size differs from release metadata"
        );
    }
    windows::reject_links(path)?;
    let mut file = File::create(path).context("cannot create update file")?;
    let mut buffer = [0; 65536];
    let mut received = 0u64;
    let mut at = Instant::now();
    loop {
        cancelled(cancel)?;
        let size = response.read(&mut buffer).context("download interrupted")?;
        if size == 0 {
            break;
        }
        received += size as u64;
        ensure!(
            received <= asset.size && received <= cap(component),
            "download exceeds the component size limit"
        );
        file.write_all(&buffer[..size])
            .context("cannot write update file")?;
        if at.elapsed() >= Duration::from_millis(100) {
            progress(received as f32 * 100.0 / asset.size as f32);
            at = Instant::now();
        }
    }
    ensure!(received == asset.size, "download is incomplete");
    file.sync_all().context("cannot flush update file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit manual smoke: never part of the network-free default test suite.
    #[cfg(windows)]
    #[test]
    #[ignore = "downloads the live GitHub yt-dlp release into a temporary directory"]
    fn github_check_and_yt_dlp_download_smoke() -> Result<()> {
        let client = client()?;
        let cancel = AtomicBool::new(false);
        let mut releases = Vec::new();
        for component in COMPONENTS {
            let release = check_release(&client, component, &cancel)?;
            println!("{component:?}: {}", release.tag_name);
            releases.push(release);
        }
        let release = &releases[1];
        let asset = release.download_asset(Component::YtDlp)?;
        let expected = expected_checksum(&client, release, Component::YtDlp, asset, &cancel)?;
        // Also exercise FFmpeg's live published checksum without fetching its large archive.
        let ffmpeg = releases[2].download_asset(Component::Ffmpeg)?;
        let ffmpeg_hash =
            expected_checksum(&client, &releases[2], Component::Ffmpeg, ffmpeg, &cancel)?;
        ensure!(
            ffmpeg.digest.as_deref() == Some(format!("sha256:{ffmpeg_hash}").as_str()),
            "ffmpeg published checksum differs from asset digest"
        );
        let directory =
            std::env::temp_dir().join(format!("macafon-update-smoke-{}", std::process::id()));
        fs::create_dir_all(&directory)?;
        let result = (|| {
            windows::check_space(&directory, asset.size * 2)?;
            let part = directory.join("yt-dlp.exe.part");
            download(&client, asset, Component::YtDlp, &part, &cancel, |_| {})?;
            verify_file(&part, &expected, asset.digest.as_deref(), &cancel)?;
            let executable = directory.join("yt-dlp.exe");
            windows::replace_file(&part, &executable)?;
            windows::install_tool_in(
                &directory.join("tools"),
                Component::YtDlp,
                &executable,
                &release.tag_name,
                &cancel,
            )?;
            let executable = directory.join("tools/yt-dlp/yt-dlp.exe");
            let version = crate::sounddeck::download_worker::probe_version(
                &executable,
                &["--ignore-config", "--no-plugin-dirs", "--version"],
                &cancel,
            )
            .context("downloaded yt-dlp did not run")?;
            ensure!(
                version == release.tag_name,
                "yt-dlp version differs from release tag"
            );
            println!(
                "verified and ran yt-dlp {version} ({} bytes); ffmpeg checksum verified",
                asset.size
            );
            Ok(())
        })();
        let _ = fs::remove_dir_all(directory);
        result
    }
    #[test]
    fn update_versions_tags_checksums_and_host_policy() {
        let release: Release = serde_json::from_value(serde_json::json!({
            "tag_name": "v1.2.3", "html_url": "https://github.com/Mar1kLab/Macafon/releases/tag/v1.2.3",
            "draft": false, "prerelease": false, "assets": []
        })).expect("release fixture");
        let mut manager = Manager::new(&macafon_config::UpdatesConfig::default());
        describe_release(&mut manager.state.app, &release);
        assert_eq!(manager.state.app.latest_version.as_deref(), Some("1.2.3"));
        assert!(newer(Component::App, Some("0.9.9"), "v0.10.0"));
        assert!(!newer(Component::App, Some("1.2.3"), "v1.2.3"));
        assert!(!newer(Component::App, Some("1.2.3"), "v1.2.4-rc1"));
        assert!(!newer(Component::YtDlp, Some("2026.10.26"), "2026.10.26"));
        assert!(newer(Component::Ffmpeg, Some("old"), "latest"));
        let hash = "a".repeat(64);
        assert_eq!(
            checksum(&format!("{hash}  yt-dlp.exe\n"), "yt-dlp.exe"),
            Some(hash)
        );
        assert_eq!(checksum("bad  yt-dlp.exe", "yt-dlp.exe"), None);
        for url in [
            "https://github.com/x",
            "https://api.github.com/x",
            "https://objects.githubusercontent.com/x",
            "https://release-assets.githubusercontent.com/x?q=token",
            "https://codeload.github.com/x",
        ] {
            assert!(validate_url(url).is_ok());
        }
        for url in [
            "http://github.com/x",
            "https://github.com.attacker.test/x",
            "https://evilgithub.com/x",
            "https://user@github.com/x",
            "https://github.com:444/x",
        ] {
            assert!(validate_url(url).is_err());
        }
        for component in COMPONENTS {
            assert!(validate_size(component, cap(component)).is_ok());
            assert!(validate_size(component, cap(component) + 1).is_err());
        }
    }
    #[test]
    fn checksum_mismatch_and_missing_checksum_delete_download() {
        let path =
            std::env::temp_dir().join(format!("macafon-update-checksum-{}", std::process::id()));
        let cancel = AtomicBool::new(false);
        for expected in ["0".repeat(64), "missing".into()] {
            fs::write(&path, b"file").expect("write fixture");
            assert!(verify_file(&path, &expected, None, &cancel).is_err());
            assert!(!path.exists());
        }
        fs::write(&path, b"file").expect("write fixture");
        let hash = format!("{:x}", Sha256::digest(b"file"));
        assert!(
            verify_file(
                &path,
                &hash,
                Some(&format!("sha256:{}", "0".repeat(64))),
                &cancel
            )
            .is_err()
        );
        assert!(!path.exists());
    }
    #[test]
    fn automatic_checks_are_opt_in_and_daily() {
        let mut manager = Manager::new(&macafon_config::UpdatesConfig::default());
        assert!(!manager.automatic_due());
        manager.state.check_automatically = true;
        assert!(manager.automatic_due());
        manager.state.last_checked_unix = Some(unix_now());
        assert!(!manager.automatic_due());
        manager.state.last_checked_unix = Some(unix_now() - DAY_SECONDS);
        assert!(manager.automatic_due());
    }
}
