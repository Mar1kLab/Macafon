//! The manager and its hidden window belong to this service thread, never the audio thread.

use super::{Binding, Status, press, status};
use crate::host::ControlMessage;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use macafon_backend::ControlSender;
use macafon_ipc::HotkeyBackend;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayoutList, HKL, MAPVK_VK_TO_VSC, MapVirtualKeyExW, ToUnicodeEx,
    VK_CONTROL, VK_LCONTROL, VK_MENU, VK_OEM_3, VK_RMENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, PostThreadMessageW, SetTimer, WM_NULL,
};

/// Wakes the hotkey thread's message loop for new bindings; 0 until its queue exists.
#[derive(Clone, Default)]
pub struct Waker(Arc<AtomicU32>);

impl Waker {
    pub fn wake(&self) {
        let thread = self.0.load(Ordering::Acquire);
        if thread != 0 {
            // SAFETY: posts an empty message to a thread of this process; no pointers.
            let _ = unsafe { PostThreadMessageW(thread, WM_NULL, WPARAM(0), LPARAM(0)) };
        }
    }
}

struct Backend {
    sided: Option<super::sided::Hook>,
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: HashMap<u32, String>,
    accelerators: HashMap<u32, super::accel::Accel>,
    held: HashSet<u32>,
    timer: usize,
}

impl Backend {
    fn new() -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|error| error.to_string())?;
        Ok(Self {
            sided: None,
            manager,
            registered: Vec::new(),
            actions: HashMap::new(),
            accelerators: HashMap::new(),
            held: HashSet::new(),
            timer: 0,
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
        self.sided = None;
        for key in self.registered.drain(..) {
            if let Err(error) = self.manager.unregister(key) {
                tracing::warn!(%error, "Cannot unregister a Windows hotkey");
            }
        }
        self.actions.clear();
        self.accelerators.clear();
        let mut errors = HashMap::new();
        let (sided, bindings): (Vec<_>, Vec<_>) = bindings
            .into_iter()
            .partition(|binding| binding.accel.sided());
        if !sided.is_empty() {
            let ids: Vec<_> = sided.iter().map(|binding| binding.action.clone()).collect();
            match super::sided::Hook::bind(sided, control) {
                Ok(hook) => self.sided = Some(hook),
                Err(error) => {
                    for id in ids {
                        errors.insert(id, error.clone());
                    }
                }
            }
        }
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
                    self.accelerators.insert(key.id(), binding.accel);
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
        // RegisterHotKey's release notification only watches the main key. Stop a hold
        // as soon as any modifier is released or the physical combination changes.
        self.held.retain(|id| {
            let Some(accel) = self.accelerators.get(id) else {
                return false;
            };
            // SAFETY: read-only queries of virtual key states on the hotkey message thread.
            let down = |vk| unsafe { GetAsyncKeyState(vk) } < 0;
            let m = accel.modifiers;
            let active = super::sided::virtual_key(&accel.key).is_some_and(|vk| down(vk as i32))
                && m.ctrl == down(0x11)
                && m.alt == down(0x12)
                && m.shift == down(0x10)
                && m.super_ == (down(0x5b) || down(0x5c));
            if !active && let Some(action) = self.actions.get(id) {
                press(control, action, false);
            }
            active
        });
        self.poll_while_held();
    }

    /// Releases are polled every 20 ms, only while a hotkey is held: idle, the thread sleeps.
    fn poll_while_held(&mut self) {
        if self.held.is_empty() {
            if self.timer != 0 {
                // SAFETY: the timer was created on this thread and has no callback.
                let _ = unsafe { KillTimer(None, self.timer) };
                self.timer = 0;
            }
        } else if self.timer == 0 {
            // SAFETY: a thread timer with no callback; GetMessageW pumps its messages.
            self.timer = unsafe { SetTimer(None, 0, 20, None) };
            if self.timer == 0 {
                tracing::warn!(error = %std::io::Error::last_os_error(), "Cannot poll held hotkeys");
            }
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        for key in &self.registered {
            let _ = self.manager.unregister(*key);
        }
        if self.timer != 0 {
            // SAFETY: the timer was created on this thread and has no callback.
            let _ = unsafe { KillTimer(None, self.timer) };
        }
    }
}

/// Whether Ctrl+Alt+key is AltGr+key that types a character in any installed layout.
pub fn types_with_altgr(accel: &super::accel::Accel) -> bool {
    let m = accel.modifiers;
    if !(m.ctrl && m.alt) || m.super_ || m.alt_side == 1 {
        return false;
    }
    let key = accel.key.as_bytes();
    let vk = match key {
        [byte] if byte.is_ascii_alphanumeric() => u32::from(*byte),
        _ if accel.key == "Backquote" => u32::from(VK_OEM_3.0),
        _ => return false,
    };
    let mut state = [0u8; 256];
    for held in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU] {
        state[usize::from(held.0)] = 0x80;
    }
    if m.shift {
        state[usize::from(VK_SHIFT.0)] = 0x80;
    }
    // SAFETY: a null buffer asks for the count; the second call fills a buffer of that size.
    let layouts = unsafe {
        let count = GetKeyboardLayoutList(None);
        let mut layouts = vec![HKL::default(); usize::try_from(count).unwrap_or(0)];
        let filled = GetKeyboardLayoutList(Some(&mut layouts));
        layouts.truncate(usize::try_from(filled).unwrap_or(0));
        layouts
    };
    layouts.into_iter().any(|layout| {
        let mut text = [0u16; 8];
        // SAFETY: plain buffers owned by this frame; flag 4 leaves the dead-key state alone.
        let written = unsafe {
            let scan = MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(layout));
            ToUnicodeEx(vk, scan, &state, &mut text, 4, Some(layout))
        };
        // Control characters are Ctrl+key itself, not an AltGr character.
        written < 0
            || text[..usize::try_from(written).unwrap_or(0)]
                .iter()
                .any(|&unit| unit >= 0x20)
    })
}

pub fn run(receiver: Receiver<Vec<Binding>>, control: ControlSender<ControlMessage>, waker: Waker) {
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
    // The manager's window gave this thread a message queue: bindings can wake it from now on.
    // SAFETY: no arguments; the ID of the calling thread.
    waker
        .0
        .store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let mut message = MSG::default();
    // Bindings sent before the waker was set are taken before the first wait.
    let mut first = true;
    loop {
        if std::mem::take(&mut first) {
            if let Some(bindings) = receiver.try_iter().last() {
                backend.bind(bindings, &control);
            }
        }
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
