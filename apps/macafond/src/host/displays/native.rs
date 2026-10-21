//! The native confirmation runs on its own thread; only the control loop decides outcomes.
use crate::host::ControlMessage;
use macafon_backend::ControlSender;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::{Controls::*, WindowsAndMessaging::*};
use windows::core::{HRESULT, w};

struct Context {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}

unsafe extern "system" fn callback(
    window: HWND,
    message: TASKDIALOG_NOTIFICATIONS,
    _wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: TaskDialogIndirect is synchronous on this thread; Context outlives all callbacks.
    let context = unsafe { &*(data as *const Context) };
    // SAFETY: window is the live dialog passed by Windows. Text is copied synchronously.
    unsafe {
        if message == TDN_CREATED {
            let mut rect = RECT::default();
            if GetWindowRect(window, &mut rect).is_ok() {
                let _ = SetWindowPos(
                    window,
                    Some(HWND_TOPMOST),
                    (GetSystemMetrics(SM_CXSCREEN) - (rect.right - rect.left)) / 2,
                    (GetSystemMetrics(SM_CYSCREEN) - (rect.bottom - rect.top)) / 2,
                    0,
                    0,
                    SWP_NOSIZE,
                );
            }
        }
        if message == TDN_TIMER || message == TDN_CREATED {
            if context.cancelled.load(Ordering::Acquire) || Instant::now() >= context.deadline {
                SendMessageW(
                    window,
                    TDM_CLICK_BUTTON.0 as u32,
                    Some(WPARAM(101)),
                    Some(LPARAM(0)),
                );
            } else {
                let seconds = context
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .div_ceil(1000);
                let text: Vec<u16> = format!("Reverting in {seconds} s")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                SendMessageW(
                    window,
                    TDM_SET_ELEMENT_TEXT.0 as u32,
                    Some(WPARAM(TDE_CONTENT.0 as usize)),
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
        }
    }
    HRESULT(0)
}

pub(super) fn show(
    control: ControlSender<ControlMessage>,
    generation: u64,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) {
    let failed_control = control.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("macafon-display-confirm".into())
        .spawn(move || {
            let context = Context {
                deadline,
                cancelled,
            };
            if context.cancelled.load(Ordering::Acquire) {
                return;
            }
            let buttons = [
                TASKDIALOG_BUTTON {
                    nButtonID: 100,
                    pszButtonText: w!("Keep"),
                },
                TASKDIALOG_BUTTON {
                    nButtonID: 101,
                    pszButtonText: w!("Revert"),
                },
            ];
            let config = TASKDIALOGCONFIG {
                cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
                dwFlags: TDF_CALLBACK_TIMER | TDF_ALLOW_DIALOG_CANCELLATION,
                pszWindowTitle: w!("Macafon"),
                pszMainInstruction: w!("Keep these display settings?"),
                pszContent: w!("Reverting in 15 s"),
                cButtons: buttons.len() as u32,
                pButtons: buttons.as_ptr(),
                nDefaultButton: 101,
                pfCallback: Some(callback),
                lpCallbackData: &context as *const Context as isize,
                ..Default::default()
            };
            let mut button = 0;
            // SAFETY: config, button labels and callback context live until this synchronous call returns.
            let result = unsafe { TaskDialogIndirect(&config, Some(&mut button), None, None) };
            if let Err(error) = &result {
                tracing::warn!(%error, "Cannot show display confirmation");
            }
            if !context.cancelled.load(Ordering::Acquire) {
                let _ = control.send(ControlMessage::DisplayDecision {
                    generation,
                    keep: result.is_ok() && button == 100 && Instant::now() < deadline,
                });
            }
        })
    {
        tracing::warn!(%error, "Cannot start display confirmation");
        let _ = failed_control.send(ControlMessage::DisplayDecision {
            generation,
            keep: false,
        });
    }
}
