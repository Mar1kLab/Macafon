//! Soundpad monitor (PRD §7.6): one WASAPI stream that plays the soundpad into the user's
//! headphones, next to the sound going into the cable.

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{ProAudioThread, RenderStream};
use macafon_backend::SoundpadTap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

const WAIT_TIMEOUT_MS: u32 = 200;

/// Plays until dropped or until the device fails ([`Monitor::is_running`]).
pub struct Monitor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Monitor {
    /// Open `device_id` and start mixing `tap` into it; fails if the device cannot be opened.
    pub fn start(device_id: String, mut tap: SoundpadTap) -> Result<Self, Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let (opened, result) = std::sync::mpsc::channel();
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("macafon-soundpad-monitor".into())
            .spawn(move || {
                let outcome = (|| {
                    let com = ComGuard::new()?;
                    let stream = RenderStream::open(&device_id)?;
                    Ok::<_, Error>((com, stream))
                })();
                let (_com, stream) = match outcome {
                    Ok(opened_stream) => {
                        let _ = opened.send(Ok(()));
                        opened_stream
                    }
                    Err(error) => {
                        let _ = opened.send(Err(error));
                        return;
                    }
                };
                let _mmcss = ProAudioThread::register();
                while !stopping.load(Ordering::Acquire) {
                    if !stream.wait(WAIT_TIMEOUT_MS) {
                        continue;
                    }
                    let written = stream.render_available(u32::MAX, |output, channels| {
                        output.fill(0.0);
                        tap.mix(output, channels);
                    });
                    if let Err(error) = written {
                        tracing::warn!(%error, "Soundpad monitor stopped");
                        break;
                    }
                }
            })
            .map_err(|error| Error::Thread(error.to_string()))?;
        match result.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Thread("soundpad monitor thread exited".into()))
            }
        }
    }

    /// False once the device failed; the daemon opens it again.
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
