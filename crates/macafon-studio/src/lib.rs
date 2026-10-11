//! Embedded DeepFilterNet3 inference runs on a dedicated worker: tract allocates tensors.
//! The audio thread only copies fixed-size frames through bounded SPSC queues.
use df::tract::{DfParams, DfTract, ReduceMask, RuntimeParams};
use macafon_dsp::settings::NoiseSuppressionParams;
use macafon_dsp::{FRAME_SAMPLES, SAMPLE_RATE_HZ, STUDIO_LATENCY_SAMPLES, sanitize};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// Model: FFT 960, hop 480, lookahead 2 => 1440; worker adapter => 960.

const QUEUE_FRAMES: usize = 8;
const WORKER_BUDGET: Duration = Duration::from_millis(8);

#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    #[error("could not start DeepFilterNet3 worker: {0}")]
    Worker(#[from] std::io::Error),
    #[error("could not initialize embedded DeepFilterNet3 model: {0}")]
    Model(String),
}

struct Frame {
    sequence: u64,
    samples: [f32; FRAME_SAMPLES],
}

#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    overload: AtomicBool,
    intensity_pct: AtomicU32,
    post_filter: AtomicBool,
    activity: AtomicU32,
    failure: OnceLock<String>,
    inference_ns: AtomicU64,
}

struct WorkerGuard(Arc<Shared>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
        self.0.overload.store(true, Ordering::Release);
    }
}

pub struct StudioNoiseSuppression {
    input: rtrb::Producer<Frame>,
    results: rtrb::Consumer<Frame>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    frame: [f32; FRAME_SAMPLES],
    output: [f32; FRAME_SAMPLES],
    position: usize,
    sequence: u64,
    missed: u8,
    offline: bool,
    delayed: [f32; FRAME_SAMPLES],
}

impl StudioNoiseSuppression {
    pub fn new(params: &NoiseSuppressionParams) -> Result<Self, StudioError> {
        Self::build(params, false)
    }
    /// Synchronous worker adapter for offline analysis; must never enter an audio callback.
    pub fn offline(params: &NoiseSuppressionParams) -> Result<Self, StudioError> {
        Self::build(params, true)
    }
    /// Last wall-clock inference duration, measured by the worker, for local profiling.
    pub fn last_inference_ns(&self) -> u64 {
        self.shared.inference_ns.load(Ordering::Relaxed)
    }
    fn build(params: &NoiseSuppressionParams, offline: bool) -> Result<Self, StudioError> {
        let (input, mut inputs) = rtrb::RingBuffer::<Frame>::new(QUEUE_FRAMES);
        let (mut output, results) = rtrb::RingBuffer::<Frame>::new(QUEUE_FRAMES);
        let shared = Arc::new(Shared::default());
        shared
            .intensity_pct
            .store(params.intensity_pct.to_bits(), Ordering::Relaxed);
        shared
            .post_filter
            .store(params.post_filter, Ordering::Relaxed);
        let worker_shared = shared.clone();
        let (ready, startup) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("macafon-studio".into())
            .spawn(move || {
                let _guard = WorkerGuard(worker_shared.clone());
                // The model stays on this thread; its tract state need not implement Send.
                let model = DfTract::new(
                    DfParams::default(),
                    &RuntimeParams::default().with_mask_reduce(ReduceMask::NONE),
                );
                let mut model = match model {
                    Ok(model) => model,
                    Err(error) => {
                        let _ = ready.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                let model_latency =
                    model.fft_size - model.hop_size + model.lookahead * model.hop_size;
                if model.sr != SAMPLE_RATE_HZ as usize
                    || model.hop_size != FRAME_SAMPLES
                    || model_latency + 2 * FRAME_SAMPLES != STUDIO_LATENCY_SAMPLES as usize
                {
                    let _ = ready.send(Err(
                        "embedded model has unexpected sample rate, hop or latency".into(),
                    ));
                    return;
                }
                let mut enhanced = [0.0; FRAME_SAMPLES];
                // Tract compiles and initializes its plans before the worker accepts audio.
                if ready.send(Ok(())).is_err() {
                    return;
                }
                let mut slow_frames = 0u8;
                let mut dry_delay = [0.0; (STUDIO_LATENCY_SAMPLES as usize) - 2 * FRAME_SAMPLES];
                let mut dry_position = 0usize;
                let mut delayed_dry = [0.0; FRAME_SAMPLES];
                while !worker_shared.stop.load(Ordering::Acquire) {
                    let Ok(frame) = inputs.pop() else {
                        thread::sleep(Duration::from_micros(250));
                        continue;
                    };
                    let intensity_pct =
                        f32::from_bits(worker_shared.intensity_pct.load(Ordering::Relaxed));
                    // Upstream's exact zero attenuation bypass removes the model latency.
                    // Keep its state running and use our aligned dry path at zero intensity.
                    model.set_atten_lim(intensity_pct.max(0.01));
                    for (sample, delayed) in frame.samples.iter().zip(&mut delayed_dry) {
                        *delayed = dry_delay[dry_position];
                        dry_delay[dry_position] = *sample;
                        dry_position += 1;
                        if dry_position == dry_delay.len() {
                            dry_position = 0;
                        }
                    }
                    model.post_filter = worker_shared.post_filter.load(Ordering::Relaxed);
                    let start = Instant::now();
                    let processed = model.process(
                        ndarray::ArrayView1::from(&frame.samples[..]).insert_axis(ndarray::Axis(0)),
                        ndarray::ArrayViewMut1::from(&mut enhanced[..])
                            .insert_axis(ndarray::Axis(0)),
                    );
                    let elapsed = start.elapsed();
                    worker_shared.inference_ns.store(
                        elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
                        Ordering::Relaxed,
                    );
                    slow_frames = if elapsed > WORKER_BUDGET {
                        slow_frames.saturating_add(1)
                    } else {
                        0
                    };
                    if (!offline && slow_frames >= 3) || processed.is_err() {
                        worker_shared.overload.store(true, Ordering::Release);
                    }
                    if let Err(error) = &processed {
                        let _ = worker_shared.failure.set(format!("{error:#}"));
                        enhanced.fill(0.0);
                    } else if intensity_pct < 0.01 {
                        enhanced = delayed_dry;
                    }
                    let input_power: f32 = frame.samples.iter().map(|x| x * x).sum();
                    let output_power: f32 = enhanced.iter().map(|x| x * x).sum();
                    let activity = if input_power > 1e-12 {
                        (1.0 - (output_power / input_power).sqrt()).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    worker_shared
                        .activity
                        .store(activity.to_bits(), Ordering::Relaxed);
                    let _ = output.push(Frame {
                        sequence: frame.sequence,
                        samples: enhanced,
                    });
                }
            })?;
        match startup.recv() {
            Ok(Ok(())) => Ok(Self {
                input,
                results,
                shared,
                worker: Some(worker),
                frame: [0.0; FRAME_SAMPLES],
                output: [0.0; FRAME_SAMPLES],
                position: 0,
                sequence: 0,
                missed: 0,
                offline,
                delayed: [0.0; FRAME_SAMPLES],
            }),
            result => {
                let _ = worker.join();
                Err(StudioError::Model(match result {
                    Ok(Err(error)) => error,
                    _ => "worker stopped during initialization".into(),
                }))
            }
        }
    }
}

impl macafon_dsp::StudioProcessor for StudioNoiseSuppression {
    fn set(&mut self, params: &NoiseSuppressionParams) {
        self.shared
            .intensity_pct
            .store(params.intensity_pct.to_bits(), Ordering::Relaxed);
        self.shared
            .post_filter
            .store(params.post_filter, Ordering::Relaxed);
    }
    fn failure_reason(&self) -> Option<&str> {
        self.shared.failure.get().map(String::as_str)
    }
    fn activity(&self) -> f32 {
        f32::from_bits(self.shared.activity.load(Ordering::Relaxed))
    }
    fn overloaded(&self) -> bool {
        self.shared.overload.load(Ordering::Acquire)
    }

    fn process(&mut self, io: &mut [f32]) {
        for sample in io {
            self.frame[self.position] = sanitize(*sample).clamp(-1.0, 1.0);
            *sample = sanitize(self.output[self.position]);
            self.position += 1;
            if self.position != FRAME_SAMPLES {
                continue;
            }
            let frame = Frame {
                sequence: self.sequence,
                samples: self.frame,
            };
            let submitted = self.input.push(frame).is_ok();
            if self.offline {
                // This constructor is reserved for offline A/B analysis. The worker still
                // owns its non-Send tract model; waiting here preserves every input frame.
                loop {
                    if let Ok(result) = self.results.pop() {
                        self.output = self.delayed;
                        self.delayed = result.samples;
                        break;
                    }
                    if self.shared.stop.load(Ordering::Acquire) {
                        self.output.fill(0.0);
                        break;
                    }
                    thread::sleep(Duration::from_micros(250));
                }
                self.sequence = self.sequence.wrapping_add(1);
                self.position = 0;
                continue;
            }
            self.output.fill(0.0);
            let mut received = self.sequence == 0;
            if self.sequence > 0 {
                let expected = self.sequence - 1;
                // Discard late frames; replaying them would silently change the reported latency.
                while let Ok(frame) = self.results.peek() {
                    if frame.sequence > expected {
                        break;
                    }
                    if let Ok(frame) = self.results.pop() {
                        if frame.sequence == expected {
                            self.output = frame.samples;
                            received = true;
                            break;
                        }
                    }
                }
            }
            self.missed = if received && submitted {
                0
            } else {
                self.missed.saturating_add(1)
            };
            if self.missed >= 3 {
                self.shared.overload.store(true, Ordering::Release);
            }
            self.sequence = self.sequence.wrapping_add(1);
            self.position = 0;
        }
    }
}

impl Drop for StudioNoiseSuppression {
    fn drop(&mut self) {
        // Processors are retired to the control thread; never drop this from an audio callback.
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
