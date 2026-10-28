//! Windows application volume groups. All COM ownership and disk I/O stay on one MTA
//! worker. Callers need no COM apartment. Commands are synchronous except `set_app`, which
//! is queued and coalesced so a held volume key never waits for COM or the disk; events
//! contain no COM interfaces. One watcher must own a state directory at a time.
use crate::devices::{device_id, enumerator};
use crate::lift::{self, Ramp, Saved, Store};
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
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::core::{BOOL, GUID, HSTRING, Interface, PCWSTR, Ref, implement};

/// Stable pseudo executable for the Windows system sounds session.
pub const SYSTEM_SOUNDS: &str = "<system-sounds>";
pub(crate) const SOUNDDECK: &str = "<sounddeck>";
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
    /// Queued faders for this app could not be written; the caller may send them again.
    SetFailed {
        exe: String,
        message: String,
    },
    /// A session of an app without settings got the new-app faders (`set_new_app`) the moment
    /// it appeared; the caller corrects or restores it once the app is classified.
    NewAppDefaulted {
        exe: String,
    },
    /// Recovery records retained for absent apps/endpoints or ambiguous new session IDs.
    /// They are retried on the next start or explicit `restore()`.
    RecoveryPending {
        sessions: usize,
    },
}

enum Command {
    Master(
        Option<(f32, bool)>,
        Sender<Result<(String, f32, bool), Error>>,
    ),
    Apps(Sender<Result<Vec<App>, Error>>),
    Peaks(Sender<Result<Vec<OutputPeak>, Error>>),
    Set(String, MixSettings),
    NewApp(Option<MixSettings>, Sender<Result<(), Error>>),
    RestoreApp(String, Sender<Result<(), Error>>),
    Restore(Sender<Result<(), Error>>),
    /// Stop without giving anything back: logoff/shutdown.
    Keep(Sender<Result<(), Error>>),
    /// Manage the system volume (WIN-CH-013) or mirror it.
    Manage(bool, Sender<Result<(), Error>>),
    /// A callback published a change; the loop picks it up from `Signal`.
    Wake,
    Stop,
}

/// What COM callbacks share with the worker: flags, and a wake-up so the loop sleeps until
/// something happens instead of polling them. One queued wake-up at a time.
struct Signal {
    dirty: AtomicBool,
    queued: AtomicBool,
    commands: Sender<Command>,
}
impl Signal {
    fn changed(&self) {
        self.dirty.store(true, Ordering::Release);
        self.wake();
    }
    fn wake(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            let _ = self.commands.send(Command::Wake);
        }
    }
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
        let wake = commands.clone();
        let worker = std::thread::Builder::new()
            .name("macafon-mixer".into())
            .spawn(move || {
                let result = (|| {
                    let _com = ComGuard::new()?;
                    let mut state = Worker::new(events_tx.clone(), wake, &state_dir)?;
                    let mut journal = Journal::open(&state_dir)?;
                    state.refresh()?;
                    let saved = state.store.load().unwrap_or_else(|error| {
                        let _ = state.events.send(MixerEvent::Error(error.to_string()));
                        Vec::new()
                    });
                    // Kept at logoff/shutdown: the sessions still play at what Macafon set,
                    // and the mixer takes them over again instead of jumping to the originals.
                    if !state.can_take_over(&saved) {
                        journal.restore(&mut state)?;
                    }
                    state.recover_levels(saved);
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
    pub fn master(&self, value: Option<(f32, bool)>) -> Result<(String, f32, bool), Error> {
        self.request(|reply| Command::Master(value, reply))
    }
    pub fn apps(&self) -> Result<Vec<App>, Error> {
        self.request(Command::Apps)
    }
    /// Device peak for every active render endpoint; callers choose their output meter.
    pub fn output_peaks(&self) -> Result<Vec<OutputPeak>, Error> {
        self.request(Command::Peaks)
    }
    /// Remember and apply all faders/mutes to every session of this executable, including
    /// future sessions. Group/channel membership remains the caller's responsibility.
    /// Queued: a failure arrives as [`MixerEvent::SetFailed`].
    pub fn set_app(&self, exe: &str, settings: MixSettings) -> Result<(), Error> {
        self.commands
            .send(Command::Set(exe.to_lowercase(), settings))
            .map_err(|e| Error::Thread(e.to_string()))
    }
    /// Faders for sessions of apps that have no settings yet — the default channel — applied
    /// on the worker as the session appears, so a new app's first sound is not played at full
    /// volume while the daemon classifies it. `None` leaves new apps alone.
    pub fn set_new_app(&self, settings: Option<MixSettings>) -> Result<(), Error> {
        self.request(|reply| Command::NewApp(settings, reply))
    }
    /// Stop managing this executable (including future sessions), and conditionally
    /// restore its original volume/mute. Failed or absent sessions remain journaled.
    pub fn restore_app(&self, exe: &str) -> Result<(), Error> {
        self.request(|reply| Command::RestoreApp(exe.to_lowercase(), reply))
    }
    pub fn restore(&self) -> Result<(), Error> {
        self.request(Command::Restore)
    }
    /// Logoff/shutdown: leave app volumes and the system volume as they are, so the next
    /// logon plays as quietly as this one ended until Macafon starts again. The journal
    /// keeps everything for a later restore. Drop the watcher afterwards.
    pub fn keep(&self) -> Result<(), Error> {
        self.request(Command::Keep)
    }
    /// WIN-CH-013 on: Master is the device volume on the cubic curve and the loudest app is
    /// lifted to full session volume. Off: Master mirrors the Windows volume slider and
    /// sessions are never lifted. Turning it off gives the lift back first.
    pub fn set_managed(&self, managed: bool) -> Result<(), Error> {
        self.request(|reply| Command::Manage(managed, reply))
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
/// Macafon's own stream (A/B player, preview, the cable), watched but never set.
struct InternalSession {
    endpoint: String,
    instance: String,
    control: IAudioSessionControl2,
    callback: IAudioSessionEvents,
}
impl Drop for InternalSession {
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
    signal: Arc<Signal>,
    created: Arc<CreatedQueue>,
    endpoints: Vec<Endpoint>,
    sessions: Vec<LiveSession>,
    internal: Vec<InternalSession>,
    settings: HashMap<String, MixSettings>,
    /// Faders for apps without settings (`set_new_app`).
    new_app: Option<MixSettings>,
    /// Apps the caller stopped managing: never given the new-app faders again.
    released: std::collections::HashSet<String>,
    /// New sessions whose settings are not applied yet; kept until a write succeeds.
    unapplied: Vec<String>,
    /// The default output's volume: Master and the lift (see [`crate::lift`]).
    level: Option<Level>,
    store: Store,
    /// Stop without restoring (logoff/shutdown).
    keep: bool,
    /// WIN-CH-013: the system volume is Macafon's; else Master mirrors the Windows slider.
    managed: bool,
    /// The default output's volume while not managed.
    mirror: Option<(String, IAudioEndpointVolume)>,
    events: Sender<MixerEvent>,
}

/// The default output's volume control, reactivated only when the default endpoint changes.
struct Level {
    id: String,
    volume: IAudioEndpointVolume,
    min_db: f32,
    top_db: f32,
    /// The device's volume step; 0 when it does not say.
    step_db: f32,
    /// The Master fader; changes only from the host or the system volume slider.
    master_pct: f32,
    muted: bool,
    /// The device volume as last written or seen.
    written_db: f32,
    ramp: Ramp,
}
impl Level {
    /// Take over a device as it is: Master is its volume, nothing lifted.
    fn adopt(id: String, volume: IAudioEndpointVolume) -> Result<Self, Error> {
        let (mut min_db, mut max_db, mut step_db) = (0.0, 0.0, 0.0);
        // SAFETY: a live endpoint volume on the COM worker; out-pointers are locals.
        let (current, muted) = unsafe {
            volume.GetVolumeRange(&mut min_db, &mut max_db, &mut step_db)?;
            (volume.GetMasterVolumeLevel()?, volume.GetMute()?.as_bool())
        };
        Ok(Self {
            id,
            min_db,
            top_db: max_db.min(0.0).max(min_db),
            step_db: step_db.clamp(0.0, lift::STEP_DB),
            master_pct: lift::master_pct(current),
            muted,
            written_db: current,
            ramp: Ramp::new(0.0),
            volume,
        })
    }
    /// Master as device dB within the device's range.
    fn master_db(&self) -> f32 {
        lift::master_db(self.master_pct).clamp(self.min_db, self.top_db)
    }
    fn device_db(&self, lift_db: f32) -> f32 {
        (self.master_db() - lift_db).clamp(self.min_db, self.top_db)
    }
    /// What the sessions of this endpoint are multiplied by: the device is this much lower.
    fn boost(&self) -> f32 {
        lift::gain((self.master_db() - self.written_db).max(0.0))
    }
    fn saved(&self, kept: bool) -> Saved {
        Saved {
            endpoint: self.id.clone(),
            master_db: self.master_db(),
            written_db: self.written_db,
            kept,
        }
    }
}
impl Worker {
    fn new(
        events: Sender<MixerEvent>,
        commands: Sender<Command>,
        state_dir: &Path,
    ) -> Result<Self, Error> {
        let enumerator = enumerator()?;
        let signal = Arc::new(Signal {
            dirty: AtomicBool::new(false),
            queued: AtomicBool::new(false),
            commands,
        });
        let devices_callback = DeviceEvents {
            signal: signal.clone(),
        }
        .into();
        // SAFETY: enumerator and callback are owned until explicit unregistration.
        unsafe {
            enumerator.RegisterEndpointNotificationCallback(&devices_callback)?;
        }
        Ok(Self {
            enumerator,
            devices_callback,
            signal,
            created: Arc::new(CreatedQueue::default()),
            endpoints: Vec::new(),
            sessions: Vec::new(),
            internal: Vec::new(),
            settings: HashMap::new(),
            new_app: None,
            released: std::collections::HashSet::new(),
            unapplied: Vec::new(),
            level: None,
            managed: true,
            mirror: None,
            store: Store::new(state_dir),
            keep: false,
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
                        signal: self.signal.clone(),
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
                let sounddeck = pid == std::process::id()
                    && control.GetGroupingParam().ok() == Some(crate::wasapi::SOUNDDECK_SESSION);
                // Internal cable/A-B/preview streams must never share the monitor's fader.
                // They are only watched: one playing on the default output holds the lift.
                if pid == std::process::id() && !sounddeck {
                    if !self.internal.iter().any(|s| s.instance == instance) {
                        let callback: IAudioSessionEvents = SessionEvents {
                            signal: self.signal.clone(),
                            foreign: Arc::new(AtomicU64::new(0)),
                        }
                        .into();
                        control.RegisterAudioSessionNotification(&callback)?;
                        self.internal.push(InternalSession {
                            endpoint,
                            instance,
                            control,
                            callback,
                        });
                    }
                    continue;
                }
                let system = control.IsSystemSoundsSession() == S_OK;
                let executable = (!system)
                    .then(|| crate::sessions::executable(pid))
                    .flatten();
                let exe = if sounddeck {
                    SOUNDDECK.into()
                } else if system {
                    SYSTEM_SOUNDS.into()
                } else {
                    executable
                        .as_deref()
                        .and_then(|p| p.rsplit('\\').next().map(str::to_lowercase))
                        .unwrap_or_else(|| format!("<process-{pid}>"))
                };
                let foreign = Arc::new(AtomicU64::new(0));
                let callback = SessionEvents {
                    signal: self.signal.clone(),
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
            self.internal.retain(|s| {
                ids.contains(&s.endpoint)
                    && s.control
                        .GetState()
                        .is_ok_and(|state| state != AudioSessionStateExpired)
            });
        }
        Ok(())
    }

    /// Accept volume changes made outside Macafon. The changed session keeps its value; the
    /// app's other sessions follow it, since they share one app fader.
    fn external_changes(&mut self, journal: &mut Journal) {
        let mut siblings = Vec::new();
        for session in &self.sessions {
            let bits = session.foreign.swap(0, Ordering::AcqRel);
            if bits & VOLUME_PENDING == 0 {
                continue;
            }
            // The lift is Macafon's, not the app's fader.
            let observed = f32::from_bits(bits as u32) / self.boost(&session.key.endpoint);
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
                siblings.extend(
                    self.sessions
                        .iter()
                        .filter(|s| s.key.exe == session.key.exe && s.instance != session.instance)
                        .map(|s| (s.instance.clone(), settings)),
                );
            }
            // The app's own mute: a channel or Master mute shows on the session too.
            let _ = self.events.send(MixerEvent::ExternalVolume {
                exe: session.key.exe.clone(),
                app_pct: settings.app_pct,
                muted: settings.app_muted,
            });
        }
        if !siblings.is_empty() {
            for (instance, error) in self.apply_all(&siblings, journal) {
                let _ = self
                    .events
                    .send(MixerEvent::Error(format!("{instance}: {error}")));
            }
        }
    }

    /// Write faders to sessions: every change is journaled first, in one durable write,
    /// then set. Returns the instances that failed.
    fn apply_all(
        &mut self,
        targets: &[(String, MixSettings)],
        journal: &mut Journal,
    ) -> Vec<(String, Error)> {
        let mut failed = Vec::new();
        let mut writes = Vec::new();
        for (instance, settings) in targets {
            let Some(index) = self.sessions.iter().position(|s| &s.instance == instance) else {
                failed.push((instance.clone(), Error::DeviceNotFound(instance.clone())));
                continue;
            };
            let session = &self.sessions[index];
            // SAFETY: these are live session interfaces on their owning MTA worker.
            let previous = unsafe {
                session.volume.GetMasterVolume().and_then(|volume| {
                    Ok(Value {
                        volume,
                        muted: session.volume.GetMute()?.as_bool(),
                    })
                })
            };
            let previous = match previous {
                Ok(previous) => previous,
                Err(error) => {
                    failed.push((instance.clone(), error.into()));
                    continue;
                }
            };
            let volume =
                session_volume(settings.channel_pct, settings.app_pct, settings.master_pct)
                    * self.boost(&session.key.endpoint);
            let target = Value {
                volume: volume.min(1.0),
                muted: settings.channel_muted || settings.app_muted || settings.master_muted,
            };
            // The lift is recomputed from the device as read back: float noise is no change.
            if previous.muted == target.muted
                && (previous.volume - target.volume).abs()
                    <= previous.volume.max(target.volume) * 1e-3
            {
                continue;
            }
            journal.record(
                &StoredSession {
                    key: session.key.clone(),
                    instance: instance.clone(),
                    value: previous,
                },
                target,
                |instance| self.sessions.iter().any(|s| s.instance == instance),
            );
            writes.push((index, previous, target));
        }
        // Write-ahead: nothing is set unless its recovery record is on disk.
        if let Err(error) = journal.commit() {
            let message = error.to_string();
            failed.extend(writes.into_iter().map(|(index, ..)| {
                (
                    self.sessions[index].instance.clone(),
                    Error::Journal(message.clone()),
                )
            }));
            return failed;
        }
        for (index, previous, target) in writes {
            let session = &self.sessions[index];
            // SAFETY: live session interfaces on their owning MTA worker.
            let result = unsafe {
                (if previous.volume != target.volume {
                    session.volume.SetMasterVolume(target.volume, &CONTEXT)
                } else {
                    Ok(())
                })
                .and_then(|()| {
                    if previous.muted != target.muted {
                        session.volume.SetMute(target.muted, &CONTEXT)
                    } else {
                        Ok(())
                    }
                })
            };
            if let Err(error) = result {
                failed.push((session.instance.clone(), error.into()));
            }
        }
        failed
    }

    /// Apply the remembered settings to new sessions; a failed one is retried on the next
    /// refresh instead of being taken for an existing, already handled session.
    fn apply_new(&mut self, journal: &mut Journal) -> Result<(), Error> {
        let mut targets = Vec::new();
        let mut defaulted = Vec::new();
        for instance in std::mem::take(&mut self.unapplied) {
            let Some(session) = self.sessions.iter().find(|s| s.instance == instance) else {
                continue; // Gone.
            };
            let exe = session.key.exe.clone();
            let settings = match self.settings.get(&exe) {
                Some(settings) => *settings,
                None => match self.new_app.filter(|_| !self.released.contains(&exe)) {
                    Some(mut settings) => {
                        // Windows remembers an app's own mute across launches; the default
                        // faders must not lift it.
                        // SAFETY: a live session interface owned by this worker.
                        settings.app_muted =
                            unsafe { session.volume.GetMute() }.is_ok_and(|m| m.as_bool());
                        defaulted.push((instance.clone(), exe, settings));
                        settings
                    }
                    None => continue, // Unmanaged.
                },
            };
            targets.push((instance, settings));
        }
        let failed = self.apply_all(&targets, journal);
        for (instance, exe, settings) in defaulted {
            if !failed.iter().any(|(bad, _)| *bad == instance) {
                let _ = self
                    .events
                    .send(MixerEvent::NewAppDefaulted { exe: exe.clone() });
                if settings.app_muted {
                    // The host keeps the app's own mute for its later fader writes.
                    let _ = self.events.send(MixerEvent::ExternalVolume {
                        exe,
                        app_pct: settings.app_pct,
                        muted: true,
                    });
                }
            }
        }
        let mut first_error = None;
        for (instance, error) in failed {
            self.unapplied.push(instance);
            first_error.get_or_insert(error);
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

    /// How much the sessions on `endpoint` are lifted; 1 off the default output.
    fn boost(&self, endpoint: &str) -> f32 {
        self.level
            .as_ref()
            .filter(|level| level.id == endpoint)
            .map_or(1.0, Level::boost)
    }

    /// The faders a session plays at: its app's, or the new-app faders until the host
    /// classifies it (keeping the app's own mute, as `apply_new` does).
    fn mix_for(&self, session: &LiveSession) -> Option<MixSettings> {
        if let Some(settings) = self.settings.get(&session.key.exe) {
            return Some(*settings);
        }
        let mut settings = self
            .new_app
            .filter(|_| !self.released.contains(&session.key.exe))?;
        // SAFETY: a live session interface owned by this worker.
        settings.app_muted = unsafe { session.volume.GetMute() }.is_ok_and(|m| m.as_bool());
        Some(settings)
    }

    /// The default output's id.
    fn default_output(&self) -> Result<(String, IMMDevice), Error> {
        // SAFETY: called on the COM worker.
        let device = unsafe {
            self.enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)?
        };
        Ok((device_id(&device)?, device))
    }

    /// Follow the default output. A volume changed in Windows moves Master (explicit user
    /// changes persist and are not restored on Quit); a new default device adopts its volume,
    /// and the old one gets its lift back.
    fn sync_level(&mut self, journal: &mut Journal) -> Result<(), Error> {
        if !self.managed {
            return Ok(());
        }
        let (id, device) = self.default_output()?;
        if let Some(level) = self.level.as_mut().filter(|level| level.id == id) {
            // SAFETY: live endpoint volume on its owning worker.
            let (current, muted) = unsafe {
                (
                    level.volume.GetMasterVolumeLevel()?,
                    level.volume.GetMute()?.as_bool(),
                )
            };
            level.muted = muted;
            if (current - level.written_db).abs() > lift::FOREIGN_DB {
                // The lift stays where it is; the slider moved Master.
                let lift_db = (level.master_db() - level.written_db).max(0.0);
                level.master_pct = lift::master_pct(current + lift_db);
                level.written_db = current;
                if lift_db > 0.0 {
                    self.store.save(&level.saved(false))?;
                }
            }
            return Ok(());
        }
        let level = self.take_over(id, &device)?;
        if let Some(old) = self.level.replace(level) {
            self.unlift(old, journal);
        }
        // The new default's sessions get its lift (none for a device Macafon never lowered).
        let id = self.level.as_ref().map(|level| level.id.clone());
        if let Some(id) = id {
            self.reapply(journal, &id);
        }
        Ok(())
    }

    /// A device becomes the default output. One Macafon lowered and nobody touched since
    /// (kept at logoff, or left by a crash or an earlier switch) keeps its Master and lift;
    /// any other is taken as it is.
    fn take_over(&self, id: String, device: &IMMDevice) -> Result<Level, Error> {
        // SAFETY: called on the COM worker.
        let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let mut level = Level::adopt(id, volume)?;
        let record = self
            .store
            .load()
            .unwrap_or_default()
            .into_iter()
            .find(|saved| saved.endpoint == level.id);
        if let Some(saved) = record {
            if untouched(&saved, level.written_db) {
                level.master_pct = lift::master_pct(saved.master_db);
                level.ramp = Ramp::new(level.master_db() - level.written_db);
                self.store.save(&level.saved(false))?;
            } else {
                // Moved since: the user's volume.
                self.store.clear(&level.id)?;
            }
        }
        Ok(level)
    }

    /// An output stopped being the default: its sessions lose the lift, then the device
    /// gets it back. Sessions first: quieter for a moment, never louder. On failure the
    /// device stays lowered and its record stays for a later start.
    fn unlift(&mut self, old: Level, journal: &mut Journal) {
        if old.written_db >= old.master_db() - 0.01 {
            return;
        }
        if !self.reapply(journal, &old.id) {
            return;
        }
        // SAFETY: the old endpoint's interface is still owned by this worker.
        match unsafe { old.volume.SetMasterVolumeLevel(old.master_db(), &CONTEXT) } {
            Ok(()) => {
                if let Err(error) = self.store.clear(&old.id) {
                    let _ = self.events.send(MixerEvent::Error(error.to_string()));
                }
            }
            Err(error) => {
                let _ = self.events.send(MixerEvent::Error(format!(
                    "Cannot give the previous output its volume back: {error}"
                )));
            }
        }
    }

    /// Write the faders again to the managed sessions of one endpoint: its lift changed.
    /// Whether every write landed.
    fn reapply(&mut self, journal: &mut Journal, endpoint: &str) -> bool {
        let targets: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| s.key.endpoint == endpoint)
            .filter_map(|s| {
                self.mix_for(s)
                    .map(|settings| (s.instance.clone(), settings))
            })
            .collect();
        let failed = self.apply_all(&targets, journal);
        let ok = failed.is_empty();
        for (instance, error) in failed {
            let _ = self
                .events
                .send(MixerEvent::Error(format!("{instance}: {error}")));
        }
        ok
    }

    /// Set the device for `lift_db` and lift its sessions to match. When the device goes
    /// down it is written first, when it goes up the sessions are, and only if they all
    /// landed: in between the sound is a little quieter, never louder.
    fn set_lift(&mut self, lift_db: f32, journal: &mut Journal) -> Result<(), Error> {
        let Some(level) = self.level.as_mut() else {
            return Ok(());
        };
        let id = level.id.clone();
        let target_db = level.device_db(lift_db);
        let previous_lift = level.ramp.lift_db;
        level.ramp.lift_db = lift_db;
        let raising = target_db > level.written_db;
        if lift_db > 0.0 || level.written_db < level.master_db() - 0.01 {
            // Write-ahead: a crash in between leaves a record to undo.
            let mut saved = level.saved(false);
            saved.written_db = target_db.min(level.written_db);
            self.store.save(&saved)?;
        }
        if raising {
            // A device may round up: lift the sessions for one volume step more.
            let expected = (target_db + level.step_db).min(level.top_db);
            let before = std::mem::replace(&mut level.written_db, expected);
            let landed = self.reapply(journal, &id);
            let level = self
                .level
                .as_mut()
                .expect("the level is not dropped while set");
            level.written_db = before;
            if !landed {
                // Keep the device down; the next step retries.
                level.ramp.lift_db = previous_lift;
                return Err(Error::Journal("app volumes not lowered".into()));
            }
        }
        let level = self
            .level
            .as_mut()
            .expect("the level is not dropped while set");
        // SAFETY: live endpoint volume on its owning worker.
        let written = unsafe { level.volume.SetMasterVolumeLevel(target_db, &CONTEXT) };
        // A failed write leaves the device where it was: the sessions follow what it is.
        // SAFETY: as above.
        if let Ok(actual) = unsafe { level.volume.GetMasterVolumeLevel() } {
            level.written_db = actual;
        }
        let saved = level.saved(false);
        let done = level.written_db >= level.master_db() - 0.01;
        self.reapply(journal, &id);
        if done {
            self.store.clear(&id)?;
        } else {
            self.store.save(&saved)?;
        }
        written.map_err(Error::from)
    }

    /// The Master fader from the host: the device moves, the lift stays. A mute goes on
    /// before and comes off after the volume change.
    fn master(
        &mut self,
        value: Option<(f32, bool)>,
        journal: &mut Journal,
    ) -> Result<(String, f32, bool), Error> {
        if !self.managed {
            return self.mirror_master(value);
        }
        self.sync_level(journal)?;
        let missing = || Error::DeviceNotFound("default output".into());
        if let Some((percent, muted)) = value {
            let level = self.level.as_mut().ok_or_else(missing)?;
            if muted && !level.muted {
                // SAFETY: live endpoint volume on its owning worker.
                unsafe { level.volume.SetMute(true, &CONTEXT)? };
                level.muted = true;
            }
            level.master_pct = pct(percent);
            let lift_db = level.ramp.lift_db;
            self.set_lift(lift_db, journal)?;
            let level = self.level.as_mut().ok_or_else(missing)?;
            if !muted && level.muted {
                // SAFETY: as above.
                unsafe { level.volume.SetMute(false, &CONTEXT)? };
                level.muted = false;
            }
        }
        let level = self.level.as_ref().ok_or_else(missing)?;
        Ok((level.id.clone(), level.master_pct, level.muted))
    }

    /// Not managed: Master is the Windows volume slider as it is (explicit user changes
    /// persist and are not restored on Quit).
    fn mirror_master(&mut self, value: Option<(f32, bool)>) -> Result<(String, f32, bool), Error> {
        let (id, device) = self.default_output()?;
        let volume = match &self.mirror {
            Some((known, volume)) if *known == id => volume.clone(),
            _ => {
                // SAFETY: called on the COM worker.
                let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
                self.mirror = Some((id.clone(), volume.clone()));
                volume
            }
        };
        // SAFETY: live endpoint volume on its owning worker.
        unsafe {
            if let Some((percent, muted)) = value {
                if muted {
                    volume.SetMute(true, &CONTEXT)?;
                }
                volume.SetMasterVolumeLevelScalar(pct(percent) / 100.0, &CONTEXT)?;
                if !muted {
                    volume.SetMute(false, &CONTEXT)?;
                }
            }
            Ok((
                id,
                volume.GetMasterVolumeLevelScalar()? * 100.0,
                volume.GetMute()?.as_bool(),
            ))
        }
    }

    /// Switch WIN-CH-013 on or off. Off gives the lift back at once (sessions first, then
    /// the device) and keeps it if the sessions cannot be written.
    fn set_managed(&mut self, managed: bool, journal: &mut Journal) -> Result<(), Error> {
        if managed == self.managed {
            return Ok(());
        }
        if managed {
            self.managed = true;
            self.mirror = None;
            return self.sync_level(journal);
        }
        // The device as it is now: a volume moved in Windows, or a lift an earlier run left
        // on it (not taken over yet), must be what is given back.
        self.sync_level(journal)?;
        if self.level.is_some() {
            if self.new_app.is_none() && self.settings.is_empty() {
                // Nothing to write the faders from yet (just started from a kept logoff):
                // the lifted sessions get their originals before the device goes up.
                journal.restore(self)?;
            }
            self.set_lift(0.0, journal)?;
        }
        if let Some(level) = self.level.take() {
            self.store.clear(&level.id)?;
        }
        self.managed = false;
        Ok(())
    }

    /// The loudest app the default output plays now, as its session gain before the lift.
    /// An active app Macafon does not manage counts at full scale: lifting would make it
    /// quieter.
    fn reference(&self) -> Option<f32> {
        let level = self.level.as_ref()?;
        // SAFETY: live session interfaces on their owning worker.
        let internal = self.internal.iter().any(|s| {
            s.endpoint == level.id
                && unsafe { s.control.GetState() }
                    .is_ok_and(|state| state == AudioSessionStateActive)
        });
        if internal {
            // The A/B player is not lifted: it plays at full device volume.
            return Some(1.0);
        }
        let mut loudest: Option<f32> = None;
        for session in self.sessions.iter().filter(|s| s.key.endpoint == level.id) {
            // SAFETY: live session interfaces on their owning worker.
            let active = unsafe { session.control.GetState() }
                .is_ok_and(|state| state == AudioSessionStateActive);
            if !active {
                continue;
            }
            let gain = match self.mix_for(session) {
                Some(s) if s.channel_muted || s.app_muted || s.master_muted => continue,
                Some(s) => session_volume(s.channel_pct, s.app_pct, s.master_pct),
                None => {
                    // SAFETY: as above.
                    let audible = unsafe {
                        session.volume.GetMasterVolume().is_ok_and(|v| v > 0.0)
                            && session.volume.GetMute().is_ok_and(|m| !m.as_bool())
                    };
                    if !audible {
                        continue;
                    }
                    1.0
                }
            };
            loudest = Some(loudest.map_or(gain, |l| l.max(gain)));
        }
        loudest
    }

    /// Move the lift one step toward the loudest app; returns when to look again.
    fn step_lift(&mut self, journal: &mut Journal) -> Option<Duration> {
        // Nothing is managed (the host has not caught up after a start, or the mixer is off
        // and `restore` already undid the lift), or kept for the next logon: hold.
        if self.keep || (self.new_app.is_none() && self.settings.is_empty()) {
            return None;
        }
        let reference = self.reference();
        let level = self.level.as_mut()?;
        let target = lift::target_lift(reference, level.master_db() - level.min_db);
        if let Some(next) = level.ramp.next(target, Instant::now()) {
            if let Err(error) = self.set_lift(next, journal) {
                let _ = self.events.send(MixerEvent::Error(format!(
                    "Cannot set the system volume: {error}"
                )));
                return Some(Duration::from_millis(500));
            }
        }
        self.level.as_ref()?.ramp.wake_in(target, Instant::now())
    }

    /// Mixer off or Quit, after the journal gave the sessions their originals back: the
    /// device gets Master.
    fn undo_lift(&mut self) -> Result<(), Error> {
        let Some(level) = self.level.as_mut() else {
            return Ok(());
        };
        level.ramp = Ramp::new(0.0);
        let target = level.master_db();
        if level.written_db < target - 0.01 {
            // SAFETY: live endpoint volume on its owning worker.
            unsafe {
                level.volume.SetMasterVolumeLevel(target, &CONTEXT)?;
                level.written_db = level.volume.GetMasterVolumeLevel()?;
            }
        }
        let id = level.id.clone();
        self.store.clear(&id)
    }

    /// At start, before anything else: whether the lift kept at logoff can be taken over as
    /// it is. Only when every lowered device is the default output and untouched; otherwise
    /// the sessions go back to their originals first and the devices after.
    fn can_take_over(&self, saved: &[Saved]) -> bool {
        if !saved.iter().any(|saved| saved.kept) {
            return false;
        }
        let Ok((default, _)) = self.default_output() else {
            return false;
        };
        saved.iter().all(|saved| {
            saved.endpoint == default
                && self
                    .device_volume(&saved.endpoint)
                    .is_some_and(|(_, current)| untouched(saved, current))
        })
    }

    /// At start, after the session journal: devices lowered by an earlier run get their
    /// volume back, except the default output, which `sync_level` takes over with its lift.
    /// A device volume moved since then is the user's and stays.
    fn recover_levels(&mut self, saved: Vec<Saved>) {
        let default = self.default_output().ok().map(|(id, _)| id);
        for saved in saved {
            if Some(&saved.endpoint) == default.as_ref() {
                continue;
            }
            // Absent: undone when it is back at a later start.
            let Some((volume, current)) = self.device_volume(&saved.endpoint) else {
                continue;
            };
            let result = (|| {
                if untouched(&saved, current) {
                    // SAFETY: live endpoint volume on this worker.
                    unsafe { volume.SetMasterVolumeLevel(saved.master_db, &CONTEXT)? };
                }
                self.store.clear(&saved.endpoint)
            })();
            if let Err(error) = result {
                let _ = self
                    .events
                    .send(MixerEvent::Error(format!("system volume: {error}")));
            }
        }
    }

    fn device_volume(&self, id: &str) -> Option<(IAudioEndpointVolume, f32)> {
        // SAFETY: COM worker; the HSTRING outlives the call.
        unsafe {
            let device = self.enumerator.GetDevice(&HSTRING::from(id)).ok()?;
            let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
            let current = volume.GetMasterVolumeLevel().ok()?;
            Some((volume, current))
        }
    }

    fn run(&mut self, commands: Receiver<Command>, journal: &mut Journal) {
        let mut refresh_after = Instant::now();
        loop {
            // Cleared before reading the flags: a later callback queues a new wake-up.
            self.signal.queued.store(false, Ordering::Release);
            self.external_changes(journal);
            if Instant::now() >= refresh_after && self.signal.dirty.swap(false, Ordering::AcqRel) {
                let result = self
                    .refresh()
                    .and_then(|()| self.sync_level(journal))
                    .and_then(|()| self.apply_new(journal));
                if let Err(e) = result {
                    self.signal.dirty.store(true, Ordering::Release);
                    refresh_after = Instant::now() + Duration::from_millis(500);
                    let _ = self.events.send(MixerEvent::Error(e.to_string()));
                }
                let _ = self.events.send(MixerEvent::AppsChanged);
            }
            // Queued faders: a held key sends one per frame and app; only the latest per app
            // is written, in one journal write, before whatever command came after them.
            // Callbacks wake the loop; the timeout only paces a refresh retry after an error.
            let mut wait = if self.signal.dirty.load(Ordering::Acquire) {
                refresh_after.saturating_duration_since(Instant::now())
            } else {
                Duration::from_secs(1)
            };
            if let Some(lift) = self.step_lift(journal) {
                wait = wait.min(lift);
            }
            let mut received = commands.recv_timeout(wait.max(Duration::from_millis(1)));
            let mut sets: Vec<(String, MixSettings)> = Vec::new();
            while let Ok(Command::Set(exe, settings)) = received {
                match sets.iter_mut().find(|(known, _)| *known == exe) {
                    Some(set) => set.1 = settings,
                    None => sets.push((exe, settings)),
                }
                received = commands.try_recv().map_err(|error| match error {
                    mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
                    mpsc::TryRecvError::Disconnected => mpsc::RecvTimeoutError::Disconnected,
                });
            }
            if !sets.is_empty() {
                self.set_apps(sets, journal);
            }
            match received {
                Ok(Command::Master(value, reply)) => {
                    let result = self.master(value, journal);
                    let _ = reply.send(result);
                }
                Ok(Command::Apps(reply)) => {
                    self.external_changes(journal);
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
                Ok(Command::NewApp(settings, reply)) => {
                    self.new_app = settings;
                    let _ = reply.send(Ok(()));
                }
                Ok(Command::RestoreApp(exe, reply)) => {
                    self.settings.remove(&exe);
                    self.released.insert(exe.clone());
                    let result = journal.restore_app(self, &exe);
                    self.report_pending(journal);
                    let _ = reply.send(result);
                }
                Ok(Command::Restore(reply)) => {
                    self.settings.clear();
                    self.new_app = None;
                    // The device stays down unless the sessions are back.
                    let result = journal.restore(self).and_then(|()| self.undo_lift());
                    self.report_pending(journal);
                    let _ = reply.send(result);
                }
                Ok(Command::Manage(managed, reply)) => {
                    let result = self.set_managed(managed, journal);
                    let _ = reply.send(result);
                }
                Ok(Command::Keep(reply)) => {
                    self.keep = true;
                    // Saved even without a lift: the next start takes the sessions over
                    // instead of restoring them for a moment.
                    let result = self
                        .level
                        .as_ref()
                        .map_or(Ok(()), |level| self.store.save(&level.saved(true)));
                    let _ = reply.send(result);
                }
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if !self.keep {
                        let result = journal.restore(self).and_then(|()| self.undo_lift());
                        if let Err(e) = result {
                            let _ = self.events.send(MixerEvent::Error(e.to_string()));
                        }
                    }
                    break;
                }
                // Sets were drained above.
                Ok(Command::Set(..) | Command::Wake) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn set_apps(&mut self, sets: Vec<(String, MixSettings)>, journal: &mut Journal) {
        self.external_changes(journal);
        let mut targets = Vec::new();
        for (exe, settings) in &sets {
            self.released.remove(exe);
            self.settings.insert(exe.clone(), *settings);
            targets.extend(
                self.sessions
                    .iter()
                    .filter(|s| &s.key.exe == exe)
                    .map(|s| (s.instance.clone(), *settings)),
            );
        }
        let failed = self.apply_all(&targets, journal);
        for (exe, _) in sets {
            let message = failed.iter().find_map(|(instance, error)| {
                self.sessions
                    .iter()
                    .any(|s| &s.instance == instance && s.key.exe == exe)
                    .then(|| error.to_string())
            });
            if let Some(message) = message {
                let _ = self.events.send(MixerEvent::SetFailed { exe, message });
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
/// The device still shows what Macafon wrote, give or take one step of the ramp.
fn untouched(saved: &Saved, current_db: f32) -> bool {
    (current_db - saved.written_db).abs() <= lift::STEP_DB + lift::FOREIGN_DB
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
    signal: Arc<Signal>,
    created: Arc<CreatedQueue>,
    endpoint: Arc<str>,
}
impl IAudioSessionNotification_Impl for SessionCreated_Impl {
    fn OnSessionCreated(&self, session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        if let Some(control) = session.cloned() {
            self.created.push(self.endpoint.clone(), control);
        }
        self.signal.changed();
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessionEvents {
    signal: Arc<Signal>,
    foreign: Arc<AtomicU64>,
}
impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        self.signal.changed();
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
            self.signal.wake();
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
        self.signal.changed();
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        self.signal.changed();
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceEvents {
    signal: Arc<Signal>,
}
impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        self.signal.changed();
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.signal.changed();
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.signal.changed();
        Ok(())
    }
    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        _: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.signal.changed();
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        self.signal.changed();
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
