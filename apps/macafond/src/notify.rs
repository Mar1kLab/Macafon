//! Desktop notifications (freedesktop `org.freedesktop.Notifications`): the first-close hint
//! of BG-001, sent by the daemon because the UI process has already exited.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// The last OSD notification, replaced by the next one instead of piling up (HK-006).
static OSD: AtomicU32 = AtomicU32::new(0);

/// A short on-screen message after a hotkey changed a preset or a volume (HK-006).
pub fn osd(summary: String) {
    show(summary, String::new(), true, 1500);
}

/// Fire and forget on a helper thread; a missing notification server is not an error.
pub fn send(summary: &'static str, body: &'static str) {
    show(summary.to_owned(), body.to_owned(), false, 6000);
}

fn show(summary: String, body: String, replace: bool, timeout_ms: i32) {
    let spawned = std::thread::Builder::new()
        .name("macafon-notify".into())
        .spawn(move || {
            let result = zbus::blocking::Connection::session().and_then(|bus| {
                bus.call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &(
                        "Macafon",
                        if replace {
                            OSD.load(Ordering::Relaxed)
                        } else {
                            0u32
                        },
                        "macafon",
                        summary.as_str(),
                        body.as_str(),
                        Vec::<&str>::new(),
                        HashMap::<&str, zbus::zvariant::Value>::new(),
                        timeout_ms,
                    ),
                )
                .and_then(|reply| reply.body().deserialize::<u32>())
                .map(|id| {
                    if replace {
                        OSD.store(id, Ordering::Relaxed);
                    }
                })
            });
            if let Err(error) = result {
                tracing::debug!(%error, "Desktop notification not shown");
            }
        });
    if let Err(error) = spawned {
        tracing::debug!(%error, "Cannot start notification thread");
    }
}
