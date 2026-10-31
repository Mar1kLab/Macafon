//! Freedesktop actions when the window is closed; the control loop owns the deadline.
use crate::host::ControlMessage;
use macafon_backend::ControlSender;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

pub(super) fn show(
    control: ControlSender<ControlMessage>,
    generation: u64,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) {
    let spawned = std::thread::Builder::new().name("macafon-display-confirm".into()).spawn(move || {
        if let Err(error) = notification(control,generation,deadline,cancelled) {
            tracing::debug!(%error,"Display confirmation notification unavailable; deadline still applies");
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(%error,"Cannot start display confirmation notification");
    }
}
fn notification(
    control: ControlSender<ControlMessage>,
    generation: u64,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> zbus::Result<()> {
    let bus = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(1))
        .build()?;
    let proxy = zbus::blocking::Proxy::new(
        &bus,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )?;
    // Subscribe before Notify, so a quick click cannot be missed.
    let signals = proxy.receive_signal("ActionInvoked")?;
    let id = Arc::new(AtomicU32::new(0));
    let action_id = id.clone();
    let (actions, receiver) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("macafon-display-actions".into())
        .spawn(move || {
            for signal in signals {
                if let Ok((notification, action)) = signal.body().deserialize::<(u32, String)>()
                    && notification == action_id.load(Ordering::Acquire)
                    && matches!(action.as_str(), "keep" | "revert")
                {
                    let _ = actions.send(action == "keep");
                }
            }
        })
        .map_err(zbus::Error::from)?;
    let result = (|| {
        let mut shown_seconds = 0;
        while Instant::now() < deadline && !cancelled.load(Ordering::Acquire) {
            let seconds = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .div_ceil(1000);
            if seconds != shown_seconds {
                let notification: u32 = proxy.call(
                    "Notify",
                    &(
                        "Macafon",
                        id.load(Ordering::Acquire),
                        "macafon",
                        "Keep these display settings?",
                        format!("Reverting in {seconds} s"),
                        vec!["keep", "Keep", "revert", "Revert"],
                        HashMap::<&str, zbus::zvariant::Value>::new(),
                        (seconds * 1000) as i32,
                    ),
                )?;
                id.store(notification, Ordering::Release);
                shown_seconds = seconds;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(keep) => {
                    if !cancelled.load(Ordering::Acquire) {
                        let _ = control.send(ControlMessage::DisplayDecision {
                            generation,
                            keep: keep && Instant::now() < deadline,
                        });
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(())
    })();
    let notification = id.load(Ordering::Acquire);
    if notification != 0 {
        let _ = proxy.call::<_, _, ()>("CloseNotification", &(notification,));
    }
    drop(proxy);
    let _ = bus.close();
    let _ = worker.join();
    result
}
