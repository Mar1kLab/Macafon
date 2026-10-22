//! Win32 tray: Shell and icons are owned by a dedicated message-loop thread.
use super::{TrayState, icon};
use crate::host::ControlMessage;
use macafon_backend::ControlSender;
use macafon_ipc::Request;
use std::sync::{OnceLock, mpsc};
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::{Shell::*, WindowsAndMessaging::*};
use windows::core::w;

const CALLBACK: u32 = WM_APP + 1;
/// `CALLBACK` re-posted for the loop under its own id, so a posted copy is never re-posted.
const QUEUED: u32 = WM_APP + 2;
/// Posted after every queued update: the loop sleeps until something arrives.
const WAKE: u32 = WM_APP + 3;
/// Icon animation frames; the timer runs only while the icon moves.
const FRAME_TIMER: usize = 1;
static NOTIFICATIONS: OnceLock<Updates> = OnceLock::new();

/// Queue for the tray thread that also wakes its message loop.
#[derive(Clone)]
struct Updates {
    sender: mpsc::Sender<Update>,
    window: usize,
}
impl Updates {
    fn send(&self, update: Update) -> bool {
        if self.sender.send(update).is_err() {
            return false;
        }
        // SAFETY: posts to the worker's window without borrowing its memory.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.window as *mut _)),
                WAKE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        true
    }
}
enum Update {
    State(TrayState),
    Balloon {
        title: String,
        body: String,
        replace: bool,
    },
}
pub fn notify(title: String, body: String, replace: bool) -> bool {
    NOTIFICATIONS.get().is_some_and(|updates| {
        updates.send(Update::Balloon {
            title,
            body,
            replace,
        })
    })
}

pub struct Tray {
    updates: Updates,
    thread: Option<std::thread::JoinHandle<()>>,
    shown: TrayState,
}
impl Tray {
    pub fn spawn(state: TrayState, control: ControlSender<ControlMessage>) -> Option<Self> {
        let (updates, receiver) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let initial = state.clone();
        let thread = std::thread::Builder::new()
            .name("macafon-tray".into())
            .spawn(move || {
                if let Err(error) = run(initial, control, receiver, ready) {
                    tracing::warn!(%error, "Tray icon unavailable");
                }
            })
            .ok()?;
        let window = started.recv().ok()?;
        let updates = Updates {
            sender: updates,
            window,
        };
        let _ = NOTIFICATIONS.set(updates.clone());
        Some(Self {
            updates,
            thread: Some(thread),
            shown: state,
        })
    }
    pub fn update(&mut self, state: TrayState) {
        if state != self.shown {
            self.shown = state.clone();
            self.updates.send(Update::State(state));
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: posts to the worker's window without borrowing its memory.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.updates.window as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: window-manager arguments; no application pointers are used.
    unsafe {
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        // Explorer sends the icon callbacks instead of posting them: queue them for the loop,
        // which owns the state and launches the UI outside Explorer's send.
        if message == CALLBACK {
            let _ = PostMessageW(Some(window), QUEUED, wparam, lparam);
            return LRESULT(0);
        }
        // Queued callbacks belong to the outer loop; never re-post a dispatched copy.
        if message == QUEUED {
            return LRESULT(0);
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}
fn copy_text<const N: usize>(destination: &mut [u16; N], text: &str) {
    let mut end = 0;
    for character in text.chars().filter(|&character| character != '\0') {
        let mut units = [0; 2];
        let encoded = character.encode_utf16(&mut units);
        if end + encoded.len() >= N {
            break;
        }
        destination[end..end + encoded.len()].copy_from_slice(encoded);
        end += encoded.len();
    }
    destination[end] = 0;
}
struct Owner {
    window: HWND,
    data: NOTIFYICONDATAW,
    icon: HICON,
}
impl Owner {
    fn refresh(&mut self, state: &TrayState, bend: f32, add: bool) -> windows::core::Result<()> {
        let pixmap = icon::render(state, bend).remove(2);
        let size = pixmap.width;
        debug_assert_eq!(size, pixmap.height);
        // RT_ICON: bottom-up 32-bit DIB followed by its monochrome AND mask.
        let mut resource = Vec::new();
        resource.extend(40u32.to_le_bytes());
        resource.extend(size.to_le_bytes());
        resource.extend((size * 2).to_le_bytes());
        resource.extend(1u16.to_le_bytes());
        resource.extend(32u16.to_le_bytes());
        resource.extend([0; 24]);
        for row in pixmap.data.chunks_exact(size as usize * 4).rev() {
            for pixel in row.chunks_exact(4) {
                resource.extend([pixel[3], pixel[2], pixel[1], pixel[0]]);
            }
        }
        resource.resize(
            resource.len() + (size as usize).div_ceil(32) * 4 * size as usize,
            0,
        );
        // SAFETY: complete DIB and mask; Windows copies the bytes.
        let next = unsafe {
            CreateIconFromResourceEx(&resource, true, 0x00030000, size, size, LR_DEFAULTCOLOR)?
        };
        self.data.hIcon = next;
        self.data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        self.data.szTip.fill(0);
        let preset = state
            .presets
            .iter()
            .find(|preset| Some(&preset.id) == state.preset.as_ref())
            .map_or("Custom", |preset| preset.name.as_str());
        copy_text(
            &mut self.data.szTip,
            &format!(
                "Macafon\nPreset: {preset}{}{}{}{}",
                if state.preset_modified {
                    " (modified)"
                } else {
                    ""
                },
                if !state.connected {
                    "\nMicrophone disconnected"
                } else {
                    ""
                },
                if state.muted {
                    "\nMicrophone muted"
                } else {
                    ""
                },
                if state.bypass {
                    "\nProcessing bypassed"
                } else {
                    ""
                }
            ),
        );
        // SAFETY: handles and buffers belong to this thread and are live throughout the call.
        let success =
            unsafe { Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &self.data) }
                .as_bool();
        // SAFETY: Shell copies the icon; release our previous owned handle exactly once.
        unsafe {
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
        }
        self.icon = next;
        if success {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }
    fn balloon(&self, title: &str, body: &str, replace: bool) {
        let mut data = NOTIFYICONDATAW {
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..self.data
        };
        data.szInfo.fill(0);
        data.szInfoTitle.fill(0);
        // SAFETY: Shell copies the buffers; empty text dismisses the previous balloon.
        unsafe {
            if replace {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
            copy_text(&mut data.szInfoTitle, title);
            copy_text(&mut data.szInfo, if body.is_empty() { title } else { body });
            let shown = Shell_NotifyIconW(NIM_MODIFY, &data).as_bool();
            tracing::debug!(%title, shown, replace, "Tray notification");
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: resources are owned by this thread; remove Shell's icon before releasing it.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
            if IsWindow(Some(self.window)).as_bool() {
                let _ = DestroyWindow(self.window);
            }
        }
    }
}
fn run(
    mut state: TrayState,
    control: ControlSender<ControlMessage>,
    updates: mpsc::Receiver<Update>,
    ready: mpsc::SyncSender<usize>,
) -> windows::core::Result<()> {
    // SAFETY: this thread owns its hidden window, class, timer and message loop.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("MacafonTray");
        let descriptor = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&descriptor) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("Macafon tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let mut owner = Owner {
            window,
            icon: HICON::default(),
            data: NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: window,
                uID: 1,
                uCallbackMessage: CALLBACK,
                ..Default::default()
            },
        };
        let mut bend = icon::target(&state);
        owner.refresh(&state, bend, true)?;
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        tracing::info!("Windows tray icon ready");
        let _ = ready.send(window.0 as usize);
        let mut from = bend;
        let mut target = bend;
        let mut started = Instant::now();
        let mut message = MSG::default();
        loop {
            if GetMessageW(&mut message, None, 0, 0).0 <= 0 {
                break;
            }
            if message.message == WAKE || message.message == WM_TIMER {
                let mut changed = false;
                let mut osd = None;
                while let Ok(update) = updates.try_recv() {
                    match update {
                        Update::State(next) => {
                            state = next;
                            changed = true;
                        }
                        Update::Balloon {
                            title,
                            body,
                            replace: true,
                        } => osd = Some((title, body)),
                        Update::Balloon {
                            title,
                            body,
                            replace: false,
                        } => owner.balloon(&title, &body, false),
                    }
                }
                if let Some((title, body)) = osd {
                    owner.balloon(&title, &body, true);
                }
                if changed {
                    from = bend;
                    target = icon::target(&state);
                    started = Instant::now();
                    if bend != target && SetTimer(Some(window), FRAME_TIMER, 25, None) == 0 {
                        bend = target; // No frames: jump to the end.
                    }
                }
                let animating = bend != target;
                if animating {
                    let progress = (started.elapsed().as_secs_f32() / 0.3).min(1.0);
                    bend = if progress == 1.0 {
                        target
                    } else {
                        from + (target - from)
                            * (0.5 - 0.5 * (std::f32::consts::PI * progress).cos())
                    };
                }
                if changed || animating {
                    let _ = owner.refresh(&state, bend, false);
                }
                if bend == target {
                    let _ = KillTimer(Some(window), FRAME_TIMER);
                }
            } else if message.message == taskbar_created {
                let _ = owner.refresh(&state, bend, true);
            } else if message.message == QUEUED {
                match message.lParam.0 as u32 {
                    WM_LBUTTONUP => crate::ui::open(),
                    WM_MBUTTONUP => send(&control, Request::MicToggleMute),
                    // Keep the compact panel available from the native profile menu.
                    WM_RBUTTONUP => {
                        if let Err(error) = display_menu(window, &state, &control) {
                            tracing::warn!(%error, "Cannot open display profile menu");
                            crate::ui::quick_panel();
                        }
                    }
                    _ => {}
                }
            } else {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        drop(owner);
        let _ = UnregisterClassW(class, Some(instance.into()));
    }
    Ok(())
}
fn send(control: &ControlSender<ControlMessage>, request: Request) {
    let (reply, _) = mpsc::channel();
    let _ = control.send(ControlMessage::Request { request, reply });
}

/// Native profile actions and the existing compact panel share the tray menu.
fn display_menu(
    window: HWND,
    state: &TrayState,
    control: &ControlSender<ControlMessage>,
) -> windows::core::Result<()> {
    use windows::Win32::Foundation::POINT;
    use windows::core::PCWSTR;
    // SAFETY: menus are owned here, used synchronously and destroyed after TrackPopupMenu.
    unsafe {
        let menu = CreatePopupMenu()?;
        let result = (|| {
            AppendMenuW(menu, MF_STRING, 1, w!("Open Macafon"))?;
            AppendMenuW(menu, MF_STRING, 3, w!("Quick panel"))?;
            if !state.displays.is_empty()
                || macafon_ipc::Platform::current().capabilities().displays
            {
                let profiles = CreatePopupMenu()?;
                // Attach immediately: destroying the parent also destroys this submenu.
                if let Err(error) =
                    AppendMenuW(menu, MF_POPUP, profiles.0 as usize, w!("Display profile"))
                {
                    let _ = DestroyMenu(profiles);
                    return Err(error);
                }
                for (index, (id, name)) in state.displays.iter().enumerate() {
                    let text: Vec<u16> = name
                        .replace('&', "&&")
                        .encode_utf16()
                        .chain(Some(0))
                        .collect();
                    let checked = if state.active_display.as_ref() == Some(id) {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                    AppendMenuW(
                        profiles,
                        MF_STRING | checked,
                        index + 100,
                        PCWSTR(text.as_ptr()),
                    )?;
                }
                if !state.displays.is_empty() {
                    AppendMenuW(profiles, MF_SEPARATOR, 0, PCWSTR::null())?;
                }
                AppendMenuW(profiles, MF_STRING, 2, w!("All monitors on"))?;
            }
            let mut point = POINT::default();
            GetCursorPos(&mut point)?;
            let _ = SetForegroundWindow(window);
            let selected = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                window,
                None,
            )
            .0;
            match selected {
                1 => crate::ui::open(),
                2 => send(control, Request::DisplaysAllOn),
                3 => crate::ui::quick_panel(),
                id if id >= 100 => {
                    if let Some((id, _)) = state.displays.get((id - 100) as usize) {
                        send(control, Request::DisplaysApply { id: id.clone() });
                    }
                }
                _ => {}
            }
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            Ok(())
        })();
        let _ = DestroyMenu(menu);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_text_is_terminated_without_splitting_unicode() {
        let mut text = [0; 4];
        copy_text(&mut text, "a\u{1f399}b");
        assert_eq!(
            String::from_utf16(&text[..3]).ok().as_deref(),
            Some("a\u{1f399}")
        );
        assert_eq!(text[3], 0);
        let mut short = [0; 2];
        copy_text(&mut short, "\u{1f399}");
        assert_eq!(short, [0, 0]);
    }
}
