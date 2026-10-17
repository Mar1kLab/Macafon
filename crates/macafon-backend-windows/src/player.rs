//! A/B playback (PRD §6.4) on a render endpoint: one WASAPI stream fed by [`AbRt`].

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{ProAudioThread, RenderStream};
use macafon_engine::AbRt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

const WAIT_TIMEOUT_MS: u32 = 200;
/// Largest render request handled in one piece: one second.
const MAX_FRAMES: usize = macafon_dsp::SAMPLE_RATE_HZ as usize;

/// Plays until dropped.
pub struct Player {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Open `device_id` and start playing; fails if the device cannot be opened.
    pub fn start(device_id: String, mut player: AbRt) -> Result<Self, Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let (opened, result) = std::sync::mpsc::channel();
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("macafon-ab".into())
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
                let mut mono = vec![0.0f32; MAX_FRAMES].into_boxed_slice();
                while !stopping.load(Ordering::Acquire) {
                    if !stream.wait(WAIT_TIMEOUT_MS) {
                        continue;
                    }
                    let written = stream.render_available(MAX_FRAMES as u32, |output, channels| {
                        let block = &mut mono[..output.len() / channels];
                        player.process(block);
                        for (frame, sample) in output.chunks_exact_mut(channels).zip(block.iter()) {
                            frame.fill(*sample);
                        }
                    });
                    if let Err(error) = written {
                        tracing::warn!(%error, "A/B playback stopped");
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
                Err(Error::Thread("A/B playback thread exited".into()))
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
