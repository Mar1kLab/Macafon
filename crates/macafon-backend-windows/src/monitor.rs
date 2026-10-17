//! Soundpad monitor (PRD §7.6): one WASAPI stream that plays the soundpad into the user's
//! headphones, next to the sound going into the cable.

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{ProAudioThread, Stream};
use assert_no_alloc::assert_no_alloc;
use macafon_backend::SoundpadTap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use windows::Win32::Media::Audio::{AUDCLNT_BUFFERFLAGS_SILENT, IAudioRenderClient};

const CHANNELS: u16 = 2;
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
                    let stream = Stream::open(&device_id, CHANNELS, 0)?;
                    // SAFETY: the client was initialised by Stream::open.
                    let client: IAudioRenderClient = unsafe { stream.client.GetService() }?;
                    // SAFETY: prefill with silence before Start, as WASAPI expects.
                    unsafe {
                        client.GetBuffer(stream.buffer_frames)?;
                        client.ReleaseBuffer(
                            stream.buffer_frames,
                            AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
                        )?;
                        stream.client.Start()?;
                    }
                    Ok::<_, Error>((com, stream, client))
                })();
                let (_com, stream, client) = match outcome {
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
                let channels = usize::from(stream.channels);
                while !stopping.load(Ordering::Acquire) {
                    if !stream.wait(WAIT_TIMEOUT_MS) {
                        continue;
                    }
                    // SAFETY: GetBuffer returns room for `frames` interleaved float32 frames,
                    // released right after filling.
                    let written = unsafe {
                        (|| {
                            let padding = stream.client.GetCurrentPadding()?;
                            let frames = stream.buffer_frames.saturating_sub(padding);
                            if frames == 0 {
                                return Ok(());
                            }
                            let data = client.GetBuffer(frames)?;
                            let out = std::slice::from_raw_parts_mut(
                                data.cast::<f32>(),
                                frames as usize * channels,
                            );
                            assert_no_alloc(|| {
                                out.fill(0.0);
                                tap.mix(out, channels);
                            });
                            client.ReleaseBuffer(frames, 0)
                        })()
                    };
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
