//! Virtual microphone path (PRD §12.2.1): WASAPI capture → chain → drift buffer → WASAPI
//! render into `CABLE Input` (or any render endpoint while testing).
//!
//! The chain runs on the capture thread (MMCSS "Pro Audio"); the render thread only reads the
//! drift buffer. Both loops are allocation-free after start-up.

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{HNS_PER_SECOND, ProAudioThread, Stream, device_period_hns};
use assert_no_alloc::assert_no_alloc;
use macafon_engine::{DriftReader, DriftStats, DriftWriter, MicRt, drift_buffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient,
    IAudioRenderClient,
};

const RATE_HZ: f64 = macafon_dsp::SAMPLE_RATE_HZ as f64;
/// The cable is stereo; mono is duplicated into both channels (MIC-011 on Windows).
const RENDER_CHANNELS: u16 = 2;
const WAIT_TIMEOUT_MS: u32 = 200;

#[derive(Debug, Clone)]
pub struct MicrophoneConfig {
    pub input_id: String,
    pub output_id: String,
    /// Capture-to-playback latency kept by the drift loop on top of the minimum safe level
    /// (render buffer + one capture period).
    pub latency_margin_ms: f64,
    /// Requested WASAPI buffer per stream, ms; 0 lets the audio engine choose.
    pub stream_buffer_ms: f64,
    /// Render at the device's smallest shared-mode period (IAudioClient3), falling back to
    /// the default period when the device cannot.
    pub low_latency_render: bool,
}

#[derive(Debug, Default)]
struct Counters {
    capture_frames: AtomicU64,
    render_frames: AtomicU64,
    discontinuities: AtomicU32,
    render_padding: AtomicU32,
    render_max_request: AtomicU32,
    capture_max_packet: AtomicU32,
}

#[derive(Debug, Clone, Copy)]
struct StreamInfo {
    buffer_frames: u32,
    period_hns: i64,
    latency_hns: i64,
}

/// Snapshot for the spike report and, later, Doctor (WIN-MIC-008).
#[derive(Debug, Clone, Copy)]
pub struct PipelineReport {
    pub elapsed_s: f64,
    pub capture_buffer_ms: f64,
    pub capture_period_ms: f64,
    pub capture_latency_ms: f64,
    pub render_buffer_ms: f64,
    pub render_period_ms: f64,
    pub render_latency_ms: f64,
    pub render_padding_ms: f64,
    pub chain_latency_ms: f64,
    /// Capture-to-playback level held by the drift loop: its buffer plus the render queue.
    pub drift_level_ms: f64,
    /// Lowest drift-buffer fill since the previous report.
    pub drift_buffer_min_ms: Option<f64>,
    /// Largest single render request and capture packet since the previous report.
    pub render_max_request_ms: f64,
    pub capture_max_packet_ms: f64,
    /// Correction applied by the drift loop; converges to the clock difference.
    pub drift_ratio_ppm: i32,
    /// Independent estimate from frame counts against the wall clock.
    pub capture_rate_hz: f64,
    pub render_rate_hz: f64,
    pub underruns: u32,
    pub dropped_frames: u64,
    pub skipped_frames: u64,
    pub discontinuities: u32,
}

impl PipelineReport {
    /// Capture period + chain + drift level + engine latencies, without the cable's own
    /// buffer.
    pub fn estimated_latency_ms(&self) -> f64 {
        self.capture_period_ms
            + self.chain_latency_ms
            + self.drift_level_ms
            + self.capture_latency_ms.max(0.0)
            + self.render_latency_ms.max(0.0)
    }
}

pub struct MicrophonePipeline {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<Result<(), Error>>>,
    counters: Arc<Counters>,
    drift: Arc<DriftStats>,
    capture: StreamInfo,
    render: StreamInfo,
    chain_latency_samples: u32,
    started: Instant,
}

impl MicrophonePipeline {
    pub fn start(config: &MicrophoneConfig, processor: MicRt) -> Result<Self, Error> {
        // The render side must keep one whole capture packet in reserve: packets arrive once
        // per capture period, however short the render period is.
        let capture_period_frames =
            (device_period_hns(&config.input_id)? as f64 * RATE_HZ / HNS_PER_SECOND).ceil() as i64;
        let reserve_frames =
            capture_period_frames + (config.latency_margin_ms * RATE_HZ / 1000.0).round() as i64;
        let (writer, reader) = drift_buffer(1);
        let drift = reader.stats();
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(Counters::default());
        let buffer_hns = (config.stream_buffer_ms * HNS_PER_SECOND / 1000.0) as i64;
        let chain_latency_samples = processor.latency_samples();

        let low_latency = config.low_latency_render;
        let (ready_tx, ready_rx) = mpsc::channel();
        let render = {
            let (stop, counters, id, ready) = (
                stop.clone(),
                counters.clone(),
                config.output_id.clone(),
                ready_tx.clone(),
            );
            spawn("macafon-render", move || {
                render_loop(
                    &id,
                    buffer_hns,
                    low_latency,
                    reserve_frames,
                    reader,
                    &stop,
                    &counters,
                    ready,
                )
            })?
        };
        let render_info = wait_ready(&ready_rx)?;
        let capture = {
            let (stop, counters, id) = (stop.clone(), counters.clone(), config.input_id.clone());
            spawn("macafon-capture", move || {
                capture_loop(
                    &id, buffer_hns, processor, writer, &stop, &counters, ready_tx,
                )
            })?
        };
        let capture_info = wait_ready(&ready_rx)?;
        Ok(Self {
            stop,
            threads: vec![capture, render],
            counters,
            drift,
            capture: capture_info,
            render: render_info,
            chain_latency_samples,
            started: Instant::now(),
        })
    }

    pub fn is_running(&self) -> bool {
        self.threads.iter().all(|thread| !thread.is_finished())
    }

    pub fn report(&self) -> PipelineReport {
        let elapsed_s = self.started.elapsed().as_secs_f64();
        let ms = |frames: f64| frames * 1000.0 / RATE_HZ;
        let hns_ms = |hns: i64| hns as f64 / 10_000.0;
        let counters = &self.counters;
        PipelineReport {
            elapsed_s,
            capture_buffer_ms: ms(f64::from(self.capture.buffer_frames)),
            capture_period_ms: hns_ms(self.capture.period_hns),
            capture_latency_ms: hns_ms(self.capture.latency_hns),
            render_buffer_ms: ms(f64::from(self.render.buffer_frames)),
            render_period_ms: hns_ms(self.render.period_hns),
            render_latency_ms: hns_ms(self.render.latency_hns),
            render_padding_ms: ms(f64::from(counters.render_padding.load(Ordering::Relaxed))),
            chain_latency_ms: ms(f64::from(self.chain_latency_samples)),
            drift_level_ms: ms(f64::from(self.drift.level_frames())),
            drift_buffer_min_ms: self
                .drift
                .take_min_fill_frames()
                .map(|frames| ms(f64::from(frames))),
            render_max_request_ms: ms(f64::from(
                counters.render_max_request.swap(0, Ordering::Relaxed),
            )),
            capture_max_packet_ms: ms(f64::from(
                counters.capture_max_packet.swap(0, Ordering::Relaxed),
            )),
            drift_ratio_ppm: self.drift.ratio_ppm(),
            capture_rate_hz: counters.capture_frames.load(Ordering::Relaxed) as f64 / elapsed_s,
            render_rate_hz: counters.render_frames.load(Ordering::Relaxed) as f64 / elapsed_s,
            underruns: self.drift.underruns(),
            dropped_frames: self.drift.dropped_frames(),
            skipped_frames: self.drift.skipped_frames(),
            discontinuities: counters.discontinuities.load(Ordering::Relaxed),
        }
    }

    /// Stop both threads and return the first error either of them hit.
    pub fn stop(mut self) -> Result<(), Error> {
        self.stop.store(true, Ordering::Release);
        let mut result = Ok(());
        for thread in self.threads.drain(..) {
            let outcome = thread
                .join()
                .unwrap_or_else(|_| Err(Error::Thread("audio thread panicked".into())));
            if result.is_ok() {
                result = outcome;
            }
        }
        result
    }
}

impl Drop for MicrophonePipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn spawn(
    name: &str,
    body: impl FnOnce() -> Result<(), Error> + Send + 'static,
) -> Result<JoinHandle<Result<(), Error>>, Error> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(body)
        .map_err(|error| Error::Thread(error.to_string()))
}

fn wait_ready(ready: &mpsc::Receiver<Result<StreamInfo, Error>>) -> Result<StreamInfo, Error> {
    ready
        .recv()
        .map_err(|_| Error::Thread("audio thread exited during start-up".into()))?
}

fn info(stream: &Stream) -> StreamInfo {
    StreamInfo {
        buffer_frames: stream.buffer_frames,
        period_hns: stream.period_hns,
        latency_hns: stream.latency_hns,
    }
}

fn capture_loop(
    device_id: &str,
    buffer_hns: i64,
    mut processor: MicRt,
    mut writer: DriftWriter,
    stop: &AtomicBool,
    counters: &Counters,
    ready: mpsc::Sender<Result<StreamInfo, Error>>,
) -> Result<(), Error> {
    let _com = ComGuard::new()?;
    let opened = Stream::open(device_id, 1, buffer_hns).and_then(|stream| {
        // SAFETY: the client was initialised by Stream::open.
        let client: IAudioCaptureClient = unsafe { stream.client.GetService() }?;
        Ok((stream, client))
    });
    let (stream, client) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(Error::Thread(format!("capture: {error}"))));
            return Err(error);
        }
    };
    let mut scratch = vec![0.0f32; stream.buffer_frames as usize].into_boxed_slice();
    let _ = ready.send(Ok(info(&stream)));
    let _mmcss = ProAudioThread::register();
    // SAFETY: starting an initialised client.
    unsafe { stream.client.Start() }?;
    while !stop.load(Ordering::Acquire) {
        if !stream.wait(WAIT_TIMEOUT_MS) {
            continue;
        }
        // SAFETY: GetBuffer/ReleaseBuffer pairs on our capture client; the packet holds
        // `frames` mono float32 samples as negotiated in Stream::open.
        unsafe {
            while client.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                client.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let count = (frames as usize).min(scratch.len());
                let block = &mut scratch[..count];
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    block.fill(0.0);
                } else {
                    block.copy_from_slice(std::slice::from_raw_parts(data.cast::<f32>(), count));
                }
                client.ReleaseBuffer(frames)?;
                counters
                    .capture_max_packet
                    .fetch_max(frames, Ordering::Relaxed);
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    counters.discontinuities.fetch_add(1, Ordering::Relaxed);
                }
                assert_no_alloc(|| {
                    processor.process(block);
                    writer.write(block);
                });
                counters
                    .capture_frames
                    .fetch_add(u64::from(frames), Ordering::Relaxed);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_loop(
    device_id: &str,
    buffer_hns: i64,
    low_latency: bool,
    reserve_frames: i64,
    mut reader: DriftReader,
    stop: &AtomicBool,
    counters: &Counters,
    ready: mpsc::Sender<Result<StreamInfo, Error>>,
) -> Result<(), Error> {
    let _com = ComGuard::new()?;
    let stream = if low_latency {
        Stream::open_low_latency(device_id).or_else(|error| {
            tracing::warn!("low-latency render unavailable, using the default period: {error}");
            Stream::open(device_id, RENDER_CHANNELS, buffer_hns)
        })
    } else {
        Stream::open(device_id, RENDER_CHANNELS, buffer_hns)
    };
    let opened = stream.and_then(|stream| {
        // SAFETY: the client was initialised by Stream::open.
        let client: IAudioRenderClient = unsafe { stream.client.GetService() }?;
        Ok((stream, client))
    });
    let (stream, client) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(Error::Thread(format!("render: {error}"))));
            return Err(error);
        }
    };
    let buffer_frames = stream.buffer_frames;
    let channels = usize::from(stream.channels);
    // A request may refill the whole render buffer while the next capture packet is still
    // pending; keeping at least that much in flight rules out underruns (see drift.rs).
    let period_frames = (stream.period_hns as f64 * RATE_HZ / HNS_PER_SECOND).ceil() as i64;
    let target = i64::from(buffer_frames) + reserve_frames.max(period_frames);
    reader.set_target(target.max(i64::from(buffer_frames)) as usize);
    let mut mono = vec![0.0f32; buffer_frames as usize].into_boxed_slice();
    // SAFETY: prefill the whole buffer with silence before Start, as WASAPI expects.
    unsafe {
        client.GetBuffer(buffer_frames)?;
        client.ReleaseBuffer(buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
    }
    let _ = ready.send(Ok(info(&stream)));
    let _mmcss = ProAudioThread::register();
    // SAFETY: starting an initialised client.
    unsafe { stream.client.Start() }?;
    while !stop.load(Ordering::Acquire) {
        if !stream.wait(WAIT_TIMEOUT_MS) {
            continue;
        }
        // SAFETY: GetBuffer returns room for `frames` interleaved float32 frames of
        // `channels` channels, released right after filling.
        unsafe {
            let padding = stream.client.GetCurrentPadding()?;
            counters.render_padding.store(padding, Ordering::Relaxed);
            let frames = buffer_frames.saturating_sub(padding);
            if frames == 0 {
                continue;
            }
            counters
                .render_max_request
                .fetch_max(frames, Ordering::Relaxed);
            let data = client.GetBuffer(frames)?;
            let out =
                std::slice::from_raw_parts_mut(data.cast::<f32>(), frames as usize * channels);
            let block = &mut mono[..frames as usize];
            assert_no_alloc(|| {
                reader.read(block, padding as usize);
                for (frame, sample) in out.chunks_exact_mut(channels).zip(block.iter()) {
                    frame.fill(*sample);
                }
            });
            client.ReleaseBuffer(frames, 0)?;
            counters
                .render_frames
                .fetch_add(u64::from(frames), Ordering::Relaxed);
        }
    }
    Ok(())
}
