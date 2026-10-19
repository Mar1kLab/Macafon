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
}

impl Hotkeys {
    /// Start the service; it reports its backend through `ControlMessage::HotkeyStatus`.
    pub fn spawn(control: ControlSender<ControlMessage>) -> Self {
        let (bindings, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("macafon-hotkeys".into())
            .spawn(move || run(receiver, control));
        if let Err(error) = spawned {
            tracing::warn!(%error, "Cannot start the hotkey service");
        }
        Self { bindings }
    }

    /// A service without a backend; the test reads what would be registered.
    #[cfg(test)]
    pub fn fake() -> (Self, mpsc::Receiver<Vec<Binding>>) {
        let (bindings, receiver) = mpsc::channel();
        (Self { bindings }, receiver)
    }

    pub fn bind(&self, bindings: Vec<Binding>) {
        let _ = self.bindings.send(bindings);
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

fn run(receiver: mpsc::Receiver<Vec<Binding>>, control: ControlSender<ControlMessage>) {
    #[cfg(windows)]
    windows::run(receiver, control);
    #[cfg(not(windows))]
    run_other(receiver, control);
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
    // Activated/Deactivated arrive on their own thread; ids are action ids.
    let connection = portal.connection().clone();
    let signals = control.clone();
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
            for message in messages {
                let header = message.header();
                let Some(member) = header.member() else {
                    continue;
                };
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
                if let Ok((_, id, _, _)) = message.body().deserialize::<Body>() {
                    press(&signals, &id, pressed);
                }
            }
        });
    if let Err(error) = listener {
        tracing::warn!(%error, "Cannot listen to shortcut signals");
    }
    while let Ok(mut bindings) = receiver.recv() {
        // Only the latest set matters; the portal may show a dialog for each bind.
        while let Ok(newer) = receiver.try_recv() {
            bindings = newer;
        }
        let shortcuts: Vec<portal::Shortcut> = bindings
            .iter()
            .map(|binding| portal::Shortcut {
                id: binding.action.clone(),
                description: binding.description.clone(),
                trigger: binding.accel.portal_trigger(),
            })
            .collect();
        let (triggers, errors) = match portal.bind(&shortcuts) {
            Ok(triggers) => (triggers, HashMap::new()),
            Err(error) => {
                tracing::warn!(%error, "Portal did not bind the shortcuts");
                let errors = bindings
                    .iter()
                    .map(|binding| (binding.action.clone(), error.clone()))
                    .collect();
                (HashMap::new(), errors)
            }
        };
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

#[cfg(test)]
mod repeat_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
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

/// Escape is consumed by the hook only while a dictation recording is active.
pub fn dictation_recording(active: bool) {
    #[cfg(windows)]
    sided::DICTATION_RECORDING.store(active, std::sync::atomic::Ordering::Release);
    #[cfg(not(windows))]
    let _ = active;
}
