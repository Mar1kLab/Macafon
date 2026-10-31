//! Global hotkeys service (PRD §8.3): the daemon registers them, so they work with the window
//! closed (HK-003). A helper thread owns the backend; presses arrive on the control loop.

pub mod accel;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(windows)]
mod sided;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

use crate::host::ControlMessage;
use accel::Accel;
use macafon_backend::ControlSender;
use macafon_ipc::HotkeyBackend;
use std::collections::HashMap;
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};

/// A shortcut to register: action id, readable description, shortcut.
#[derive(Clone, PartialEq)]
pub struct Binding {
    pub action: String,
    pub description: String,
    pub accel: Accel,
}

/// What the backend reports back after (re)binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub backend: HotkeyBackend,
    /// Desktop-assigned triggers (portal).
    pub triggers: HashMap<String, String>,
    pub errors: HashMap<String, String>,
}

pub struct Hotkeys {
    bindings: mpsc::Sender<Vec<Binding>>,
    #[cfg(windows)]
    waker: windows::Waker,
}

impl Hotkeys {
    /// Start the service; it reports its backend through `ControlMessage::HotkeyStatus`.
    pub fn spawn(control: ControlSender<ControlMessage>) -> Self {
        let (bindings, receiver) = mpsc::channel();
        #[cfg(windows)]
        let waker = windows::Waker::default();
        #[cfg(windows)]
        let thread_waker = waker.clone();
        let spawned = std::thread::Builder::new()
            .name("macafon-hotkeys".into())
            .spawn(move || {
                #[cfg(windows)]
                windows::run(receiver, control, thread_waker);
                #[cfg(not(windows))]
                run_other(receiver, control);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "Cannot start the hotkey service");
        }
        Self {
            bindings,
            #[cfg(windows)]
            waker,
        }
    }

    /// A service without a backend; the test reads what would be registered.
    #[cfg(test)]
    pub fn fake() -> (Self, mpsc::Receiver<Vec<Binding>>) {
        let (bindings, receiver) = mpsc::channel();
        (
            Self {
                bindings,
                #[cfg(windows)]
                waker: windows::Waker::default(),
            },
            receiver,
        )
    }

    pub fn bind(&self, bindings: Vec<Binding>) {
        let _ = self.bindings.send(bindings);
        #[cfg(windows)]
        self.waker.wake();
    }
}

/// A default shortcut may be bound here: on Windows Ctrl+Alt also means AltGr, so a key that
/// types a character with AltGr in an installed layout (`€`, `@`, `ś`) is left alone.
pub fn usable_by_default(shortcut: &str) -> bool {
    let Ok(accel) = Accel::parse(shortcut) else {
        return false;
    };
    #[cfg(windows)]
    return !windows::types_with_altgr(&accel);
    #[cfg(not(windows))]
    {
        let _ = accel;
        true
    }
}

fn status(control: &ControlSender<ControlMessage>, status: Status) {
    let _ = control.send(ControlMessage::HotkeyStatus(status));
}

fn press(control: &ControlSender<ControlMessage>, action: &str, pressed: bool) {
    let _ = control.send(ControlMessage::Hotkey {
        action: action.to_owned(),
        pressed,
    });
}

#[cfg(not(windows))]
fn run_other(receiver: mpsc::Receiver<Vec<Binding>>, control: ControlSender<ControlMessage>) {
    #[cfg(target_os = "linux")]
    if let Some(portal) = portal::Portal::connect() {
        return run_portal(portal, receiver, control);
    }
    #[cfg(target_os = "linux")]
    if x11::available() {
        match x11::X11::new() {
            Ok(backend) => return run_x11(backend, receiver, control),
            Err(error) => tracing::warn!(%error, "X11 hotkeys unavailable"),
        }
    }
    tracing::info!("No global shortcut backend; CLI commands remain (HK-004)");
    status(
        &control,
        Status {
            backend: HotkeyBackend::Unavailable,
            triggers: HashMap::new(),
            errors: HashMap::new(),
        },
    );
    // Keep the channel open so binding updates are simply dropped.
    while receiver.recv().is_ok() {}
}

/// Stable, collision-free trigger identity: changing a preference must create a new portal
/// shortcut because KDE retains its previous assignment for an existing ID.
#[cfg(target_os = "linux")]
fn portal_id(action: &str, trigger: &str) -> String {
    use std::fmt::Write;
    let mut id = format!("{action}@");
    for byte in trigger.bytes() {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct PortalEvents {
    session: Option<zbus::zvariant::OwnedObjectPath>,
    actions: HashMap<String, String>,
    held: std::collections::HashSet<String>,
    triggers: HashMap<String, String>,
}

#[cfg(target_os = "linux")]
fn run_portal(
    mut portal: portal::Portal,
    receiver: mpsc::Receiver<Vec<Binding>>,
    control: ControlSender<ControlMessage>,
) {
    status(
        &control,
        Status {
            backend: HotkeyBackend::Portal,
            triggers: HashMap::new(),
            errors: HashMap::new(),
        },
    );
    // Serialize registration and event dispatch, including releases from a replaced session.
    let events = Arc::new(Mutex::new(PortalEvents::default()));
    let listening = events.clone();
    // Subscribe before accepting the first binding, so an immediate press cannot be lost.
    let connection = portal.connection().clone();
    let signals = control.clone();
    let (ready, started) = mpsc::channel();
    let listener = std::thread::Builder::new()
        .name("macafon-hotkey-signals".into())
        .spawn(move || {
            let Ok(proxy) = zbus::blocking::Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.GlobalShortcuts",
            ) else {
                return;
            };
            let Ok(messages) = proxy.receive_all_signals() else {
                return;
            };
            let _ = ready.send(());
            for message in messages {
                let header = message.header();
                let Some(member) = header.member() else {
                    continue;
                };
                let mut events = listening.lock().unwrap_or_else(|p| p.into_inner());
                if member.as_str() == "ShortcutsChanged" {
                    type Changed = (
                        zbus::zvariant::OwnedObjectPath,
                        Vec<(String, HashMap<String, zbus::zvariant::OwnedValue>)>,
                    );
                    if let Ok((session, shortcuts)) = message.body().deserialize::<Changed>()
                        && events.session.as_ref() == Some(&session)
                    {
                        for (id, props) in shortcuts {
                            if let Some(action) = events.actions.get(&id).cloned()
                                && let Some(trigger) = props.get("trigger_description")
                                && let Ok(trigger) = String::try_from(trigger.clone())
                            {
                                events.triggers.insert(action, trigger);
                            }
                        }
                        status(
                            &signals,
                            Status {
                                backend: HotkeyBackend::Portal,
                                triggers: events.triggers.clone(),
                                errors: HashMap::new(),
                            },
                        );
                    }
                    continue;
                }
                let pressed = match member.as_str() {
                    "Activated" => true,
                    "Deactivated" => false,
                    _ => continue,
                };
                type Body = (
                    zbus::zvariant::OwnedObjectPath,
                    String,
                    u64,
                    HashMap<String, zbus::zvariant::OwnedValue>,
                );
                if let Ok((session, id, _, _)) = message.body().deserialize::<Body>()
                    && events.session.as_ref() == Some(&session)
                    && let Some(action) = events.actions.get(&id).cloned()
                {
                    let changed = if pressed {
                        events.held.insert(action.clone())
                    } else {
                        events.held.remove(&action)
                    };
                    if changed {
                        press(&signals, &action, pressed);
                    }
                }
            }
        });
    if let Err(error) = listener {
        tracing::warn!(%error, "Cannot listen to shortcut signals");
    }
    let _ = started.recv();
    while let Ok(mut bindings) = receiver.recv() {
        // Only the latest set matters; the portal may show a dialog for each bind.
        while let Ok(newer) = receiver.try_recv() {
            bindings = newer;
        }
        let shortcuts: Vec<portal::Shortcut> = bindings
            .iter()
            .map(|binding| portal::Shortcut {
                id: portal_id(&binding.action, &binding.accel.portal_trigger()),
                description: binding.description.clone(),
                trigger: binding.accel.portal_trigger(),
            })
            .collect();
        let mut events = events.lock().unwrap_or_else(|p| p.into_inner());
        for action in events.held.drain() {
            press(&control, &action, false);
        }
        events.session = None;
        events.actions = shortcuts
            .iter()
            .zip(&bindings)
            .map(|(s, b)| (s.id.clone(), b.action.clone()))
            .collect();
        let (triggers, errors) = match portal.bind(&shortcuts) {
            Ok(triggers) => (
                triggers
                    .into_iter()
                    .filter_map(|(id, trigger)| {
                        events
                            .actions
                            .get(&id)
                            .map(|action| (action.clone(), trigger))
                    })
                    .collect(),
                HashMap::new(),
            ),
            Err(error) => {
                tracing::warn!(%error, "Portal did not bind the shortcuts");
                let errors = bindings
                    .iter()
                    .map(|binding| (binding.action.clone(), error.clone()))
                    .collect();
                (HashMap::new(), errors)
            }
        };
        events.session = portal.session().cloned();
        events.triggers = triggers.clone();
        status(
            &control,
            Status {
                backend: HotkeyBackend::Portal,
                triggers,
                errors,
            },
        );
    }
}

#[cfg(target_os = "linux")]
fn run_x11(
    mut backend: x11::X11,
    receiver: mpsc::Receiver<Vec<Binding>>,
    control: ControlSender<ControlMessage>,
) {
    status(
        &control,
        Status {
            backend: HotkeyBackend::X11,
            triggers: HashMap::new(),
            errors: HashMap::new(),
        },
    );
    let ids: Arc<Mutex<HashMap<u32, String>>> = Arc::default();
    let events = ids.clone();
    let presses = control.clone();
    let listener = std::thread::Builder::new()
        .name("macafon-hotkey-events".into())
        .spawn(move || {
            let receiver = global_hotkey::GlobalHotKeyEvent::receiver();
            while let Ok(event) = receiver.recv() {
                let action = events
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&event.id)
                    .cloned();
                if let Some(action) = action {
                    let pressed = event.state == global_hotkey::HotKeyState::Pressed;
                    press(&presses, &action, pressed);
                }
            }
        });
    if let Err(error) = listener {
        tracing::warn!(%error, "Cannot listen to X11 hotkeys");
    }
    while let Ok(bindings) = receiver.recv() {
        let list: Vec<(String, Accel)> = bindings
            .into_iter()
            .map(|binding| (binding.action, binding.accel))
            .collect();
        let (registered, errors) = backend.bind(&list);
        *ids.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = registered;
        status(
            &control,
            Status {
                backend: HotkeyBackend::X11,
                triggers: HashMap::new(),
                errors,
            },
        );
    }
}

/// Sleeps until the control loop's next volume deadline. Only one wake can be pending:
/// after sending it, wait for the host to supply the next deadline (or cancellation).
/// This thread never touches audio or runs hotkey actions.
pub(super) struct RepeatTimer(mpsc::Sender<Option<std::time::Instant>>);

impl RepeatTimer {
    pub(super) fn spawn(control: ControlSender<ControlMessage>) -> Self {
        let (sender, receiver) = mpsc::channel::<Option<std::time::Instant>>();
        let spawned = std::thread::Builder::new()
            .name("macafon-hotkey-repeat".into())
            .spawn(move || {
                let mut deadline: Option<std::time::Instant> = None;
                loop {
                    let result = if let Some(at) = deadline {
                        receiver
                            .recv_timeout(at.saturating_duration_since(std::time::Instant::now()))
                    } else {
                        receiver
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    };
                    match result {
                        Ok(next) => deadline = next,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            deadline = None;
                            if control.send(ControlMessage::HotkeyRepeat).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "Cannot start the hotkey repeat timer");
        }
        Self(sender)
    }

    pub(super) fn schedule(&self, deadline: Option<std::time::Instant>) {
        let _ = self.0.send(deadline);
    }
}

/// Escape is consumed by the hook only while a dictation recording is active.
pub fn dictation_recording(active: bool) {
    #[cfg(windows)]
    sided::DICTATION_RECORDING.store(active, std::sync::atomic::Ordering::Release);
    #[cfg(not(windows))]
    let _ = active;
}

#[cfg(test)]
mod repeat_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    // The control channel hands back the unsent message, as the daemon's does.
    #[allow(clippy::result_large_err)]
    fn timer_wakes_without_backend_ticks_and_waits_for_rearming() {
        let (sender, receiver) = mpsc::channel();
        let control =
            ControlSender::new(move |message| sender.send(message).map_err(|error| error.0));
        let timer = RepeatTimer::spawn(control);
        let deadline = Instant::now() + Duration::from_millis(20);
        timer.schedule(Some(deadline));
        assert!(matches!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("timer wake"),
            ControlMessage::HotkeyRepeat
        ));
        assert!(Instant::now() >= deadline, "never fire before the deadline");
        assert!(
            receiver.recv_timeout(Duration::from_millis(75)).is_err(),
            "a stalled host cannot build up repeat messages"
        );
        timer.schedule(Some(Instant::now()));
        assert!(matches!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("rearmed timer"),
            ControlMessage::HotkeyRepeat
        ));
        timer.schedule(Some(Instant::now() + Duration::from_millis(20)));
        timer.schedule(None);
        assert!(
            receiver.recv_timeout(Duration::from_millis(75)).is_err(),
            "release cancels the timer"
        );
    }
}
