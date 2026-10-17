//! Side-specific modifiers use the documented low-level keyboard hook. RegisterHotKey
//! only supports modifier families; unrelated input is always passed through.
use super::{Binding, press};
use crate::host::ControlMessage;
use macafon_backend::ControlSender;
use std::cell::RefCell;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::*;

struct Key {
    binding: Binding,
    vk: u32,
    held: bool,
}
struct State {
    keys: Vec<Key>,
    down: [bool; 256],
    control: ControlSender<ControlMessage>,
}
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

pub struct Hook(HHOOK);
impl Hook {
    pub fn bind(
        bindings: Vec<Binding>,
        control: &ControlSender<ControlMessage>,
    ) -> Result<Self, String> {
        let keys = bindings
            .into_iter()
            .map(|binding| {
                let vk = virtual_key(&binding.accel.key);
                vk.map(|vk| Key {
                    binding,
                    vk,
                    held: false,
                })
                .ok_or_else(|| "Unsupported physical key".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut down = [false; 256];
        for (vk, state) in down.iter_mut().enumerate() {
            // SAFETY: querying a virtual key's state does not retain any pointer.
            *state = unsafe { GetAsyncKeyState(vk as i32) } < 0;
        }
        // SAFETY: callback has static lifetime; the owning thread pumps messages until Drop.
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), None, 0) }
            .map_err(|error| error.to_string())?;
        STATE.with(|state| {
            *state.borrow_mut() = Some(State {
                keys,
                down,
                control: control.clone(),
            })
        });
        Ok(Self(hook))
    }
}
impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: this hook belongs to this thread and is uninstalled once.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
        STATE.with(|state| {
            if let Some(state) = state.borrow_mut().take() {
                for key in state.keys.iter().filter(|key| key.held) {
                    press(&state.control, &key.binding.action, false);
                }
            }
        });
    }
}

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: WH_KEYBOARD_LL provides a KBDLLHOOKSTRUCT for nonnegative codes.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let message = wparam.0 as u32;
        let pressed = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let released = message == WM_KEYUP || message == WM_SYSKEYUP;
        if pressed || released {
            let consumed = STATE.with(|cell| {
                let Ok(mut state) = cell.try_borrow_mut() else {
                    return false;
                };
                let Some(state) = state.as_mut() else {
                    return false;
                };
                let vk = match event.vkCode {
                    0x11 => {
                        if event.flags.0 & LLKHF_EXTENDED.0 != 0 {
                            0xa3
                        } else {
                            0xa2
                        }
                    }
                    0x12 => {
                        if event.flags.0 & LLKHF_EXTENDED.0 != 0 {
                            0xa5
                        } else {
                            0xa4
                        }
                    }
                    0x10 => {
                        if event.scanCode == 0x36 {
                            0xa1
                        } else {
                            0xa0
                        }
                    }
                    other => other,
                };
                let Some(slot) = state.down.get_mut(vk as usize) else {
                    return false;
                };
                *slot = pressed;
                let d = &state.down;
                let ctrl = u8::from(d[0xa2]) | (u8::from(d[0xa3]) << 1);
                let alt = u8::from(d[0xa4]) | (u8::from(d[0xa5]) << 1);
                let mut consumed = false;
                for key in &mut state.keys {
                    let m = key.binding.accel.modifiers;
                    let matches = m.ctrl == (ctrl != 0)
                        && m.alt == (alt != 0)
                        && (m.ctrl_side == 0 || m.ctrl_side == ctrl)
                        && (m.alt_side == 0 || m.alt_side == alt)
                        && m.shift == (d[0xa0] || d[0xa1])
                        && m.super_ == (d[0x5b] || d[0x5c]);
                    if key.held && (!d[key.vk as usize] || !matches) {
                        key.held = false;
                        press(&state.control, &key.binding.action, false);
                        consumed |= vk == key.vk;
                    } else if vk == key.vk && pressed && matches {
                        if !key.held {
                            press(&state.control, &key.binding.action, true);
                        }
                        key.held = true;
                        consumed = true;
                    }
                }
                consumed
            });
            if consumed {
                return LRESULT(1);
            }
        }
    }
    // SAFETY: all unmatched keyboard input continues to the next hook and normal recipient.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub(super) fn virtual_key(key: &str) -> Option<u32> {
    if key.len() == 1 {
        return Some(u32::from(key.as_bytes()[0]));
    }
    if let Some(number) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        return Some(0x6f + number);
    }
    Some(match key {
        "Space" => 0x20,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "PrintScreen" => 0x2c,
        "Insert" => 0x2d,
        "Delete" => 0x2e,
        "Pause" => 0x13,
        "ScrollLock" => 0x91,
        "Backquote" => 0xc0,
        _ => return None,
    })
}
