//! A/B playback (PRD §6.4) on a render endpoint: one WASAPI stream fed by [`AbRt`].

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{ProAudioThread, Stream};
use assert_no_alloc::assert_no_alloc;
use macafon_engine::AbRt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use windows::Win32::Media::Audio::{AUDCLNT_BUFFERFLAGS_SILENT, IAudioRenderClient};

const CHANNELS: u16 = 2;
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
                let mut mono = vec![0.0f32; MAX_FRAMES].into_boxed_slice();
                let buffer_frames = stream.buffer_frames.min(MAX_FRAMES as u32);
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
                            let frames = buffer_frames.saturating_sub(padding);
                            if frames == 0 {
                                return Ok(());
                            }
                            let data = client.GetBuffer(frames)?;
                            let out = std::slice::from_raw_parts_mut(
                                data.cast::<f32>(),
                                frames as usize * channels,
                            );
                            let block = &mut mono[..frames as usize];
                            assert_no_alloc(|| {
                                player.process(block);
                                for (frame, sample) in
                                    out.chunks_exact_mut(channels).zip(block.iter())
                                {
                                    frame.fill(*sample);
                                }
                            });
                            client.ReleaseBuffer(frames, 0)
                        })()
                    };
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
