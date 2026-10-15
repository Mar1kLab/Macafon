//! Clock-drift compensation between a capture and a render device (PRD §12.2.1).
//!
//! On Windows the microphone and VB-CABLE run on different clocks. The capture thread writes
//! processed mono samples into an SPSC buffer; the render thread reads them through an adaptive
//! cubic resampler whose ratio is steered by a slow PI loop. The controlled level is this
//! buffer plus what the render device still has queued, i.e. the latency from capture to
//! playback: the render side may ask for one period or several at once (after its queue
//! drained), and the total stays the same either way. The loop is slow (≈ 20 s time
//! constant), so the pitch deviation (≤ 0.5 %) is inaudible and latency does not accumulate.
//! Both halves are RT-safe after construction.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

const SAMPLE_RATE_HZ: f64 = macafon_dsp::SAMPLE_RATE_HZ as f64;
/// Largest correction applied: well above the ±0.1 % drift expected between devices.
pub const MAX_RATIO_DEVIATION: f64 = 0.005;
/// Level smoothing; device periods make the instantaneous level a sawtooth.
const LEVEL_SMOOTHING_S: f64 = 0.5;
// PI loop: natural frequency 0.05 Hz, damping ≈ 1 (s² + fs·Kp·s + fs·Ki = 0).
const LOOP_OMEGA: f64 = 2.0 * std::f64::consts::PI * 0.05;
const KP: f64 = 2.0 * LOOP_OMEGA / SAMPLE_RATE_HZ;
const KI: f64 = LOOP_OMEGA * LOOP_OMEGA / SAMPLE_RATE_HZ;

/// Counters shared with the control loop (Doctor, latency report).
#[derive(Debug)]
pub struct DriftStats {
    level_frames: AtomicU32,
    min_fill_frames: AtomicU32,
    ratio_ppm: AtomicI32,
    underruns: AtomicU32,
    dropped_frames: AtomicU64,
    skipped_frames: AtomicU64,
}

impl Default for DriftStats {
    fn default() -> Self {
        Self {
            level_frames: AtomicU32::new(0),
            min_fill_frames: AtomicU32::new(u32::MAX),
            ratio_ppm: AtomicI32::new(0),
            underruns: AtomicU32::new(0),
            dropped_frames: AtomicU64::new(0),
            skipped_frames: AtomicU64::new(0),
        }
    }
}

impl DriftStats {
    /// Smoothed latency from capture to playback: this buffer plus the render queue.
    pub fn level_frames(&self) -> u32 {
        self.level_frames.load(Ordering::Relaxed)
    }

    /// Lowest fill of this buffer seen by the reader since the previous call: the margin left
    /// over device jitter. `None` before the first read.
    pub fn take_min_fill_frames(&self) -> Option<u32> {
        let min = self.min_fill_frames.swap(u32::MAX, Ordering::Relaxed);
        (min != u32::MAX).then_some(min)
    }

    /// Current correction in parts per million; positive when the reader consumes faster.
    pub fn ratio_ppm(&self) -> i32 {
        self.ratio_ppm.load(Ordering::Relaxed)
    }

    pub fn underruns(&self) -> u32 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Samples the writer could not store because the buffer was full.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    /// Samples the reader discarded to return to the target after a stall.
    pub fn skipped_frames(&self) -> u64 {
        self.skipped_frames.load(Ordering::Relaxed)
    }
}

/// Create a buffer that settles at `target_frames` of total latency (this buffer plus the
/// render queue). The target must exceed the render buffer size plus one capture packet,
/// otherwise a request that refills a drained render queue can underrun.
pub fn drift_buffer(target_frames: usize) -> (DriftWriter, DriftReader) {
    let target_frames = target_frames.max(1);
    // Room for retargeting (`set_target`) and for a stalled reader before samples drop.
    let capacity = (target_frames * 4 + 1024).max(macafon_dsp::SAMPLE_RATE_HZ as usize / 2);
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let stats = Arc::new(DriftStats::default());
    (
        DriftWriter {
            producer,
            stats: stats.clone(),
        },
        DriftReader {
            consumer,
            stats,
            target: target_frames as f64,
            skip_above: target_frames * 3,
            level: target_frames as f64,
            integral: 0.0,
            phase: 0.0,
            history: [0.0; 4],
            priming: true,
        },
    )
}

pub struct DriftWriter {
    producer: rtrb::Producer<f32>,
    stats: Arc<DriftStats>,
}

impl DriftWriter {
    /// RT-safe. Returns how many samples were stored; the rest are dropped and counted.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let count = samples.len().min(self.producer.slots());
        if let Ok(chunk) = self.producer.write_chunk_uninit(count) {
            chunk.fill_from_iter(samples.iter().copied());
        }
        let dropped = samples.len() - count;
        if dropped > 0 {
            self.stats
                .dropped_frames
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
        count
    }

    pub fn stats(&self) -> Arc<DriftStats> {
        self.stats.clone()
    }
}

pub struct DriftReader {
    consumer: rtrb::Consumer<f32>,
    stats: Arc<DriftStats>,
    target: f64,
    skip_above: usize,
    /// Smoothed level: buffered plus queued downstream, in frames.
    level: f64,
    integral: f64,
    /// Position between `history[1]` and `history[2]`, in input samples.
    phase: f64,
    history: [f32; 4],
    priming: bool,
}

impl DriftReader {
    /// RT-safe. Fills `out` with resampled mono audio; silence while (re)priming.
    /// `queued_frames` is what the render device has not played yet (its padding).
    pub fn read(&mut self, out: &mut [f32], queued_frames: usize) {
        let available = self.consumer.slots();
        if self.priming {
            if available + queued_frames < self.target as usize {
                out.fill(0.0);
                return;
            }
            self.priming = false;
            self.level = (available + queued_frames) as f64;
        }
        if available + queued_frames > self.skip_above {
            let excess = available + queued_frames - self.target as usize;
            self.skip(excess.min(available));
        }
        let available = self.consumer.slots();
        self.stats
            .min_fill_frames
            .fetch_min(available as u32, Ordering::Relaxed);
        let ratio = self.update_ratio(available + queued_frames, out.len());
        let wanted = (self.phase + out.len() as f64 * ratio).ceil() as usize + 1;
        let Ok(chunk) = self.consumer.read_chunk(wanted.min(available)) else {
            out.fill(0.0);
            return;
        };
        let (first, second) = chunk.as_slices();
        let mut input = first.iter().chain(second.iter()).copied();
        let mut consumed = 0;
        let mut written = out.len();
        'block: for (index, sample) in out.iter_mut().enumerate() {
            while self.phase >= 1.0 {
                let Some(next) = input.next() else {
                    written = index;
                    break 'block;
                };
                consumed += 1;
                self.history = [self.history[1], self.history[2], self.history[3], next];
                self.phase -= 1.0;
            }
            *sample = hermite(&self.history, self.phase as f32);
            self.phase += ratio;
        }
        chunk.commit(consumed);
        if written < out.len() {
            out[written..].fill(0.0);
            self.priming = true;
            self.integral = 0.0;
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn stats(&self) -> Arc<DriftStats> {
        self.stats.clone()
    }

    /// Change the target before streaming starts, once the render buffer size is known.
    pub fn set_target(&mut self, target_frames: usize) {
        let target_frames = target_frames.clamp(1, self.consumer.buffer().capacity() / 4);
        self.target = target_frames as f64;
        self.skip_above = target_frames * 3;
        self.level = self.target;
    }

    fn update_ratio(&mut self, level: usize, frames: usize) -> f64 {
        let dt = frames as f64 / SAMPLE_RATE_HZ;
        let alpha = (dt / LEVEL_SMOOTHING_S).min(1.0);
        self.level += alpha * (level as f64 - self.level);
        let error = self.level - self.target;
        let limit = MAX_RATIO_DEVIATION / KI;
        self.integral = (self.integral + error * dt).clamp(-limit, limit);
        let deviation =
            (KP * error + KI * self.integral).clamp(-MAX_RATIO_DEVIATION, MAX_RATIO_DEVIATION);
        self.stats
            .level_frames
            .store(self.level.round() as u32, Ordering::Relaxed);
        self.stats
            .ratio_ppm
            .store((deviation * 1e6).round() as i32, Ordering::Relaxed);
        1.0 + deviation
    }

    fn skip(&mut self, count: usize) {
        if let Ok(chunk) = self.consumer.read_chunk(count) {
            chunk.commit_all();
        }
        self.level = self.target;
        self.stats
            .skipped_frames
            .fetch_add(count as u64, Ordering::Relaxed);
    }
}

/// Catmull-Rom interpolation between `x[1]` and `x[2]`.
fn hermite(x: &[f32; 4], t: f32) -> f32 {
    let c1 = 0.5 * (x[2] - x[0]);
    let c2 = x[0] - 2.5 * x[1] + 2.0 * x[2] - 0.5 * x[3];
    let c3 = 0.5 * (x[3] - x[0]) + 1.5 * (x[1] - x[2]);
    ((c3 * t + c2) * t + c1) * t + x[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;

    /// Writer and reader on clocks `drift` apart, in blocks like WASAPI periods.
    fn simulate(drift: f64, seconds: f64, target: usize) -> (DriftWriter, DriftReader, Vec<f32>) {
        let (mut writer, mut reader) = drift_buffer(target);
        let mut output = Vec::new();
        let mut produced = 0.0;
        let mut phase = 0.0f64;
        let block = 480;
        let mut input = vec![0.0f32; 2048];
        let mut out = vec![0.0f32; block];
        let blocks = (seconds * SAMPLE_RATE_HZ / block as f64) as usize;
        for _ in 0..blocks {
            produced += block as f64 * (1.0 + drift);
            let count = produced.floor() as usize;
            produced -= count as f64;
            for sample in &mut input[..count] {
                *sample = (phase * std::f64::consts::TAU).sin() as f32 * 0.5;
                phase = (phase + 1000.0 / SAMPLE_RATE_HZ) % 1.0;
            }
            assert_no_alloc(|| {
                writer.write(&input[..count]);
                reader.read(&mut out, 0);
            });
            output.extend_from_slice(&out);
        }
        (writer, reader, output)
    }

    #[test]
    fn locks_onto_drift_without_underruns_or_latency_growth() {
        for drift in [-0.001, 0.001] {
            let target = 960;
            let (_writer, reader, _) = simulate(drift, 120.0, target);
            let stats = reader.stats();
            assert_eq!(stats.underruns(), 0, "drift {drift}");
            assert_eq!(stats.dropped_frames(), 0, "drift {drift}");
            assert_eq!(stats.skipped_frames(), 0, "drift {drift}");
            let level = stats.level_frames() as i64;
            assert!(
                (level - target as i64).abs() < 60,
                "drift {drift}: level {level}"
            );
            let ppm = stats.ratio_ppm() as f64;
            assert!((ppm - drift * 1e6).abs() < 50.0, "drift {drift}: {ppm} ppm");
        }
    }

    #[test]
    fn resampled_tone_stays_clean() {
        let (_writer, _reader, output) = simulate(0.001, 30.0, 960);
        // After settling, consecutive samples of a 1 kHz tone at 0.5 change by at most
        // 2π·1000/48000·0.5 ≈ 0.065; a click or a dropped block would exceed that.
        let settled = &output[48_000 * 5..];
        let max_step = settled
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 0.07, "max step {max_step}");
        let peak = settled
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
    }

    #[test]
    fn render_bursts_are_covered_by_the_render_queue() {
        // Render device with a 1056-frame buffer draining 480 per period; sometimes two
        // periods pass between wake-ups and it asks for 960 at once (seen on WASAPI).
        let buffer = 1056;
        let (mut writer, mut reader) = drift_buffer(buffer + 480 + 96);
        let input = [0.1f32; 480];
        let mut out = vec![0.0f32; buffer];
        let mut padding = buffer;
        for period in 0..20_000 {
            writer.write(&input);
            let drained = match period % 7 {
                3 => 0,
                4 => 960,
                _ => 480,
            };
            padding = padding.saturating_sub(drained);
            let request = buffer - padding;
            assert_no_alloc(|| reader.read(&mut out[..request], padding));
            padding = buffer;
        }
        assert_eq!(reader.stats().underruns(), 0);
        assert_eq!(reader.stats().skipped_frames(), 0);
    }

    #[test]
    fn stall_returns_to_target_instead_of_accumulating_latency() {
        let target = 960;
        let (mut writer, mut reader) = drift_buffer(target);
        let mut out = vec![0.0f32; 480];
        writer.write(&vec![0.1; target]);
        reader.read(&mut out, 0);
        // The render side stops for a while (e.g. its session was paused).
        writer.write(&vec![0.1; target * 3]);
        reader.read(&mut out, 0);
        assert!(reader.stats().skipped_frames() > 0);
        assert!(reader.consumer.slots() <= target);
    }

    #[test]
    fn underrun_reprimes_with_silence() {
        let (mut writer, mut reader) = drift_buffer(480);
        let mut out = vec![1.0f32; 480];
        reader.read(&mut out, 0);
        assert!(
            out.iter().all(|sample| *sample == 0.0),
            "silent while priming"
        );
        writer.write(&[0.25; 600]);
        reader.read(&mut out, 0);
        reader.read(&mut out, 0);
        assert_eq!(reader.stats().underruns(), 1);
        assert!(reader.priming);
    }
}
