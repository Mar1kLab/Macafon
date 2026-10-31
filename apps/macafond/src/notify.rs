//! Desktop notifications (freedesktop `org.freedesktop.Notifications`): the first-close hint
//! of BG-001, sent by the daemon because the UI process has already exited.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU32, Ordering};

/// The last OSD notification, replaced by the next one instead of piling up (HK-006).
#[cfg(target_os = "linux")]
static OSD: AtomicU32 = AtomicU32::new(0);

/// A short on-screen message after a hotkey changed a preset or a volume (HK-006); Windows
/// shows its own popup instead (`crate::osd`).
#[cfg(not(windows))]
pub fn osd(summary: String) {
    show(summary, String::new(), true, 1500);
}

/// Fire and forget on a helper thread; a missing notification server is not an error.
pub fn send(summary: &'static str, body: &'static str) {
    show(summary.to_owned(), body.to_owned(), false, 6000);
}

/// A tray balloon needs no installed shortcut or AppUserModelID.
#[cfg(not(target_os = "linux"))]
fn show(summary: String, body: String, _replace: bool, _timeout_ms: i32) {
    #[cfg(windows)]
    if crate::tray::windows_tray::notify(summary.clone(), body.clone(), _replace) {
        return;
    }
    tracing::info!(%summary, %body, "Notification");
}

#[cfg(target_os = "linux")]
fn show(summary: String, body: String, replace: bool, timeout_ms: i32) {
    let spawned = std::thread::Builder::new()
        .name("macafon-notify".into())
        .spawn(move || {
            let result = deliver(&summary, &body, replace, timeout_ms, &OSD);
            if let Err(error) = result {
                tracing::debug!(%error, "Desktop notification not shown");
            }
        });
    if let Err(error) = spawned {
        tracing::debug!(%error, "Cannot start notification thread");
    }
}

#[cfg(target_os = "linux")]
fn deliver(
    summary: &str,
    body: &str,
    replace: bool,
    timeout_ms: i32,
    slot: &AtomicU32,
) -> zbus::Result<()> {
    zbus::blocking::Connection::session().and_then(|bus| {
        bus.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Macafon",
                if replace {
                    slot.load(Ordering::Relaxed)
                } else {
                    0u32
                },
                "macafon",
                summary,
                body,
                Vec::<&str>::new(),
                HashMap::<&str, zbus::zvariant::Value>::new(),
                timeout_ms,
            ),
        )
        .and_then(|reply| reply.body().deserialize::<u32>())
        .map(|id| {
            if replace {
                slot.store(id, Ordering::Relaxed);
            }
        })
    })
}

/// A single ordered sender replaces the dictation notification; audio-level ticks never
/// create D-Bus traffic. No recognized text is included in the notification.
#[cfg(target_os = "linux")]
pub fn dictation(view: &crate::osd::dictation::View) {
    use crate::osd::dictation::State;
    use std::sync::{OnceLock, mpsc};
    static SENDER: OnceLock<mpsc::Sender<(&'static str, i32)>> = OnceLock::new();
    let (label, timeout) = match view.state {
        State::Listening if view.elapsed_s >= 270 => ("Too long — recording stops at 5 minutes", 0),
        State::Listening => ("Listening", 0),
        State::Transcribing => ("Transcribing", 0),
        State::Done => ("Done", 1500),
        State::Copied => ("Copied — press Ctrl+V", 4000),
        State::NoSpeech => ("No speech", 2000),
        State::Error => ("Dictation error — check Macafon", 4000),
        State::Cancelled => ("Cancelled", 1500),
    };
    let sender = SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<(&'static str, i32)>();
        let _ = std::thread::Builder::new()
            .name("dictation-notify".into())
            .spawn(move || {
                let slot = AtomicU32::new(0);
                let mut previous = None;
                while let Ok((label, timeout)) = receiver.recv() {
                    if timeout == 0 && previous == Some(label) {
                        continue;
                    }
                    previous = Some(label);
                    if let Err(error) = deliver("Macafon dictation", label, true, timeout, &slot) {
                        tracing::debug!(%error, "Dictation notification not shown");
                    }
                }
            });
        sender
    });
    let _ = sender.send((label, timeout));
}
