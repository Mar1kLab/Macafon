//! Windows application volume groups. All COM ownership and disk I/O stay on one MTA
//! worker. Callers need no COM apartment. Commands are synchronous; events contain no COM
//! interfaces. One watcher must own a state directory at a time.
use crate::devices::{device_id, enumerator};
use crate::mixer_journal::{Journal, Key, SessionStore, StoredSession, Value};
use crate::{ComGuard, Error};
use macafon_dsp::channel::volume_gain;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{PROPERTYKEY, S_OK};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::core::{BOOL, GUID, Interface, PCWSTR, Ref, implement};

/// Stable pseudo executable for the Windows system sounds session.
pub const SYSTEM_SOUNDS: &str = "<system-sounds>";
const CONTEXT: GUID = GUID::from_u128(0xac36e42c_763e_4420_976c_6be5be43d672);
const VOLUME_PENDING: u64 = 1 << 33;

/// Faders in percent (0..100), combined before the shared cubic curve. Mutes are ORed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixSettings {
    pub channel_pct: f32,
    pub app_pct: f32,
    pub master_pct: f32,
    pub channel_muted: bool,
    pub app_muted: bool,
    pub master_muted: bool,
}
impl Default for MixSettings {
    fn default() -> Self {
        Self {
            channel_pct: 100.0,
            app_pct: 100.0,
            master_pct: 100.0,
            channel_muted: false,
            app_muted: false,
            master_muted: false,
        }
    }
}

fn pct(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// Effective Windows session scalar, using the same curve as Linux channel DSP.
pub fn session_volume(channel_pct: f32, app_pct: f32, master_pct: f32) -> f32 {
    volume_gain(pct(channel_pct) * (pct(app_pct) / 100.0) * (pct(master_pct) / 100.0))
}

/// Recover an application fader from a foreign session scalar. At a zero channel/Master
/// the app value is unobservable: preserve its last value instead of dividing by zero.
pub fn app_volume(observed: f32, channel_pct: f32, master_pct: f32, previous_pct: f32) -> f32 {
    let denominator = (pct(channel_pct) / 100.0) * (pct(master_pct) / 100.0);
    if denominator == 0.0 || !observed.is_finite() {
        return pct(previous_pct);
    }
    (observed.clamp(0.0, 1.0).cbrt() / denominator * 100.0).clamp(0.0, 100.0)
}

#[derive(Debug, Clone)]
pub struct App {
    /// Lowercase executable file name, or [`SYSTEM_SOUNDS`]. Unknown protected processes
    /// use a PID-scoped pseudo key, never one shared "unknown application" group.
    pub exe: String,
    /// Full executable path used to resolve the application icon.
    pub executable: Option<String>,
    pub session_count: usize,
    pub active: bool,
    pub app_pct: f32,
    pub muted: bool,
    /// Maximum session peak across endpoints (linear 0..1).
    pub peak: f32,
}

#[derive(Debug, Clone)]
pub struct OutputPeak {
    pub endpoint_id: String,
    pub peak: f32,
}

#[derive(Debug, Clone)]
pub enum MixerEvent {
    /// Topology, state, display metadata or session lifetime changed; request `apps()`.
    AppsChanged,
    /// Foreign changes are accepted without writing back to the changed session.
    ExternalVolume {
        exe: String,
        app_pct: f32,
        muted: bool,
    },
    Error(String),
    /// Recovery records retained for absent apps/endpoints or ambiguous new session IDs.
    /// They are retried on the next start or explicit `restore()`.
    RecoveryPending {
        sessions: usize,
    },
}

enum Command {
    Apps(Sender<Result<Vec<App>, Error>>),
    Peaks(Sender<Result<Vec<OutputPeak>, Error>>),
    Set(String, MixSettings, Sender<Result<(), Error>>),
    RestoreApp(String, Sender<Result<(), Error>>),
    Restore(Sender<Result<(), Error>>),
    Stop,
}

/// Owns a dedicated worker and its subscriptions. Drop joins the worker and attempts
/// conditional restoration; call `restore()` first to observe any restoration error.
/// A failed restore remains journaled for the next start. `restore()` also clears rules.
pub struct SessionWatcher {
    commands: Sender<Command>,
    events: Receiver<MixerEvent>,
    worker: Option<JoinHandle<()>>,
}

impl SessionWatcher {
    /// Discover, subscribe, and recover an earlier crash before accepting commands.
    pub fn start(state_dir: &Path) -> Result<Self, Error> {
        let state_dir = state_dir.to_owned();
        let (commands, receiver) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let (ready_tx, ready) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("macafon-mixer".into())
            .spawn(move || {
                let result = (|| {
                    let _com = ComGuard::new()?;
                    let mut state = Worker::new(events_tx.clone())?;
                    let mut journal = Journal::open(&state_dir)?;
                    state.refresh()?;
                    journal.restore(&mut state)?;
                    state.report_pending(&journal);
                    let _ = ready_tx.send(Ok(()));
                    state.run(receiver, &mut journal);
                    Ok::<_, Error>(())
                })();
                if let Err(error) = result {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|e| Error::Thread(e.to_string()))?;
        match ready.recv().map_err(|e| Error::Thread(e.to_string()))? {
            Ok(()) => Ok(Self {
                commands,
                events,
                worker: Some(worker),
            }),
            Err(e) => {
                let _ = worker.join();
                Err(e)
            }
        }
    }
    /// Drain notifications from the caller's normal control loop.
    pub fn try_event(&self) -> Option<MixerEvent> {
        self.events.try_recv().ok()
    }
    /// Current app groups, including freshly polled peaks. No automatic volume changes.
    pub fn apps(&self) -> Result<Vec<App>, Error> {
        self.request(Command::Apps)
    }
    /// Device peak for every active render endpoint; callers choose their output meter.
    pub fn output_peaks(&self) -> Result<Vec<OutputPeak>, Error> {
        self.request(Command::Peaks)
    }
    /// Remember and apply all faders/mutes to every session of this executable, including
    /// future sessions. Group/channel membership remains the caller's responsibility.
    pub fn set_app(&self, exe: &str, settings: MixSettings) -> Result<(), Error> {
        self.request(|reply| Command::Set(exe.to_lowercase(), settings, reply))
    }
    /// Stop managing this executable (including future sessions), and conditionally
    /// restore its original volume/mute. Failed or absent sessions remain journaled.
    pub fn restore_app(&self, exe: &str) -> Result<(), Error> {
        self.request(|reply| Command::RestoreApp(exe.to_lowercase(), reply))
    }
    pub fn restore(&self) -> Result<(), Error> {
        self.request(Command::Restore)
    }
    fn request<T>(
        &self,
        command: impl FnOnce(Sender<Result<T, Error>>) -> Command,
    ) -> Result<T, Error> {
        let (tx, rx) = mpsc::channel();
        self.commands
            .send(command(tx))
            .map_err(|e| Error::Thread(e.to_string()))?;
        rx.recv().map_err(|e| Error::Thread(e.to_string()))?
    }
}
impl Drop for SessionWatcher {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct LiveSession {
    executable: Option<String>,
    key: Key,
    instance: String,
    control: IAudioSessionControl2,
    volume: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
    callback: IAudioSessionEvents,
    foreign: Arc<AtomicU64>,
}
impl Drop for LiveSession {
    fn drop(&mut self) {
        // SAFETY: subscription and control remain live until after this call.
        unsafe {
            let _ = self
                .control
                .UnregisterAudioSessionNotification(&self.callback);
        }
    }
}
struct Endpoint {
    id: String,
    manager: IAudioSessionManager2,
    callback: IAudioSessionNotification,
    meter: Option<IAudioMeterInformation>,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        // SAFETY: registered callback remains live during unregistration.
        unsafe {
            let _ = self.manager.UnregisterSessionNotification(&self.callback);
        }
    }
}

struct Worker {
    enumerator: IMMDeviceEnumerator,
    devices_callback: IMMNotificationClient,
    dirty: Arc<AtomicBool>,
    created: Arc<CreatedQueue>,
    endpoints: Vec<Endpoint>,
    sessions: Vec<LiveSession>,
    settings: HashMap<String, MixSettings>,
    /// New sessions whose settings are not applied yet; kept until a write succeeds.
    unapplied: Vec<String>,
    events: Sender<MixerEvent>,
}
impl Worker {
    fn new(events: Sender<MixerEvent>) -> Result<Self, Error> {
        let enumerator = enumerator()?;
        let dirty = Arc::new(AtomicBool::new(false));
        let devices_callback = DeviceEvents {
            dirty: dirty.clone(),
        }
        .into();
        // SAFETY: enumerator and callback are owned until explicit unregistration.
        unsafe {
            enumerator.RegisterEndpointNotificationCallback(&devices_callback)?;
        }
        Ok(Self {
            enumerator,
            devices_callback,
            dirty,
            created: Arc::new(CreatedQueue::default()),
            endpoints: Vec::new(),
            sessions: Vec::new(),
            settings: HashMap::new(),
            unapplied: Vec::new(),
            events,
        })
    }

    /// Track new endpoints and sessions; new sessions are queued in `unapplied` as soon as
    /// they are tracked, so a later failure here does not skip their settings.
    fn refresh(&mut self) -> Result<(), Error> {
        // SAFETY: all interfaces are created, called, and released on this MTA worker.
        unsafe {
            let devices = self
                .enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let mut ids = Vec::new();
            for index in 0..devices.GetCount()? {
                let device = devices.Item(index)?;
                let id = device_id(&device)?;
                ids.push(id.clone());
                if !self.endpoints.iter().any(|e| e.id == id) {
                    let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                    let callback = SessionCreated {
                        dirty: self.dirty.clone(),
                        created: self.created.clone(),
                        endpoint: Arc::from(id.as_str()),
                    }
                    .into();
                    manager.RegisterSessionNotification(&callback)?;
                    self.endpoints.push(Endpoint {
                        id: id.clone(),
                        manager,
                        callback,
                        meter: device.Activate(CLSCTX_ALL, None).ok(),
                    });
                }
            }
            self.sessions.retain(|s| ids.contains(&s.key.endpoint));
            self.endpoints.retain(|e| ids.contains(&e.id));
            let mut controls = Vec::new();
            for endpoint in &self.endpoints {
                let list = endpoint.manager.GetSessionEnumerator()?;
                // GetCount also enables creation notifications for this manager.
                for index in 0..list.GetCount()? {
                    controls.push((endpoint.id.clone(), list.GetSession(index)?));
                }
            }
            controls.extend(self.created.drain());
            for (endpoint, control) in controls {
                if !ids.contains(&endpoint) {
                    continue;
                }
                let control: IAudioSessionControl2 = control.cast()?;
                if control.GetState()? == AudioSessionStateExpired {
                    continue;
                }
                let raw = control.GetSessionInstanceIdentifier()?;
                let instance = raw.to_string();
                CoTaskMemFree(Some(raw.as_ptr().cast()));
                let instance = instance.map_err(|e| Error::Format(e.to_string()))?;
                if self.sessions.iter().any(|s| s.instance == instance) {
                    continue;
                }
                let pid = control.GetProcessId().unwrap_or(0);
                let system = control.IsSystemSoundsSession() == S_OK;
                let executable = (!system)
                    .then(|| crate::sessions::executable(pid))
                    .flatten();
                let exe = if system {
                    SYSTEM_SOUNDS.into()
                } else {
                    executable
                        .as_deref()
                        .and_then(|p| p.rsplit('\\').next().map(str::to_lowercase))
                        .unwrap_or_else(|| format!("<process-{pid}>"))
                };
                let foreign = Arc::new(AtomicU64::new(0));
                let callback = SessionEvents {
                    dirty: self.dirty.clone(),
                    foreign: foreign.clone(),
                }
                .into();
                let volume = control.cast()?;
                let meter = control.cast().ok();
                control.RegisterAudioSessionNotification(&callback)?;
                self.unapplied.push(instance.clone());
                self.sessions.push(LiveSession {
                    executable,
                    key: Key { exe, endpoint },
                    instance,
                    volume,
                    meter,
                    control,
                    callback,
                    foreign,
                });
            }
            self.sessions.retain(|s| {
                s.control
                    .GetState()
                    .is_ok_and(|state| state != AudioSessionStateExpired)
            });
        }
        Ok(())
    }

    fn external_changes(&mut self) {
        for session in &self.sessions {
            let bits = session.foreign.swap(0, Ordering::AcqRel);
            if bits & VOLUME_PENDING == 0 {
                continue;
            }
            let observed = f32::from_bits(bits as u32);
            let muted = bits & (1 << 32) != 0;
            let mut settings = self
                .settings
                .get(&session.key.exe)
                .copied()
                .unwrap_or_default();
            settings.app_pct = app_volume(
                observed,
                settings.channel_pct,
                settings.master_pct,
                settings.app_pct,
            );
            if !settings.channel_muted && !settings.master_muted {
                settings.app_muted = muted;
            }
            // Observing an unmanaged app must not install a rule for future sessions.
            if let Some(remembered) = self.settings.get_mut(&session.key.exe) {
                *remembered = settings;
            }
            let _ = self.events.send(MixerEvent::ExternalVolume {
                exe: session.key.exe.clone(),
                app_pct: settings.app_pct,
                muted,
            });
        }
    }

    fn apply(
        &mut self,
        instance: &str,
        settings: MixSettings,
        journal: &mut Journal,
    ) -> Result<(), Error> {
        let session = self
            .sessions
            .iter()
            .find(|s| s.instance == instance)
            .ok_or_else(|| Error::DeviceNotFound(instance.into()))?;
        // SAFETY: these are live session interfaces on their owning MTA worker.
        unsafe {
            let previous = Value {
                volume: session.volume.GetMasterVolume()?,
                muted: session.volume.GetMute()?.as_bool(),
            };
            let target = Value {
                volume: session_volume(settings.channel_pct, settings.app_pct, settings.master_pct),
                muted: settings.channel_muted || settings.app_muted || settings.master_muted,
            };
            if previous == target {
                return Ok(());
            }
            // Journal each property separately, so a partial COM failure remains recoverable.
            let mut recorded = StoredSession {
                key: session.key.clone(),
                instance: instance.into(),
                value: previous,
            };
            if previous.volume != target.volume {
                let intermediate = Value {
                    volume: target.volume,
                    ..previous
                };
                journal.record(&recorded, intermediate)?;
                session.volume.SetMasterVolume(target.volume, &CONTEXT)?;
                recorded.value = intermediate;
            }
            if previous.muted != target.muted {
                journal.record(&recorded, target)?;
                session.volume.SetMute(target.muted, &CONTEXT)?;
            }
        }
        Ok(())
    }

    /// Apply the remembered settings to new sessions; a failed one is retried on the next
    /// refresh instead of being taken for an existing, already handled session.
    fn apply_new(&mut self, journal: &mut Journal) -> Result<(), Error> {
        let mut first_error = None;
        for instance in std::mem::take(&mut self.unapplied) {
            let Some(settings) = self
                .sessions
                .iter()
                .find(|s| s.instance == instance)
                .and_then(|s| self.settings.get(&s.key.exe))
                .copied()
            else {
                continue; // Gone or unmanaged.
            };
            if let Err(error) = self.apply(&instance, settings, journal) {
                self.unapplied.push(instance);
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn apps(&self) -> Result<Vec<App>, Error> {
        let mut apps: HashMap<String, App> = HashMap::new();
        // SAFETY: live session interfaces; meter failures mean silence/disconnection.
        unsafe {
            for session in &self.sessions {
                let volume = session.volume.GetMasterVolume()?;
                let settings =
                    self.settings
                        .get(&session.key.exe)
                        .copied()
                        .unwrap_or(MixSettings {
                            app_pct: app_volume(volume, 100.0, 100.0, 100.0),
                            ..MixSettings::default()
                        });
                let app = apps.entry(session.key.exe.clone()).or_insert(App {
                    exe: session.key.exe.clone(),
                    executable: session.executable.clone(),
                    session_count: 0,
                    active: false,
                    app_pct: settings.app_pct,
                    muted: true,
                    peak: 0.0,
                });
                if app.executable.is_none() {
                    app.executable = session.executable.clone();
                }
                app.session_count += 1;
                app.active |= session.control.GetState()? == AudioSessionStateActive;
                app.muted &= session.volume.GetMute()?.as_bool();
                app.peak = app.peak.max(
                    session
                        .meter
                        .as_ref()
                        .and_then(|m| m.GetPeakValue().ok())
                        .unwrap_or(0.0),
                );
            }
        }
        let mut apps: Vec<_> = apps.into_values().collect();
        apps.sort_by(|a, b| a.exe.cmp(&b.exe));
        Ok(apps)
    }

    fn run(&mut self, commands: Receiver<Command>, journal: &mut Journal) {
        let mut refresh_after = Instant::now();
        loop {
            self.external_changes();
            if Instant::now() >= refresh_after && self.dirty.swap(false, Ordering::AcqRel) {
                let result = self.refresh().and_then(|()| self.apply_new(journal));
                if let Err(e) = result {
                    self.dirty.store(true, Ordering::Release);
                    refresh_after = Instant::now() + Duration::from_millis(500);
                    let _ = self.events.send(MixerEvent::Error(e.to_string()));
                }
                let _ = self.events.send(MixerEvent::AppsChanged);
            }
            match commands.recv_timeout(Duration::from_millis(20)) {
                Ok(Command::Apps(reply)) => {
                    self.external_changes();
                    let _ = reply.send(self.apps());
                }
                Ok(Command::Peaks(reply)) => {
                    // SAFETY: all meters belong to active endpoints on this worker.
                    let peaks = self
                        .endpoints
                        .iter()
                        .map(|e| OutputPeak {
                            endpoint_id: e.id.clone(),
                            peak: unsafe {
                                e.meter
                                    .as_ref()
                                    .and_then(|m| m.GetPeakValue().ok())
                                    .unwrap_or(0.0)
                            },
                        })
                        .collect();
                    let _ = reply.send(Ok(peaks));
                }
                Ok(Command::Set(exe, settings, reply)) => {
                    self.external_changes();
                    self.settings.insert(exe.clone(), settings);
                    let instances: Vec<_> = self
                        .sessions
                        .iter()
                        .filter(|s| s.key.exe == exe)
                        .map(|s| s.instance.clone())
                        .collect();
                    let mut result = Ok(());
                    for instance in instances {
                        if let Err(e) = self.apply(&instance, settings, journal) {
                            result = Err(e);
                        }
                    }
                    let _ = reply.send(result);
                }
                Ok(Command::RestoreApp(exe, reply)) => {
                    self.settings.remove(&exe);
                    let result = journal.restore_app(self, &exe);
                    self.report_pending(journal);
                    let _ = reply.send(result);
                }
                Ok(Command::Restore(reply)) => {
                    self.settings.clear();
                    let result = journal.restore(self);
                    self.report_pending(journal);
                    let _ = reply.send(result);
                }
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Err(e) = journal.restore(self) {
                        let _ = self.events.send(MixerEvent::Error(e.to_string()));
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn report_pending(&self, journal: &Journal) {
        if journal.pending() > 0 {
            let _ = self.events.send(MixerEvent::RecoveryPending {
                sessions: journal.pending(),
            });
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // SAFETY: callback stays live until unregistration completes; sessions/managers are
        // then dropped before the surrounding ComGuard uninitializes this thread.
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.devices_callback);
        }
    }
}
impl SessionStore for Worker {
    fn list(&self) -> Result<Vec<StoredSession>, Error> {
        self.sessions
            .iter()
            .map(|s| {
                // SAFETY: session volume is live on this worker.
                let value = unsafe {
                    Value {
                        volume: s.volume.GetMasterVolume()?,
                        muted: s.volume.GetMute()?.as_bool(),
                    }
                };
                Ok(StoredSession {
                    key: s.key.clone(),
                    instance: s.instance.clone(),
                    value,
                })
            })
            .collect()
    }
    fn set(&mut self, instance: &str, value: Value) -> Result<(), Error> {
        if let Some(session) = self.sessions.iter().find(|s| s.instance == instance) {
            // SAFETY: live volume interface; our GUID prevents feedback during recovery.
            unsafe {
                session.volume.SetMasterVolume(value.volume, &CONTEXT)?;
                session.volume.SetMute(value.muted, &CONTEXT)?;
            }
        }
        Ok(())
    }
}

// Creation notifications retain the supplied interface: enumeration can omit new sessions.
// MPSC atomic stack; both producer callbacks and the sole consumer belong to the COM MTA.
#[derive(Default)]
struct CreatedQueue {
    head: AtomicPtr<Created>,
}
struct Created {
    endpoint: Arc<str>,
    control: *mut std::ffi::c_void,
    next: *mut Created,
}
impl CreatedQueue {
    fn push(&self, endpoint: Arc<str>, control: IAudioSessionControl) {
        let node = Box::into_raw(Box::new(Created {
            endpoint,
            control: control.into_raw(),
            next: std::ptr::null_mut(),
        }));
        let mut head = self.head.load(Ordering::Relaxed);
        loop {
            // SAFETY: this producer owns node exclusively until the successful CAS.
            unsafe {
                (*node).next = head;
            }
            match self
                .head
                .compare_exchange_weak(head, node, Ordering::Release, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(current) => head = current,
            }
        }
    }
    fn drain(&self) -> Vec<(String, IAudioSessionControl)> {
        let mut node = self.head.swap(std::ptr::null_mut(), Ordering::Acquire);
        let mut result = Vec::new();
        while !node.is_null() {
            // SAFETY: swap detached this entire chain. Each retained MTA COM reference is
            // consumed exactly once, with no concurrent access to the detached nodes.
            unsafe {
                let entry = Box::from_raw(node);
                node = entry.next;
                result.push((
                    entry.endpoint.to_string(),
                    IAudioSessionControl::from_raw(entry.control),
                ));
            }
        }
        result.reverse();
        result
    }
}
impl Drop for CreatedQueue {
    fn drop(&mut self) {
        drop(self.drain());
    }
}

// Volume/state/device callbacks only publish atomics: no waits or disk I/O.
#[implement(IAudioSessionNotification)]
struct SessionCreated {
    dirty: Arc<AtomicBool>,
    created: Arc<CreatedQueue>,
    endpoint: Arc<str>,
}
impl IAudioSessionNotification_Impl for SessionCreated_Impl {
    fn OnSessionCreated(&self, session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        if let Some(control) = session.cloned() {
            self.created.push(self.endpoint.clone(), control);
        }
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessionEvents {
    dirty: Arc<AtomicBool>,
    foreign: Arc<AtomicU64>,
}
impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(
        &self,
        volume: f32,
        muted: BOOL,
        context: *const GUID,
    ) -> windows::core::Result<()> {
        // SAFETY: Windows supplies a valid event-context pointer for this callback or null.
        let ours = !context.is_null() && unsafe { *context == CONTEXT };
        if !ours {
            self.foreign.store(
                u64::from(volume.to_bits()) | (u64::from(muted.as_bool()) << 32) | VOLUME_PENDING,
                Ordering::Release,
            );
        }
        Ok(())
    }
    fn OnChannelVolumeChanged(
        &self,
        _: u32,
        _: *const f32,
        _: u32,
        _: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: AudioSessionState) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceEvents {
    dirty: Arc<AtomicBool>,
}
impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        _: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composition_uses_shared_cubic_curve() {
        assert!((session_volume(50.0, 80.0, 50.0) - 0.008).abs() < 1e-6);
        assert_eq!(session_volume(0.0, 100.0, 100.0), 0.0);
        assert_eq!(session_volume(f32::NAN, 100.0, 100.0), 0.0);
    }
    #[test]
    fn inversion_accepts_foreign_volume_and_handles_silent_groups() {
        assert!((app_volume(0.008, 50.0, 50.0, 100.0) - 80.0).abs() < 0.001);
        assert_eq!(app_volume(1.0, 50.0, 50.0, 70.0), 100.0);
        assert_eq!(app_volume(0.0, 0.0, 50.0, 70.0), 70.0);
    }
}
