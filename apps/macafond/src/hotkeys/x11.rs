//! X11 key grabs through `global-hotkey` (HK-003). Used only in X11 sessions: on Wayland an
//! XWayland grab would not see keys pressed in native windows.

use super::accel::{Accel, hotkey};
use global_hotkey::{GlobalHotKeyManager, hotkey::HotKey};
use std::collections::HashMap;

/// An X11 session with a reachable local display.
pub fn available() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind == "wayland");
    let Some(display) = std::env::var("DISPLAY").ok().filter(|_| !wayland) else {
        return false;
    };
    let number = display
        .strip_prefix(':')
        .and_then(|rest| rest.split('.').next())
        .and_then(|n| n.parse::<u32>().ok());
    number.is_some_and(|n| {
        std::os::unix::net::UnixStream::connect(format!("/tmp/.X11-unix/X{n}")).is_ok()
    })
}

pub struct X11 {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
}

impl X11 {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            manager: GlobalHotKeyManager::new().map_err(|error| error.to_string())?,
            registered: Vec::new(),
        })
    }

    /// Replace all grabs; returns the hotkey id of each action and the errors of the rest.
    pub fn bind(
        &mut self,
        bindings: &[(String, Accel)],
    ) -> (HashMap<u32, String>, HashMap<String, String>) {
        let _ = self.manager.unregister_all(&self.registered);
        self.registered.clear();
        let mut ids = HashMap::new();
        let mut errors = HashMap::new();
        for (action, accel) in bindings {
            let result = hotkey(accel).and_then(|key| {
                self.manager
                    .register(key)
                    .map(|()| key)
                    .map_err(|error| error.to_string())
            });
            match result {
                Ok(key) => {
                    ids.insert(key.id(), action.clone());
                    self.registered.push(key);
                }
                Err(error) => {
                    errors.insert(action.clone(), error);
                }
            }
        }
        (ids, errors)
    }
}
