//! Added-latency measurement (NFR-L6, WIN-MIC-008): the physical microphone and the virtual
//! one (`CABLE Output`) are recorded at the same time, both timelines are anchored to the QPC
//! timestamps WASAPI gives every packet, and the processed signal is located in the direct
//! one by cross-correlation. Both streams start from the same device samples, so even the
//! microphone's self-noise correlates; the result is what an app gains or loses by recording
//! Macafon instead of the microphone. Not real-time code: runs on its own threads.

use crate::Error;
use crate::com::ComGuard;
use crate::wasapi::{HNS_PER_SECOND, Stream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient};

const RATE_HZ: f64 = macafon_dsp::SAMPLE_RATE_HZ as f64;
/// Skip start-up transients of both streams.
const SETTLE_S: f64 = 0.5;
const WINDOW_SAMPLES: usize = 24_000;
/// Longest latency searched for.
pub const MAX_LATENCY_MS: f64 = 250.0;
/// Below this the peak is not trusted (silence, a muted chain or a different signal).
const MIN_CORRELATION: f64 = 0.5;

#[derive(Debug, Clone, Copy)]
pub struct LatencyMeasurement {
    pub added_latency_ms: f64,
    /// Normalised cross-correlation at the peak, 0…1.
    pub correlation: f64,
    pub direct_rms_dbfs: f64,
}

#[derive(Default)]
struct Recording {
    samples: Vec<f32>,
    /// (index of the packet's first sample, its QPC time in 100 ns units).
    packets: Vec<(usize, u64)>,
}

impl Recording {
    /// QPC time of sample `index`, from the packet it belongs to.
    fn time_of(&self, index: usize) -> Option<f64> {
        let packet = self.packets.partition_point(|(start, _)| *start <= index);
        let (start, qpc) = *self.packets.get(packet.checked_sub(1)?)?;
        Some(qpc as f64 + (index - start) as f64 * HNS_PER_SECOND / RATE_HZ)
    }

    /// Fractional sample index recorded at QPC time `time`.
    fn index_at(&self, time: f64) -> Option<f64> {
        let packet = self
            .packets
            .partition_point(|(_, qpc)| (*qpc as f64) <= time);
        let (start, qpc) = *self.packets.get(packet.checked_sub(1)?)?;
        Some(start as f64 + (time - qpc as f64) * RATE_HZ / HNS_PER_SECOND)
    }
}

/// Record `direct_id` and `processed_id` for `seconds` and find the delay between them.
/// The processing chain should be raw (no suppression), otherwise the shapes differ.
pub fn measure_added_latency(
    direct_id: &str,
    processed_id: &str,
    seconds: f64,
) -> Result<LatencyMeasurement, Error> {
    let stop = Arc::new(AtomicBool::new(false));
    let record = |id: String| {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("macafon-probe".into())
            .spawn(move || record(&id, seconds + 1.0, &stop))
            .map_err(|error| Error::Thread(error.to_string()))
    };
    let direct = record(direct_id.to_owned())?;
    let processed = record(processed_id.to_owned())?;
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < deadline && !direct.is_finished() && !processed.is_finished() {
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.store(true, Ordering::Release);
    let join = |thread: std::thread::JoinHandle<Result<Recording, Error>>| {
        thread
            .join()
            .unwrap_or_else(|_| Err(Error::Thread("probe thread panicked".into())))
    };
    let (direct, processed) = (join(direct)?, join(processed)?);
    analyse(&direct, &processed, MAX_LATENCY_MS)
}

fn record(device_id: &str, max_seconds: f64, stop: &AtomicBool) -> Result<Recording, Error> {
    let _com = ComGuard::new()?;
    let stream = Stream::open(device_id, 1, 0)?;
    // SAFETY: the client was initialised by Stream::open.
    let client: IAudioCaptureClient = unsafe { stream.client.GetService() }?;
    let capacity = (max_seconds * RATE_HZ) as usize;
    let mut recording = Recording {
        samples: Vec::with_capacity(capacity),
        packets: Vec::with_capacity(capacity / 64),
    };
    // SAFETY: starting an initialised client.
    unsafe { stream.client.Start() }?;
    while !stop.load(Ordering::Acquire) && recording.samples.len() < capacity {
        if !stream.wait(200) {
            continue;
        }
        // SAFETY: GetBuffer/ReleaseBuffer pairs; the packet holds `frames` mono floats.
        unsafe {
            while client.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let (mut frames, mut flags, mut qpc) = (0, 0, 0);
                client.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))?;
                recording.packets.push((recording.samples.len(), qpc));
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    let len = recording.samples.len();
                    recording.samples.resize(len + frames as usize, 0.0);
                } else {
                    recording
                        .samples
                        .extend_from_slice(std::slice::from_raw_parts(
                            data.cast::<f32>(),
                            frames as usize,
                        ));
                }
                client.ReleaseBuffer(frames)?;
            }
        }
    }
    Ok(recording)
}

fn analyse(
    direct: &Recording,
    processed: &Recording,
    max_latency_ms: f64,
) -> Result<LatencyMeasurement, Error> {
    let fail = |reason: &str| Err(Error::Format(format!("latency probe: {reason}")));
    let max_lag = (max_latency_ms * RATE_HZ / 1000.0) as usize;
    let start = (SETTLE_S * RATE_HZ) as usize;
    let Some(window) = direct.samples.get(start..start + WINDOW_SAMPLES) else {
        return fail("recording too short");
    };
    // Where the processed stream was at the moment the window started in the direct one.
    let Some(origin) = direct
        .time_of(start)
        .and_then(|time| processed.index_at(time))
    else {
        return fail("the virtual microphone delivered no audio");
    };
    let origin = origin.round();
    let energy = |x: &[f32]| x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>();
    let window_energy = energy(window);
    let rms = (window_energy / window.len() as f64).sqrt();
    if rms < 1e-6 {
        return fail("the microphone is silent");
    }
    // Small negative lags too: a result below zero means the timestamps are not comparable.
    let min_lag = -480i64;
    let mut best = (0i64, 0.0f64);
    for lag in min_lag..max_lag as i64 {
        let offset = origin as i64 + lag;
        let Some(other) = usize::try_from(offset)
            .ok()
            .and_then(|offset| processed.samples.get(offset..offset + WINDOW_SAMPLES))
        else {
            continue;
        };
        let dot: f64 = window
            .iter()
            .zip(other)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum();
        let norm = (window_energy * energy(other)).sqrt();
        let correlation = if norm > 0.0 { dot / norm } else { 0.0 };
        if correlation > best.1 {
            best = (lag, correlation);
        }
    }
    if best.1 < MIN_CORRELATION {
        return fail(&format!(
            "no matching signal (best correlation {:.2}); use the raw preset",
            best.1
        ));
    }
    Ok(LatencyMeasurement {
        added_latency_ms: best.0 as f64 * 1000.0 / RATE_HZ,
        correlation: best.1,
        direct_rms_dbfs: 20.0 * rms.log10(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(samples: Vec<f32>, first_qpc: u64) -> Recording {
        let packets = (0..samples.len())
            .step_by(480)
            .map(|index| {
                (
                    index,
                    first_qpc + (index as f64 * HNS_PER_SECOND / RATE_HZ) as u64,
                )
            })
            .collect();
        Recording { samples, packets }
    }

    #[test]
    fn finds_delay_across_differently_started_streams() {
        // Pseudo-random noise; the processed stream started 70 ms later and carries the
        // signal 23 ms (1104 samples) late.
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..96_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        let started_late: usize = 3360;
        let delay = 1104;
        let processed: Vec<f32> = (0..90_000)
            .map(|i| {
                (i + started_late)
                    .checked_sub(delay)
                    .map_or(0.0, |j| noise[j] * 0.5)
            })
            .collect();
        let qpc = 1_000_000_000;
        let started_late_hns = (started_late as f64 * HNS_PER_SECOND / RATE_HZ) as u64;
        let result = analyse(
            &recording(noise, qpc),
            &recording(processed, qpc + started_late_hns),
            40.0,
        )
        .expect("measured");
        assert!((result.added_latency_ms - 23.0).abs() < 0.05, "{result:?}");
        assert!(result.correlation > 0.99);
    }
}
