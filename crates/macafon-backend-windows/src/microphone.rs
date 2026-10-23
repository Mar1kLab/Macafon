//! Virtual microphone path (PRD §12.2.1): WASAPI capture → chain → drift buffer → WASAPI
//! render into `CABLE Input` (or any render endpoint while testing).
//!
//! The chain runs on the capture thread (MMCSS "Pro Audio"); the render thread only reads the
//! drift buffer. Both threads outlive their devices (WIN-MIC-006): a lost or reconfigured
//! endpoint is reopened every 500 ms, and devices can be switched while running. Without a
//! microphone the cable plays silence; without the cable the chain still runs for meters and
//! calibration (WIN-MIC-001). The streaming loops are allocation-free and lock-free; the
//! selection lock is only taken between streams.

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{HNS_PER_SECOND, ProAudioThread, Stream};
use assert_no_alloc::assert_no_alloc;
use macafon_backend::SoundDeckTap;
use macafon_engine::{DriftReader, DriftStats, DriftWriter, MicRt, drift_buffer};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient,
    IAudioRenderClient,
};

const RATE_HZ: f64 = macafon_dsp::SAMPLE_RATE_HZ as f64;
/// The cable is stereo; mono is duplicated into both channels (MIC-011 on Windows).
const RENDER_CHANNELS: u16 = 2;
const WAIT_TIMEOUT_MS: u32 = 200;
const RETRY_INTERVAL: Duration = Duration::from_millis(500);
/// Assumed until the capture device reports its period: the shared-mode default.
const DEFAULT_PERIOD_HNS: i64 = 100_000;
/// Largest capture packet handled in one piece; WASAPI packets are one period (≤ 1 s).
const MAX_PACKET_FRAMES: usize = macafon_dsp::SAMPLE_RATE_HZ as usize;
/// Chain timings queued for the control loop: several seconds of 10 ms packets.
const BUDGET_QUEUE: usize = 1024;

#[derive(Debug, Clone, Default)]
pub struct MicrophoneConfig {
    /// Physical microphone; `None` sends silence into the cable.
    pub input_id: Option<String>,
    /// `CABLE Input`; `None` keeps the chain running without output.
    pub output_id: Option<String>,
    /// Capture-to-playback latency kept by the drift loop on top of the minimum safe level
    /// (render buffer + one capture period).
    pub latency_margin_ms: f64,
    /// Requested WASAPI buffer per stream, ms; 0 lets the audio engine choose.
    pub stream_buffer_ms: f64,
    /// Render at the device's smallest shared-mode period (IAudioClient3), falling back to
    /// the default period when the device cannot.
    pub low_latency_render: bool,
}

/// The device an audio thread should use. The thread compares `generation` while streaming
/// and reads `device` only between streams.
#[derive(Default)]
struct Selection {
    generation: AtomicU64,
    device: Mutex<Option<String>>,
}

impl Selection {
    fn new(device: Option<String>) -> Self {
        Self {
            generation: AtomicU64::new(0),
            device: Mutex::new(device),
        }
    }

    fn set(&self, device: Option<String>) {
        *self.device.lock().unwrap_or_else(PoisonError::into_inner) = device;
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn current(&self) -> (u64, Option<String>) {
        let generation = self.generation.load(Ordering::Acquire);
        let device = self
            .device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        (generation, device)
    }

    fn changed_since(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) != generation
    }
}

/// What the open stream of one side looks like; zero while disconnected.
#[derive(Debug, Default)]
struct StreamState {
    connected: AtomicBool,
    buffer_frames: AtomicU32,
    period_hns: AtomicI64,
    latency_hns: AtomicI64,
}

impl StreamState {
    fn publish(&self, stream: Option<&Stream>) {
        let (buffer, period, latency) = stream.map_or((0, 0, 0), |s| {
            (s.buffer_frames, s.period_hns, s.latency_hns)
        });
        self.buffer_frames.store(buffer, Ordering::Relaxed);
        self.period_hns.store(period, Ordering::Relaxed);
        self.latency_hns.store(latency, Ordering::Relaxed);
        self.connected.store(stream.is_some(), Ordering::Release);
    }
}

#[derive(Debug, Default)]
struct Counters {
    capture_frames: AtomicU64,
    render_frames: AtomicU64,
    discontinuities: AtomicU32,
    render_padding: AtomicU32,
    render_max_request: AtomicU32,
    capture_max_packet: AtomicU32,
    reconnects: AtomicU32,
}

#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    input: Selection,
    output: Selection,
    capture: StreamState,
    render: StreamState,
    counters: Counters,
    /// Time the chain while Studio needs overload detection (NFR-S3).
    measure_budget: AtomicBool,
    /// Measurements were lost to a full queue; the next read reports a gap.
    budget_gap: AtomicBool,
}

/// Snapshot for the spike report and, later, Doctor (WIN-MIC-008).
#[derive(Debug, Clone, Copy)]
pub struct PipelineReport {
    pub elapsed_s: f64,
    pub input_connected: bool,
    pub output_connected: bool,
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
    /// Streams reopened after a device was lost or switched.
    pub reconnects: u32,
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

/// One capture packet: chain processing time against the packet's duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetSample {
    pub elapsed_ns: u64,
    pub budget_ns: u64,
}

impl BudgetSample {
    /// A sampling gap: resets the host's over-budget streak.
    pub const GAP: Self = Self {
        elapsed_ns: 0,
        budget_ns: 1,
    };
}

pub struct MicrophonePipeline {
    shared: Arc<Shared>,
    budget: Consumer<BudgetSample>,
    threads: Vec<JoinHandle<Result<(), Error>>>,
    drift: Arc<DriftStats>,
    chain_latency_samples: u32,
    started: Instant,
}

impl MicrophonePipeline {
    /// Start both threads. Missing devices are not an error: each side keeps retrying.
    /// SoundDeck sounds (§7.6) are mixed into the cable after the chain by `sounddeck`.
    pub fn start(
        config: &MicrophoneConfig,
        processor: MicRt,
        sounddeck: Option<SoundDeckTap>,
    ) -> Result<Self, Error> {
        let (writer, reader) = drift_buffer(1);
        let (budget_producer, budget) = RingBuffer::new(BUDGET_QUEUE);
        let drift = reader.stats();
        let shared = Arc::new(Shared {
            input: Selection::new(config.input_id.clone()),
            output: Selection::new(config.output_id.clone()),
            ..Shared::default()
        });
        let render = RenderSettings {
            buffer_hns: (config.stream_buffer_ms * HNS_PER_SECOND / 1000.0) as i64,
            low_latency: config.low_latency_render,
            margin_frames: (config.latency_margin_ms * RATE_HZ / 1000.0).round() as i64,
        };
        let chain_latency_samples = processor.latency_samples();
        let mut threads = Vec::with_capacity(2);
        let state = shared.clone();
        threads.push(spawn("macafon-render", move || {
            render_thread(&state, render, reader, sounddeck)
        })?);
        let state = shared.clone();
        let buffer_hns = render.buffer_hns;
        let capture = spawn("macafon-capture", move || {
            capture_thread(&state, buffer_hns, processor, writer, budget_producer)
        });
        match capture {
            Ok(thread) => threads.push(thread),
            Err(error) => {
                // The render thread must not outlive a pipeline that never started.
                shared.stop.store(true, Ordering::Release);
                for thread in threads {
                    let _ = thread.join();
                }
                return Err(error);
            }
        }
        Ok(Self {
            shared,
            budget,
            threads,
            drift,
            chain_latency_samples,
            started: Instant::now(),
        })
    }

    /// Switch the physical microphone (MIC-001/MIC-006); `None` sends silence.
    pub fn set_input(&self, device_id: Option<String>) {
        self.shared.input.set(device_id);
    }

    /// Switch the render target; `None` keeps processing without output.
    pub fn set_output(&self, device_id: Option<String>) {
        self.shared.output.set(device_id);
    }

    pub fn input_connected(&self) -> bool {
        self.shared.capture.connected.load(Ordering::Acquire)
    }

    pub fn output_connected(&self) -> bool {
        self.shared.render.connected.load(Ordering::Acquire)
    }

    /// Current capture period, ms; 0 while no microphone is open.
    pub fn capture_period_ms(&self) -> f64 {
        self.shared.capture.period_hns.load(Ordering::Relaxed) as f64 / 10_000.0
    }

    /// Start or stop timing the chain on the capture thread (NFR-S3). Turning it on reports a
    /// gap first, so a streak never spans an unmeasured stretch.
    pub fn set_budget_monitoring(&self, enabled: bool) {
        if !self.shared.measure_budget.swap(enabled, Ordering::AcqRel) && enabled {
            self.shared.budget_gap.store(true, Ordering::Release);
        }
    }

    /// Hand queued chain timings, oldest first, to `observe`; a lost stretch arrives as
    /// [`BudgetSample::GAP`].
    pub fn drain_budget(&mut self, mut observe: impl FnMut(BudgetSample)) {
        if self.shared.budget_gap.swap(false, Ordering::AcqRel) {
            observe(BudgetSample::GAP);
        }
        while let Ok(sample) = self.budget.pop() {
            observe(sample);
        }
    }

    /// False once a thread has stopped on an unrecoverable error.
    pub fn is_running(&self) -> bool {
        self.threads.iter().all(|thread| !thread.is_finished())
    }

    pub fn report(&self) -> PipelineReport {
        let elapsed_s = self.started.elapsed().as_secs_f64();
        let ms = |frames: f64| frames * 1000.0 / RATE_HZ;
        let hns_ms = |hns: &AtomicI64| hns.load(Ordering::Relaxed) as f64 / 10_000.0;
        let frames_ms = |frames: &AtomicU32| ms(f64::from(frames.load(Ordering::Relaxed)));
        let take_ms = |frames: &AtomicU32| ms(f64::from(frames.swap(0, Ordering::Relaxed)));
        let (shared, counters) = (&self.shared, &self.shared.counters);
        PipelineReport {
            elapsed_s,
            input_connected: self.input_connected(),
            output_connected: self.output_connected(),
            capture_buffer_ms: frames_ms(&shared.capture.buffer_frames),
            capture_period_ms: hns_ms(&shared.capture.period_hns),
            capture_latency_ms: hns_ms(&shared.capture.latency_hns),
            render_buffer_ms: frames_ms(&shared.render.buffer_frames),
            render_period_ms: hns_ms(&shared.render.period_hns),
            render_latency_ms: hns_ms(&shared.render.latency_hns),
            render_padding_ms: frames_ms(&counters.render_padding),
            chain_latency_ms: ms(f64::from(self.chain_latency_samples)),
            drift_level_ms: ms(f64::from(self.drift.level_frames())),
            drift_buffer_min_ms: self
                .drift
                .take_min_fill_frames()
                .map(|frames| ms(f64::from(frames))),
            render_max_request_ms: take_ms(&counters.render_max_request),
            capture_max_packet_ms: take_ms(&counters.capture_max_packet),
            drift_ratio_ppm: self.drift.ratio_ppm(),
            capture_rate_hz: counters.capture_frames.load(Ordering::Relaxed) as f64 / elapsed_s,
            render_rate_hz: counters.render_frames.load(Ordering::Relaxed) as f64 / elapsed_s,
            underruns: self.drift.underruns(),
            dropped_frames: self.drift.dropped_frames(),
            skipped_frames: self.drift.skipped_frames(),
            discontinuities: counters.discontinuities.load(Ordering::Relaxed),
            reconnects: counters.reconnects.load(Ordering::Relaxed),
        }
    }

    /// Stop both threads and return the first error either of them hit.
    pub fn stop(mut self) -> Result<(), Error> {
        self.shared.stop.store(true, Ordering::Release);
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
        self.shared.stop.store(true, Ordering::Release);
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

/// Why a stream ended.
enum Ended {
    /// Stop requested or a different device selected: reopen immediately.
    Requested,
    /// The device failed (unplugged, format changed): retry after a pause.
    Lost(Error),
}

/// Run one side: open the selected device, stream until it fails or changes, repeat.
fn supervise(
    shared: &Shared,
    selection: &Selection,
    state: &StreamState,
    side: &str,
    mut run: impl FnMut(&str, u64) -> Result<Ended, Error>,
) {
    let mut opened_before = false;
    while !shared.stop.load(Ordering::Acquire) {
        let (generation, device) = selection.current();
        let ended = match device {
            None => Ended::Lost(Error::DeviceNotFound("none selected".into())),
            Some(id) => run(&id, generation).unwrap_or_else(Ended::Lost),
        };
        state.publish(None);
        match ended {
            Ended::Requested => {}
            Ended::Lost(error) => {
                if opened_before {
                    tracing::warn!("{side} stream lost: {error}");
                    opened_before = false;
                }
                wait_for_retry(shared, selection, generation);
                continue;
            }
        }
        opened_before = true;
    }
}

fn wait_for_retry(shared: &Shared, selection: &Selection, generation: u64) {
    let deadline = Instant::now() + RETRY_INTERVAL;
    while Instant::now() < deadline
        && !shared.stop.load(Ordering::Acquire)
        && !selection.changed_since(generation)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn capture_thread(
    shared: &Shared,
    buffer_hns: i64,
    mut processor: MicRt,
    mut writer: DriftWriter,
    mut budget: Producer<BudgetSample>,
) -> Result<(), Error> {
    let _com = ComGuard::new()?;
    let _mmcss = ProAudioThread::register();
    let mut scratch = vec![0.0f32; MAX_PACKET_FRAMES].into_boxed_slice();
    supervise(
        shared,
        &shared.input,
        &shared.capture,
        "capture",
        |id, generation| {
            let stream = Stream::open(id, 1, buffer_hns)?;
            // SAFETY: the client was initialised by Stream::open.
            let client: IAudioCaptureClient = unsafe { stream.client.GetService() }?;
            shared.capture.publish(Some(&stream));
            shared.counters.reconnects.fetch_add(1, Ordering::Relaxed);
            // SAFETY: starting an initialised client.
            unsafe { stream.client.Start() }?;
            while !shared.stop.load(Ordering::Acquire) && !shared.input.changed_since(generation) {
                if !stream.wait(WAIT_TIMEOUT_MS) {
                    // Invalidation can stop event delivery entirely: probe, so the error
                    // reaches the supervisor, which reopens the device.
                    // SAFETY: a call on our initialised client.
                    unsafe { stream.client.GetCurrentPadding() }?;
                    continue;
                }
                let output = shared.render.connected.load(Ordering::Acquire);
                // SAFETY: GetBuffer/ReleaseBuffer pairs on our capture client; the packet holds
                // `frames` mono float32 samples as negotiated in Stream::open.
                unsafe {
                    while client.GetNextPacketSize()? > 0 {
                        let mut data = std::ptr::null_mut();
                        let (mut frames, mut flags) = (0, 0);
                        client.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                        let count = (frames as usize).min(scratch.len());
                        let block = &mut scratch[..count];
                        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                            block.fill(0.0);
                        } else {
                            block.copy_from_slice(std::slice::from_raw_parts(
                                data.cast::<f32>(),
                                count,
                            ));
                        }
                        client.ReleaseBuffer(frames)?;
                        let counters = &shared.counters;
                        counters
                            .capture_max_packet
                            .fetch_max(frames, Ordering::Relaxed);
                        if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                            counters.discontinuities.fetch_add(1, Ordering::Relaxed);
                        }
                        let measure = shared.measure_budget.load(Ordering::Relaxed);
                        // QueryPerformanceCounter: user-mode on current hardware, no wait.
                        let started = measure.then(Instant::now);
                        assert_no_alloc(|| {
                            processor.process(block);
                            if let Some(started) = started {
                                let sample = BudgetSample {
                                    elapsed_ns: started.elapsed().as_nanos() as u64,
                                    budget_ns: frames_ns(count),
                                };
                                if budget.push(sample).is_err() {
                                    shared.budget_gap.store(true, Ordering::Release);
                                }
                            }
                            // Without a cable nothing drains the buffer; keep it from filling.
                            if output {
                                writer.write(block);
                            }
                        });
                        counters
                            .capture_frames
                            .fetch_add(u64::from(frames), Ordering::Relaxed);
                    }
                }
            }
            Ok(Ended::Requested)
        },
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct RenderSettings {
    buffer_hns: i64,
    low_latency: bool,
    margin_frames: i64,
}

fn render_thread(
    shared: &Shared,
    settings: RenderSettings,
    mut reader: DriftReader,
    mut sounddeck: Option<SoundDeckTap>,
) -> Result<(), Error> {
    let _com = ComGuard::new()?;
    let _mmcss = ProAudioThread::register();
    let mut mono = vec![0.0f32; MAX_PACKET_FRAMES].into_boxed_slice();
    supervise(
        shared,
        &shared.output,
        &shared.render,
        "render",
        |id, generation| {
            let stream = if settings.low_latency {
                Stream::open_low_latency(id).or_else(|error| {
                    tracing::warn!(
                        "low-latency render unavailable, using the default period: {error}"
                    );
                    Stream::open(id, RENDER_CHANNELS, settings.buffer_hns)
                })?
            } else {
                Stream::open(id, RENDER_CHANNELS, settings.buffer_hns)?
            };
            // SAFETY: the client was initialised by Stream::open.
            let client: IAudioRenderClient = unsafe { stream.client.GetService() }?;
            let buffer_frames = stream.buffer_frames.min(MAX_PACKET_FRAMES as u32);
            let channels = usize::from(stream.channels);
            let render_period = to_frames(stream.period_hns);
            let mut capture_period = 0;
            // SAFETY: prefill the whole buffer with silence before Start, as WASAPI expects.
            unsafe {
                client.GetBuffer(stream.buffer_frames)?;
                client.ReleaseBuffer(stream.buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
            }
            shared.render.publish(Some(&stream));
            shared.counters.reconnects.fetch_add(1, Ordering::Relaxed);
            // SAFETY: starting an initialised client.
            unsafe { stream.client.Start() }?;
            while !shared.stop.load(Ordering::Acquire) && !shared.output.changed_since(generation) {
                // A request may refill the whole render buffer while the next capture packet is
                // still pending; keeping that much in flight rules out underruns (drift.rs).
                let period = match shared.capture.period_hns.load(Ordering::Relaxed) {
                    0 => to_frames(DEFAULT_PERIOD_HNS),
                    hns => to_frames(hns),
                };
                if period != capture_period {
                    capture_period = period;
                    let reserve = (capture_period + settings.margin_frames).max(render_period);
                    reader.set_target((i64::from(buffer_frames) + reserve) as usize);
                }
                if !stream.wait(WAIT_TIMEOUT_MS) {
                    // As in capture: an invalidated stream may never signal again.
                    // SAFETY: a call on our initialised client.
                    unsafe { stream.client.GetCurrentPadding() }?;
                    continue;
                }
                // SAFETY: GetBuffer returns room for `frames` interleaved float32 frames of
                // `channels` channels, released right after filling.
                unsafe {
                    let padding = stream.client.GetCurrentPadding()?;
                    let counters = &shared.counters;
                    counters.render_padding.store(padding, Ordering::Relaxed);
                    let frames = buffer_frames.saturating_sub(padding);
                    if frames == 0 {
                        continue;
                    }
                    counters
                        .render_max_request
                        .fetch_max(frames, Ordering::Relaxed);
                    let data = client.GetBuffer(frames)?;
                    let out = std::slice::from_raw_parts_mut(
                        data.cast::<f32>(),
                        frames as usize * channels,
                    );
                    let block = &mut mono[..frames as usize];
                    assert_no_alloc(|| {
                        reader.read(block, padding as usize);
                        match &mut sounddeck {
                            Some(tap) => tap.mix_with_voice(block, out, channels),
                            None => {
                                for (frame, sample) in
                                    out.chunks_exact_mut(channels).zip(block.iter())
                                {
                                    frame.fill(*sample);
                                }
                            }
                        }
                    });
                    client.ReleaseBuffer(frames, 0)?;
                    counters
                        .render_frames
                        .fetch_add(u64::from(frames), Ordering::Relaxed);
                }
            }
            Ok(Ended::Requested)
        },
    );
    Ok(())
}

/// Duration of `frames` at the engine rate.
fn frames_ns(frames: usize) -> u64 {
    frames as u64 * 1_000_000_000 / macafon_dsp::SAMPLE_RATE_HZ as u64
}

fn to_frames(hns: i64) -> i64 {
    (hns as f64 * RATE_HZ / HNS_PER_SECOND).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DataFlow, EndpointRole, default_device};
    use macafon_dsp::Telemetry;
    use macafon_dsp::settings::ChainSettings;

    #[test]
    #[ignore = "needs a default capture device"]
    fn chain_timings_cover_capture_packets() {
        let _com = ComGuard::new().expect("COM");
        let input = default_device(DataFlow::Capture, EndpointRole::Console)
            .expect("default capture")
            .expect("a capture device");
        let telemetry = Arc::new(Telemetry::default());
        let (_control, processor) =
            macafon_engine::microphone(&ChainSettings::default(), telemetry).expect("chain");
        let config = MicrophoneConfig {
            input_id: Some(input),
            ..MicrophoneConfig::default()
        };
        let mut pipeline = MicrophonePipeline::start(&config, processor, None).expect("pipeline");
        pipeline.set_budget_monitoring(true);
        std::thread::sleep(Duration::from_millis(600));
        let mut samples = Vec::new();
        pipeline.drain_budget(|sample| samples.push(sample));
        pipeline.stop().expect("stop");

        assert_eq!(samples.first(), Some(&BudgetSample::GAP));
        let packets = &samples[1..];
        assert!(packets.len() >= 10, "{} packets", packets.len());
        // Each packet's budget is its duration: one capture period, ≤ 1 s.
        assert!(
            packets
                .iter()
                .all(|s| s.budget_ns > 0 && s.budget_ns <= 1_000_000_000)
        );
        assert!(packets.iter().all(|s| s.elapsed_ns < s.budget_ns));
    }
}
