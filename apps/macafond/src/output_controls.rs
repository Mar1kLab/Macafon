//! Optional output hardware control runs outside the daemon control/audio loops.
#[cfg(windows)]
mod windows {
    use crate::server::Hub;
    use macafon_backend_windows::macaron;
    use macafon_ipc::{
        ErrorBody, ErrorCode, Event, MacaronSettings, Outcome, OutputControl, Request,
    };
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    struct Work {
        request: Request,
        reply: mpsc::Sender<Outcome>,
        queued_at: Instant,
    }
    pub struct Manager {
        sender: Option<mpsc::SyncSender<Work>>,
    }

    impl Manager {
        pub fn new(hub: Arc<Hub>) -> Self {
            // Host unit tests must not enumerate hardware or touch the user's DAC cache.
            if cfg!(test) {
                return Self { sender: None };
            }
            let (sender, receiver) = mpsc::sync_channel::<Work>(1);
            let started = std::thread::Builder::new()
                .name("macafon-output-control".into())
                .spawn(move || run(hub, receiver));
            Self {
                sender: started.ok().map(|_| sender),
            }
        }

        pub fn dispatch(&self, request: Request, reply: mpsc::Sender<Outcome>) {
            let work = Work {
                request,
                reply,
                queued_at: Instant::now(),
            };
            if let Some(sender) = &self.sender {
                if let Err(error) = sender.try_send(work) {
                    let (mpsc::TrySendError::Full(work) | mpsc::TrySendError::Disconnected(work)) =
                        error;
                    let _ = work.reply.send(Outcome::Error(ErrorBody::new(
                        ErrorCode::Busy,
                        "output controller is busy",
                    )));
                }
            } else {
                let _ = work.reply.send(Outcome::Error(ErrorBody::new(
                    ErrorCode::Internal,
                    "output controller could not start",
                )));
            }
        }
    }

    fn run(hub: Arc<Hub>, receiver: mpsc::Receiver<Work>) {
        let mut devices = Vec::<macaron::Device>::new();
        let mut state = Vec::<OutputControl>::new();
        let path = macafon_config::state_dir()
            .ok()
            .map(|p| p.join("macaron-last-settings.json"));
        let mut saved: BTreeMap<String, MacaronSettings> = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .filter(|bytes| bytes.len() <= 1_048_576)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        saved.retain(|_, value| value.validate());
        let mut scanned_at = Instant::now() - Duration::from_secs(2);
        let mut dirty_at: Option<Instant> = None;
        let mut volume_read_at = Instant::now() - Duration::from_secs(1);
        loop {
            // Discovery and volume queries never change settings. A HID scan is not free: every
            // 2 s while a window shows levels, else every 10 s (NFR-R7).
            let discovery = if hub.metering() {
                Duration::from_secs(2)
            } else {
                Duration::from_secs(10)
            };
            if scanned_at.elapsed() >= discovery {
                scanned_at = Instant::now();
                match macaron::discover() {
                    Ok(found) => {
                        let next: Vec<_> = found
                            .iter()
                            .map(|device| {
                                let mut item = state
                                    .iter()
                                    .find(|s| s.id == device.id)
                                    .cloned()
                                    .unwrap_or_else(|| OutputControl {
                                        id: device.id.clone(),
                                        name: "iBasso Jr. Macaron".into(),
                                        output: None,
                                        settings: saved
                                            .get(&device.id)
                                            .cloned()
                                            .unwrap_or_default(),
                                        live_volume: false,
                                        error: None,
                                    });
                                item.output = device.output.clone();
                                item
                            })
                            .collect();
                        if next != state {
                            state = next;
                            hub.broadcast(&Event::OutputControlsChanged(state.clone()));
                        }
                        devices = found;
                    }
                    Err(error) => {
                        for item in &mut state {
                            item.error = Some(error.to_string());
                            item.settings = MacaronSettings::default();
                        }
                        devices.clear();
                        hub.broadcast(&Event::OutputControlsChanged(state.clone()));
                    }
                }
            }
            let mut changed = false;
            if volume_read_at.elapsed() >= Duration::from_millis(250) {
                volume_read_at = Instant::now();
                for item in &mut state {
                    let Some(device) = devices.iter().find(|d| d.id == item.id) else {
                        continue;
                    };
                    let previous = item.clone();
                    match macaron::read_volume(&device.path) {
                        Ok((volume, balance)) => {
                            item.settings.volume = Some(volume);
                            item.settings.balance = Some(balance);
                            item.live_volume = true;
                            if item
                                .error
                                .as_deref()
                                .is_some_and(|s| s.starts_with("Volume: "))
                            {
                                item.error = None;
                            }
                            if *item != previous {
                                saved.insert(item.id.clone(), item.settings.clone());
                                dirty_at = Some(Instant::now());
                            }
                        }
                        Err(error) => {
                            item.live_volume = false;
                            item.error = Some(format!("Volume: {error}"));
                        }
                    }
                    changed |= *item != previous;
                }
            }
            if changed {
                hub.broadcast(&Event::OutputControlsChanged(state.clone()));
            }
            if dirty_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(500)) {
                if let Err(error) = save(path.as_deref(), &saved) {
                    tracing::warn!(%error, "Cannot save last DAC settings");
                    for item in &mut state {
                        item.error = Some(format!("Cannot save last device settings: {error}"));
                    }
                    hub.broadcast(&Event::OutputControlsChanged(state.clone()));
                }
                dirty_at = None;
            }
            // Without a device there are no volumes to follow: wake for commands or the scan.
            let wait = if devices.is_empty() && dirty_at.is_none() {
                Duration::from_secs(1)
            } else {
                Duration::from_millis(100)
            };
            match receiver.recv_timeout(wait) {
                Ok(work) => {
                    if work.queued_at.elapsed() >= Duration::from_secs(2) {
                        let _ = work.reply.send(Outcome::Error(ErrorBody::new(
                            ErrorCode::Busy,
                            "output command expired",
                        )));
                        continue;
                    }
                    let result = match work.request {
                        Request::OutputControlsList => Ok(serde_json::json!(state)),
                        Request::OutputControlsSet { id, settings } => {
                            let result = apply(&devices, &mut state, &id, settings);
                            if let Some(item) = state.iter().find(|s| s.id == id) {
                                saved.insert(id.clone(), item.settings.clone());
                                if let Err(error) = save(path.as_deref(), &saved) {
                                    tracing::warn!(%error, "Cannot save last DAC settings");
                                    if let Some(item) = state.iter_mut().find(|s| s.id == id) {
                                        item.error = Some(format!(
                                            "Cannot save last device settings: {error}"
                                        ));
                                    }
                                }
                            }
                            result.map(|()| {
                                hub.broadcast(&Event::OutputControlsChanged(state.clone()));
                                serde_json::json!(state)
                            })
                        }
                        _ => Err(ErrorBody::new(
                            ErrorCode::BadRequest,
                            "not an output control request",
                        )),
                    };
                    // Failed partial writes invalidate the entire last-written state too.
                    if result.is_err() {
                        hub.broadcast(&Event::OutputControlsChanged(state.clone()));
                    }
                    let _ = work.reply.send(match result {
                        Ok(value) => Outcome::Result(value),
                        Err(error) => Outcome::Error(error),
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = save(path.as_deref(), &saved);
                    break;
                }
            }
        }
    }

    fn save(
        path: Option<&std::path::Path>,
        settings: &BTreeMap<String, MacaronSettings>,
    ) -> std::io::Result<()> {
        let path = path.ok_or_else(|| std::io::Error::other("state directory unavailable"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary)?;
        serde_json::to_writer(&mut file, settings)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, path)
    }

    fn apply(
        devices: &[macaron::Device],
        state: &mut [OutputControl],
        id: &str,
        patch: MacaronSettings,
    ) -> Result<(), ErrorBody> {
        if !patch.validate() || patch.is_empty() {
            return Err(ErrorBody::new(
                ErrorCode::InvalidParams,
                "invalid Macaron settings",
            ));
        }
        let device = devices
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "Macaron disconnected"))?;
        let item = state
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "Macaron disconnected"))?;
        let mut next = item.settings.clone();
        let mut reports = Vec::new();
        if patch.volume.is_some() || patch.balance.is_some() {
            // Read the unedited half from the DAC, never from a possibly stale saved value.
            let (current_volume, current_balance) = macaron::read_volume(&device.path)
                .map_err(|e| ErrorBody::new(ErrorCode::Internal, e.to_string()))?;
            next.volume = Some(current_volume);
            next.balance = Some(current_balance);
            let volume = patch.volume.or(next.volume).ok_or_else(|| {
                ErrorBody::new(
                    ErrorCode::InvalidParams,
                    "set volume and balance together while either is unknown",
                )
            })?;
            let balance = patch.balance.or(next.balance).ok_or_else(|| {
                ErrorBody::new(
                    ErrorCode::InvalidParams,
                    "set volume and balance together while either is unknown",
                )
            })?;
            reports.extend(macaron::volume_reports(volume, balance).ok_or_else(|| {
                ErrorBody::new(ErrorCode::InvalidParams, "invalid volume/balance")
            })?);
            next.volume = Some(volume);
            next.balance = Some(balance);
        }
        reports.extend(
            macaron::setting_reports(patch.gain, patch.filter, patch.output_mode)
                .ok_or_else(|| ErrorBody::new(ErrorCode::InvalidParams, "invalid device mode"))?,
        );
        if let Some(v) = patch.gain {
            next.gain = Some(v);
        }
        if let Some(v) = patch.filter {
            next.filter = Some(v);
        }
        if let Some(v) = patch.output_mode {
            next.output_mode = Some(v);
        }
        if let Err(error) = macaron::send(&device.path, &reports) {
            item.settings = MacaronSettings::default();
            item.live_volume = false;
            item.error = Some(error.to_string());
            return Err(ErrorBody::new(ErrorCode::Internal, error.to_string()));
        }
        if patch.volume.is_some() || patch.balance.is_some() {
            let (volume, balance) = macaron::read_volume(&device.path)
                .map_err(|e| ErrorBody::new(ErrorCode::Internal, e.to_string()))?;
            if next.volume != Some(volume) {
                item.settings.volume = Some(volume);
                item.settings.balance = Some(balance);
                return Err(ErrorBody::new(
                    ErrorCode::Internal,
                    "Macaron volume write did not match readback",
                ));
            }
            next.volume = Some(volume);
            next.balance = Some(balance);
            item.live_volume = true;
        }
        item.settings = next;
        item.error = None;
        Ok(())
    }
}
#[cfg(windows)]
pub use windows::Manager;
