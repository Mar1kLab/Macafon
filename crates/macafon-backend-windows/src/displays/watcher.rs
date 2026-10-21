//! Hidden top-level windows receive display/device broadcasts; message-only windows do not.
use super::DisplayError;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub struct Watcher {
    window: usize,
    thread: Option<std::thread::JoinHandle<()>>,
}
static NEXT_CLASS: AtomicU64 = AtomicU64::new(0);

pub fn watch(on_change: impl Fn() + Send + 'static) -> Result<Watcher, DisplayError> {
    let (ready, receiver) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("macafon-displays".into())
        .spawn(move || {
            // SAFETY: this thread owns the registered class, window and message loop until cleanup.
            let result = unsafe {
                (|| -> windows::core::Result<()> {
                    let instance = GetModuleHandleW(None)?;
                    let class: Vec<_> = format!(
                        "MacafonDisplays{}",
                        NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
                    )
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                    let class = PCWSTR(class.as_ptr());
                    let descriptor = WNDCLASSW {
                        lpfnWndProc: Some(procedure),
                        hInstance: instance.into(),
                        lpszClassName: class,
                        ..Default::default()
                    };
                    if RegisterClassW(&descriptor) == 0 {
                        return Err(windows::core::Error::from_thread());
                    }
                    let window = match CreateWindowExW(
                        WS_EX_TOOLWINDOW,
                        class,
                        w!("Macafon display notifications"),
                        WS_OVERLAPPED,
                        0,
                        0,
                        0,
                        0,
                        None,
                        None,
                        Some(instance.into()),
                        None,
                    ) {
                        Ok(window) => window,
                        Err(error) => {
                            let _ = UnregisterClassW(class, Some(instance.into()));
                            return Err(error);
                        }
                    };
                    if ready.send(Ok(window.0 as usize)).is_ok() {
                        let mut message = MSG::default();
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            if message.hwnd == window
                                && message.message == WM_TIMER
                                && message.wParam.0 == 1
                            {
                                let _ = KillTimer(Some(window), 1);
                                // A caller panic must not cross an extern-system window procedure or
                                // leak the window/class. Notify outside the procedure on this thread.
                                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                    &on_change,
                                ))
                                .is_err()
                                {
                                    tracing::error!("Display change callback panicked");
                                }
                            } else {
                                let _ = TranslateMessage(&message);
                                DispatchMessageW(&message);
                            }
                        }
                    }
                    if IsWindow(Some(window)).as_bool() {
                        let _ = DestroyWindow(window);
                    }
                    let _ = UnregisterClassW(class, Some(instance.into()));
                    Ok(())
                })()
            };
            if let Err(error) = result {
                let _ = ready.send(Err(error));
            }
        })
        .map_err(|e| DisplayError::Rejected(format!("cannot start display watcher: {e}")))?;
    match receiver.recv() {
        Ok(Ok(window)) => Ok(Watcher {
            window,
            thread: Some(thread),
        }),
        result => {
            let _ = thread.join();
            Err(match result {
                Ok(Err(e)) => e.into(),
                _ => DisplayError::Rejected("display watcher stopped during startup".into()),
            })
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: asynchronous close goes to the worker's top-level window, never a system window.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.window as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if let Some(thread) = self.thread.take() {
            // A callback may drop its own watcher; joining itself would deadlock.
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DISPLAYCHANGE | WM_DEVICECHANGE => {
            // SAFETY: this window belongs to the current thread. Replacing timer 1 debounces bursts.
            unsafe {
                SetTimer(Some(window), 1, 300, None);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: quit only this window's worker thread.
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // SAFETY: forward untouched message parameters to the default window procedure.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn displays_watcher_debounces_and_stops() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let watcher = watch(move || {
            let _ = sender.send(());
        })
        .expect("watcher");
        // SAFETY: synthetic messages target only our hidden window, without changing displays.
        unsafe {
            for _ in 0..3 {
                PostMessageW(
                    Some(HWND(watcher.window as *mut _)),
                    WM_DISPLAYCHANGE,
                    WPARAM(0),
                    LPARAM(0),
                )
                .expect("post");
            }
        }
        receiver
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("notification");
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_millis(450))
                .is_err()
        );
        drop(watcher);
    }
}
