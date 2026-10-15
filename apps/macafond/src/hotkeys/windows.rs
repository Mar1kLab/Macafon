//! The manager and its hidden window belong to this service thread, never the audio thread.

use super::{Binding, Status, press, status};
use crate::host::ControlMessage;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use macafon_backend::ControlSender;
use macafon_ipc::HotkeyBackend;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, SetTimer,
};

struct Backend {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: HashMap<u32, String>,
    held: HashSet<u32>,
    timer: usize,
}

impl Backend {
    fn new() -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|error| error.to_string())?;
        // SAFETY: a thread timer with no callback; GetMessageW below pumps its messages.
        let timer = unsafe { SetTimer(None, 0, 20, None) };
        if timer == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self {
            manager,
            registered: Vec::new(),
            actions: HashMap::new(),
            held: HashSet::new(),
            timer,
        })
    }

    fn release(&mut self, control: &ControlSender<ControlMessage>) {
        for id in self.held.drain() {
            if let Some(action) = self.actions.get(&id) {
                press(control, action, false);
            }
        }
    }

    fn bind(&mut self, bindings: Vec<Binding>, control: &ControlSender<ControlMessage>) {
        // A removed held binding must not leave push-to-talk active indefinitely.
        self.release(control);
        for key in self.registered.drain(..) {
            if let Err(error) = self.manager.unregister(key) {
                tracing::warn!(%error, "Cannot unregister a Windows hotkey");
            }
        }
        self.actions.clear();
        let mut errors = HashMap::new();
        for binding in bindings {
            let result = super::accel::hotkey(&binding.accel).and_then(|key| {
                self.manager
                    .register(key)
                    .map_err(|error| error.to_string())?;
                Ok(key)
            });
            match result {
                Ok(key) => {
                    self.actions.insert(key.id(), binding.action);
                    self.registered.push(key);
                }
                Err(error) => {
                    tracing::warn!(action = %binding.description, %error, "Cannot register a Windows hotkey");
                    errors.insert(binding.action, error);
                }
            }
        }
        status(
            control,
            Status {
                backend: HotkeyBackend::Windows,
                triggers: HashMap::new(),
                errors,
            },
        );
    }

    fn events(&mut self, control: &ControlSender<ControlMessage>) {
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            let Some(action) = self.actions.get(&event.id) else {
                continue;
            };
            let pressed = event.state == HotKeyState::Pressed;
            let changed = if pressed {
                self.held.insert(event.id)
            } else {
                self.held.remove(&event.id)
            };
            if changed {
                press(control, action, pressed);
            }
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        for key in &self.registered {
            let _ = self.manager.unregister(*key);
        }
        // SAFETY: the timer was created on this thread and has no callback.
        let _ = unsafe { KillTimer(None, self.timer) };
    }
}

pub fn run(receiver: Receiver<Vec<Binding>>, control: ControlSender<ControlMessage>) {
    let mut backend = match Backend::new() {
        Ok(backend) => backend,
        Err(error) => {
            tracing::warn!(%error, "Windows hotkeys unavailable");
            status(
                &control,
                Status {
                    backend: HotkeyBackend::Unavailable,
                    triggers: HashMap::new(),
                    errors: HashMap::new(),
                },
            );
            while let Ok(bindings) = receiver.recv() {
                status(
                    &control,
                    Status {
                        backend: HotkeyBackend::Unavailable,
                        triggers: HashMap::new(),
                        errors: bindings
                            .into_iter()
                            .map(|binding| (binding.action, error.clone()))
                            .collect(),
                    },
                );
            }
            return;
        }
    };
    backend.bind(Vec::new(), &control);
    let mut message = MSG::default();
    loop {
        // SAFETY: message is writable; no window filter, so the manager's hidden window
        // and the thread timer are both serviced on the manager's owning thread.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result <= 0 {
            if result < 0 {
                tracing::warn!(error = %std::io::Error::last_os_error(), "Windows hotkey message loop failed");
            }
            break;
        }
        // SAFETY: GetMessageW returned a valid message on this thread.
        unsafe { DispatchMessageW(&message) };
        backend.events(&control);
        let mut latest = None;
        loop {
            match receiver.try_recv() {
                Ok(bindings) => latest = Some(bindings),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    backend.release(&control);
                    return;
                }
            }
        }
        if let Some(bindings) = latest {
            backend.bind(bindings, &control);
        }
    }
    backend.release(&control);
}
