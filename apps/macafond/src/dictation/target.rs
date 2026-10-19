//! Read-only target/layout capture and local Unicode insertion. Never changes a keyboard layout.
use anyhow::Result;
use macafon_ipc::{DictationGamePolicy, DictationInsert, DictationOutcome, DictationPrefs};
use macafon_stt::protocol::Language;

#[derive(Default)]
pub struct Target {
    pub window: usize,
    pub layout: Option<Language>,
}

pub fn layout_language(lang_id: usize) -> Option<Language> {
    match lang_id & 0x3ff {
        0x09 => Some(Language::En),
        0x22 => Some(Language::Uk),
        0x19 => Some(Language::Ru),
        _ => None,
    }
}

static RESTORES: std::sync::OnceLock<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>> =
    std::sync::OnceLock::new();
#[cfg(windows)]
fn track_restore(thread: std::thread::JoinHandle<()>) {
    let mut threads = RESTORES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut n = 0;
    while n < threads.len() {
        if threads[n].is_finished() {
            let _ = threads.swap_remove(n).join();
        } else {
            n += 1;
        }
    }
    threads.push(thread);
}
pub fn finish_restores() {
    if let Some(threads) = RESTORES.get() {
        let pending = std::mem::take(&mut *threads.lock().unwrap_or_else(|e| e.into_inner()));
        for thread in pending {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
impl Target {
    pub fn capture() -> Self {
        use windows::Win32::UI::{
            Input::KeyboardAndMouse::GetKeyboardLayout, WindowsAndMessaging::*,
        };
        // SAFETY: synchronous read-only window/layout queries with local output buffers.
        unsafe {
            let window = GetForegroundWindow();
            let mut class = [0; 256];
            let count = GetClassNameW(window, &mut class);
            let class = String::from_utf16_lossy(&class[..count.max(0) as usize]);
            let layout =
                if class == "ConsoleWindowClass" || class == "CASCADIA_HOSTING_WINDOW_CLASS" {
                    None
                } else {
                    let layout_window = if class == "ApplicationFrameWindow" {
                        FindWindowExW(
                            Some(window),
                            None,
                            windows::core::w!("Windows.UI.Core.CoreWindow"),
                            None,
                        )
                        .unwrap_or(window)
                    } else {
                        window
                    };
                    let thread = GetWindowThreadProcessId(layout_window, None);
                    layout_language(GetKeyboardLayout(thread).0 as usize)
                };
            Self {
                window: window.0 as usize,
                layout,
            }
        }
    }
}
#[cfg(not(windows))]
impl Target {
    pub fn capture() -> Self {
        Self::default()
    }
}

pub fn game_active() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::*;
        // SAFETY: a read-only Shell state query.
        unsafe { SHQueryUserNotificationState() }.is_ok_and(|state| {
            matches!(
                state,
                QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_BUSY | QUNS_PRESENTATION_MODE
            )
        })
    }
    #[cfg(not(windows))]
    false
}

#[cfg(windows)]
pub fn insert(
    target: &Target,
    text: &str,
    prefs: &DictationPrefs,
    copy: bool,
    game: bool,
) -> Result<DictationOutcome> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    anyhow::ensure!(
        !(game && prefs.fullscreen_games == DictationGamePolicy::Ignore),
        "insertion ignored in full-screen game"
    );
    let text = if prefs.trailing_space {
        format!("{} ", text.trim())
    } else {
        text.trim().into()
    };
    // SAFETY: read-only foreground query; HWND is compared but never dereferenced.
    let copy = copy
        || target.window == 0
        || unsafe { GetForegroundWindow() }.0 as usize != target.window
        || (game && prefs.fullscreen_games == DictationGamePolicy::Copy);
    if !copy && prefs.insert == DictationInsert::Type && type_text(&text) {
        return Ok(DictationOutcome::Inserted);
    }
    let snapshot = clipboard::put(&text, !copy && prefs.insert == DictationInsert::Paste)?;
    if copy || prefs.insert == DictationInsert::Type {
        return Ok(DictationOutcome::Copied);
    }
    if paste() {
        let thread = std::thread::Builder::new()
            .name("dictation-clipboard-restore".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let _ = clipboard::restore(snapshot);
            })?;
        track_restore(thread);
        Ok(DictationOutcome::Inserted)
    } else {
        Ok(DictationOutcome::Copied)
    }
}
#[cfg(not(windows))]
pub fn insert(
    _: &Target,
    _: &str,
    _: &DictationPrefs,
    _: bool,
    _: bool,
) -> Result<DictationOutcome> {
    anyhow::bail!("dictation is available on Windows only")
}

#[cfg(windows)]
fn type_text(text: &str) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    for batch in text.encode_utf16().collect::<Vec<_>>().chunks(64) {
        let mut inputs = Vec::with_capacity(batch.len() * 2);
        for &unit in batch {
            for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wScan: unit,
                            dwFlags: flags,
                            ..Default::default()
                        },
                    },
                });
            }
        }
        // SAFETY: the INPUT array is fully initialized and lives through the call.
        if unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } != inputs.len() as u32
        {
            return false;
        }
    }
    true
}
#[cfg(windows)]
fn paste() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let inputs = [
        (VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        (VIRTUAL_KEY(0x56), KEYBD_EVENT_FLAGS(0)),
        (VIRTUAL_KEY(0x56), KEYEVENTF_KEYUP),
        (VK_CONTROL, KEYEVENTF_KEYUP),
    ]
    .map(|(w_vk, dw_flags)| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: w_vk,
                dwFlags: dw_flags,
                ..Default::default()
            },
        },
    });
    // SAFETY: the INPUT array is initialized and borrowed only during SendInput.
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) == inputs.len() as u32 }
}

#[cfg(windows)]
mod clipboard {
    use super::*;
    use windows::Win32::{
        Foundation::{HANDLE, HGLOBAL},
        System::{DataExchange::*, Memory::*},
    };
    pub struct Snapshot {
        formats: Vec<Copy>,
        sequence: u32,
    }
    struct Copy {
        format: u32,
        handle: usize,
    }
    impl Drop for Copy {
        fn drop(&mut self) {
            if self.handle == 0 {
                return;
            }
            use windows::Win32::Graphics::Gdi::*;
            // SAFETY: these are OleDuplicateData-owned handles, never clipboard-owned ones.
            unsafe {
                let pointer = self.handle as *mut std::ffi::c_void;
                match self.format {
                    2 | 9 | 0x82 => {
                        let _ = DeleteObject(HGDIOBJ(pointer));
                    }
                    14 | 0x8e => {
                        let _ = DeleteEnhMetaFile(Some(HENHMETAFILE(pointer)));
                    }
                    3 | 0x83 => {
                        let global = HGLOBAL(pointer);
                        let data = GlobalLock(global);
                        if !data.is_null() {
                            let _ = DeleteMetaFile((*(data as *const METAFILEPICT)).hMF);
                            let _ = GlobalUnlock(global);
                        }
                        let _ = windows::Win32::Foundation::GlobalFree(Some(global));
                    }
                    _ => {
                        let _ = windows::Win32::Foundation::GlobalFree(Some(HGLOBAL(pointer)));
                    }
                }
            }
        }
    }
    struct Open;
    impl Open {
        fn new() -> Result<Self> {
            let owner = crate::osd::clipboard_owner()
                .ok_or_else(|| anyhow::anyhow!("clipboard owner window unavailable"))?;
            // SAFETY: the persistent daemon popup is a valid owner even while hidden.
            unsafe {
                OpenClipboard(Some(owner))?;
            }
            Ok(Self)
        }
    }
    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: this thread successfully opened the clipboard and closes it once.
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    fn set(format: u32, bytes: &[u8]) -> Result<()> {
        // SAFETY: storage is allocated to the exact size, locked while copying; ownership
        // passes to Windows only after SetClipboardData succeeds.
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
            let data = GlobalLock(memory);
            if data.is_null() {
                let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
                anyhow::bail!("clipboard allocation failed");
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data as *mut u8, bytes.len());
            let _ = GlobalUnlock(memory);
            if let Err(e) = SetClipboardData(format, Some(HANDLE(memory.0))) {
                let _ = windows::Win32::Foundation::GlobalFree(Some(memory));
                return Err(e.into());
            }
        }
        Ok(())
    }
    pub fn put(text: &str, save: bool) -> Result<Snapshot> {
        let _open = Open::new()?;
        let mut formats = Vec::new();
        // SAFETY: clipboard is open; GlobalSize/Lock accept only clipboard-provided handles.
        unsafe {
            if save {
                let mut format = EnumClipboardFormats(0);
                while format != 0 {
                    if let Ok(handle) = GetClipboardData(format) {
                        // OleDuplicateData understands bitmap, palette and metafile formats too.
                        let duplicate = windows::Win32::System::Ole::OleDuplicateData(
                            handle,
                            windows::Win32::System::Ole::CLIPBOARD_FORMAT(format as u16),
                            GMEM_MOVEABLE,
                        );
                        anyhow::ensure!(
                            !duplicate.is_invalid(),
                            "cannot preserve a readable clipboard format"
                        );
                        formats.push(Copy {
                            format,
                            handle: duplicate.0 as usize,
                        });
                    }
                    format = EnumClipboardFormats(format);
                }
            }
            EmptyClipboard()?;
            let bytes: Vec<_> = text
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            set(13, &bytes)?; // CF_UNICODETEXT
            let exclude = RegisterClipboardFormatW(windows::core::w!(
                "ExcludeClipboardContentFromMonitorProcessing"
            ));
            let history =
                RegisterClipboardFormatW(windows::core::w!("CanIncludeInClipboardHistory"));
            let cloud = RegisterClipboardFormatW(windows::core::w!("CanUploadToCloudClipboard"));
            set(exclude, &1u32.to_le_bytes())?;
            set(history, &0u32.to_le_bytes())?;
            set(cloud, &0u32.to_le_bytes())?;
            Ok(Snapshot {
                formats,
                sequence: GetClipboardSequenceNumber(),
            })
        }
    }
    pub fn restore(snapshot: Snapshot) -> Result<()> {
        let _open = Open::new()?;
        // SAFETY: clipboard is held exclusively; the sequence check protects new user content.
        unsafe {
            if GetClipboardSequenceNumber() != snapshot.sequence {
                return Ok(());
            }
            EmptyClipboard()?;
        }
        for mut copy in snapshot.formats {
            // SAFETY: each duplicate is owned until successfully transferred to Windows.
            unsafe {
                SetClipboardData(copy.format, Some(HANDLE(copy.handle as *mut _)))?;
            }
            copy.handle = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primary_language_mapping_handles_sublanguages_and_unknown() {
        assert_eq!(layout_language(0x0409), Some(Language::En));
        assert_eq!(layout_language(0x0809), Some(Language::En));
        assert_eq!(layout_language(0x0422), Some(Language::Uk));
        assert_eq!(layout_language(0x0419), Some(Language::Ru));
        assert_eq!(layout_language(0x0407), None);
    }
}
