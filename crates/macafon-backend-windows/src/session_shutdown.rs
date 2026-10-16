//! A top-level (not message-only) window receives session broadcasts even without a console.
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

pub(super) struct Listener {
    window: usize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Listener {
    pub(super) fn spawn() -> windows::core::Result<Self> {
        let (ready, receiver) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("macafon-session".into())
            .spawn(move || {
                // SAFETY: this worker owns its window, class and message loop for their whole lifetime.
                unsafe {
                    let result = (|| {
                        let instance = GetModuleHandleW(None)?;
                        let class = w!("MacafonSessionShutdown");
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
                            w!("Macafon session shutdown"),
                            WS_OVERLAPPED,
                            0,
                            0,
                            0,
                            0,
                            None,
                            None,
                            Some(instance.into()),
                            None,
                        );
                        let window = match window {
                            Ok(window) => window,
                            Err(error) => {
                                let _ = UnregisterClassW(class, Some(instance.into()));
                                return Err(error);
                            }
                        };
                        let _ = ready.send(Ok(window.0 as usize));
                        let mut message = MSG::default();
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                        if IsWindow(Some(window)).as_bool() {
                            let _ = DestroyWindow(window);
                        }
                        let _ = UnregisterClassW(class, Some(instance.into()));
                        Ok(())
                    })();
                    if let Err(error) = result {
                        let _ = ready.send(Err(error));
                    }
                }
            })
            .map_err(|_| windows::core::Error::from_thread())?;
        let window = receiver
            .recv()
            .map_err(|_| windows::core::Error::from_thread())??;
        Ok(Self {
            window,
            thread: Some(thread),
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // SAFETY: post to the worker's live window; cleanup has finished before joining it.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.window as *mut _)),
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
    match message {
        // Do not stop on QUERY: another application can cancel logoff.
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION if wparam.0 != 0 => {
            super::wait_for_cleanup();
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: called on the window's owning thread.
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // SAFETY: unhandled messages retain the window manager's default behavior.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[test]
    fn session_end_waits_for_cleanup_but_cancelled_logoff_does_not_stop() {
        super::super::STOP.store(false, Ordering::Release);
        super::super::FINISHED.store(false, Ordering::Release);
        let listener = Listener::spawn().expect("hidden shutdown window");
        let window = HWND(listener.window as *mut _);
        // SAFETY: synchronous messages to our live test window, without ending the real session.
        unsafe {
            assert_eq!(SendMessageW(window, WM_QUERYENDSESSION, None, None).0, 1);
            SendMessageW(window, WM_ENDSESSION, Some(WPARAM(0)), None);
        }
        assert!(!super::super::STOP.load(Ordering::Acquire));
        let cleanup = std::thread::spawn(|| {
            while !super::super::STOP.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            std::thread::sleep(Duration::from_millis(60));
            super::super::FINISHED.store(true, Ordering::Release);
        });
        let started = Instant::now();
        // SAFETY: synthetic end-session notification targets only our test window.
        unsafe {
            SendMessageW(window, WM_ENDSESSION, Some(WPARAM(1)), None);
        }
        assert!(super::super::FINISHED.load(Ordering::Acquire));
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < super::super::SHUTDOWN_GRACE);
        cleanup.join().expect("simulated cleanup thread");
        drop(listener);
        super::super::STOP.store(false, Ordering::Release);
    }
}
