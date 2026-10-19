//! The microphone chain of PRD §6.3 with meters (§6.6). Built outside the RT thread.

use crate::compressor::Compressor;
use crate::de_esser::DeEsser;
use crate::eq::Equalizer;
use crate::expander::Expander;
use crate::high_pass::HighPass;
use crate::limiter::Limiter;
use crate::noise::NoiseSuppression;
use crate::params::ParameterError;
use crate::settings::{ChainSettings, NoiseEngine};
use crate::util::{Ramp, SmoothedGain, lin_to_db};
use crate::{SAMPLE_RATE_HZ, sanitize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum ProcessorBuildError {
    #[error(transparent)]
    Invalid(#[from] ParameterError),
    #[error("Studio requires a prepared DeepFilterNet3 worker")]
    StudioUnavailable,
}

/// Observable chain boundaries; callbacks borrow samples only for the duration of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapPoint {
    Raw,
    Clean,
    Processed,
}

const METER_RMS_MS: f32 = 300.0;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RetuneError {
    #[error(transparent)]
    Invalid(#[from] ParameterError),
    /// Enabling/disabling modules, bypass, the NS engine or look-ahead change the topology or
    /// latency: build a new processor off the RT thread and swap it in.
    #[error("this change requires building a new processor")]
    RebuildRequired,
}

/// Lock-free meters written by the RT thread once per block (MTR-001…003, DSP-004).
///
/// Peaks and gain reductions hold the maximum since the previous `take`. Nothing is measured
/// until `set_enabled(true)`: meters cost nothing while no UI is watching (NFR-R7).
#[derive(Debug, Default)]
pub struct Telemetry {
    enabled: AtomicBool,
    studio_overloaded: AtomicBool,
    raw_peak: AtomicU32,
    input_peak: AtomicU32,
    input_rms: AtomicU32,
    output_peak: AtomicU32,
    output_rms: AtomicU32,
    output_loudness_power: AtomicU32,
    input_clipped: AtomicBool,
    output_clipped: AtomicBool,
    input_peak_hold: AtomicU32,
    output_peak_hold: AtomicU32,
    expander_reduction: AtomicU32,
    compressor_reduction: AtomicU32,
    de_esser_reduction: AtomicU32,
    limiter_reduction: AtomicU32,
    noise_activity: AtomicU32,
}

/// One meter reading. Levels in dBFS, reductions in dB (≤ 0), NS activity 0…1.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Meters {
    /// Physical microphone before Gain; ≥ 0 dBFS means it clipped before Macafon (MIC-G1).
    pub raw_peak_dbfs: f32,
    pub input_peak_dbfs: f32,
    pub input_rms_dbfs: f32,
    pub output_peak_dbfs: f32,
    pub output_rms_dbfs: f32,
    /// Ungated K-weighted mono loudness over the latest three seconds.
    #[cfg_attr(
        feature = "serde",
        serde(default = "silent_level", skip_serializing_if = "is_silent_level")
    )]
    pub output_lufs_short_term: f32,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub input_clipped: bool,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub output_clipped: bool,
    #[cfg_attr(
        feature = "serde",
        serde(default = "silent_level", skip_serializing_if = "is_silent_level")
    )]
    pub input_peak_hold_dbfs: f32,
    #[cfg_attr(
        feature = "serde",
        serde(default = "silent_level", skip_serializing_if = "is_silent_level")
    )]
    pub output_peak_hold_dbfs: f32,
    pub expander_reduction_db: f32,
    pub compressor_reduction_db: f32,
    pub de_esser_reduction_db: f32,
    pub limiter_reduction_db: f32,
    pub noise_activity: f32,
}

// Non-negative finite f32 values order like their bit patterns, so fetch_max works on bits.
fn raise(slot: &AtomicU32, value: f32) {
    slot.fetch_max(value.to_bits(), Ordering::Relaxed);
}

fn record(enabled: bool, slot: &AtomicU32, value: f32) {
    if enabled {
        raise(slot, value);
    }
}

fn take(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.swap(0, Ordering::Relaxed))
}

fn load(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}

impl Telemetry {
    /// Worker overload or three consecutive missed Studio deadlines. Read on control thread.
    pub fn studio_overloaded(&self) -> bool {
        self.studio_overloaded.load(Ordering::Acquire)
    }
    pub fn clear_studio_overload(&self) {
        self.studio_overloaded.store(false, Ordering::Release);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn take(&self) -> Meters {
        Meters {
            raw_peak_dbfs: lin_to_db(take(&self.raw_peak)),
            input_peak_dbfs: lin_to_db(take(&self.input_peak)),
            input_rms_dbfs: lin_to_db(load(&self.input_rms)),
            output_peak_dbfs: lin_to_db(take(&self.output_peak)),
            output_rms_dbfs: lin_to_db(load(&self.output_rms)),
            output_lufs_short_term: (-0.691
                + 10.0 * load(&self.output_loudness_power).max(1e-12).log10())
            .max(-120.0),
            input_clipped: self.input_clipped.swap(false, Ordering::Relaxed),
            output_clipped: self.output_clipped.swap(false, Ordering::Relaxed),
            input_peak_hold_dbfs: lin_to_db(load(&self.input_peak_hold)),
            output_peak_hold_dbfs: lin_to_db(load(&self.output_peak_hold)),
            expander_reduction_db: -take(&self.expander_reduction),
            compressor_reduction_db: -take(&self.compressor_reduction),
            de_esser_reduction_db: -take(&self.de_esser_reduction),
            limiter_reduction_db: -take(&self.limiter_reduction),
            noise_activity: take(&self.noise_activity),
        }
    }
}

struct LevelMeter {
    mean_square: f32,
    peak_hold: f32,
    hold_remaining: usize,
}

impl LevelMeter {
    fn measure(
        &mut self,
        block: &[f32],
        peak: &AtomicU32,
        rms: &AtomicU32,
        hold: &AtomicU32,
        clipped: &AtomicBool,
    ) {
        let (mut max, mut sum) = (0.0f32, 0.0f32);
        for sample in block {
            max = max.max(sample.abs());
            sum += sample * sample;
        }
        let samples = block.len() as f32;
        let alpha = 1.0 - (-samples / (METER_RMS_MS * 0.001 * SAMPLE_RATE_HZ as f32)).exp();
        self.mean_square += alpha * (sum / samples - self.mean_square);
        if max >= self.peak_hold || self.hold_remaining <= block.len() {
            self.peak_hold = max;
            self.hold_remaining = SAMPLE_RATE_HZ as usize;
        } else {
            self.hold_remaining -= block.len();
        }
        hold.store(sanitize(self.peak_hold).to_bits(), Ordering::Relaxed);
        if max >= 1.0 {
            clipped.store(true, Ordering::Relaxed);
        }
        raise(peak, sanitize(max));
        rms.store(
            sanitize(self.mean_square.sqrt()).to_bits(),
            Ordering::Relaxed,
        );
    }
}

pub struct MicrophoneProcessor {
    settings: ChainSettings,
    gain: Option<SmoothedGain>,
    high_pass: Option<HighPass>,
    noise: Option<NoiseSuppression>,
    expander: Option<Expander>,
    eq: Option<Equalizer>,
    compressor: Option<Compressor>,
    de_esser: Option<DeEsser>,
    limiter: Option<Limiter>,
    output: SmoothedGain,
    mute: Ramp<1>,
    input_meter: LevelMeter,
    output_meter: LevelMeter,
    loudness: crate::loudness::ShortTermLoudness,
    telemetry: Arc<Telemetry>,
}

impl MicrophoneProcessor {
    pub fn new(settings: &ChainSettings) -> Result<Self, ProcessorBuildError> {
        Self::with_telemetry(settings, Arc::default())
    }

    /// Successive processors of one microphone share meters across swaps.
    pub fn with_telemetry(
        settings: &ChainSettings,
        telemetry: Arc<Telemetry>,
    ) -> Result<Self, ProcessorBuildError> {
        Self::with_studio_processor(settings, telemetry, None)
    }

    /// Prepared Studio worker is injected by the engine; this crate stays lightweight.
    pub fn with_studio_processor(
        settings: &ChainSettings,
        telemetry: Arc<Telemetry>,
        studio: Option<Box<dyn crate::StudioProcessor>>,
    ) -> Result<Self, ProcessorBuildError> {
        settings.validate()?;
        let active = !settings.bypass;
        let build = |enabled: bool| active && enabled;
        let s = settings;
        let noise = if build(s.noise_suppression.enabled) {
            Some(NoiseSuppression::new(&s.noise_suppression.params, studio)?)
        } else {
            None
        };
        Ok(Self {
            settings: *settings,
            gain: build(s.gain.enabled).then(|| SmoothedGain::new(s.gain.params.gain_db)),
            high_pass: build(s.high_pass.enabled).then(|| HighPass::new(&s.high_pass.params)),
            noise,
            expander: build(s.expander.enabled).then(|| Expander::new(&s.expander.params)),
            eq: build(s.eq.enabled).then(|| Equalizer::new(&s.eq.params)),
            compressor: build(s.compressor.enabled).then(|| Compressor::new(&s.compressor.params)),
            de_esser: build(s.de_esser.enabled).then(|| DeEsser::new(&s.de_esser.params)),
            limiter: build(s.limiter.enabled).then(|| Limiter::new(&s.limiter.params)),
            output: SmoothedGain::new(if active { s.output_gain_db } else { 0.0 }),
            mute: Ramp::new([if s.muted { 0.0 } else { 1.0 }]),
            input_meter: LevelMeter {
                mean_square: 0.0,
                peak_hold: 0.0,
                hold_remaining: 0,
            },
            output_meter: LevelMeter {
                mean_square: 0.0,
                peak_hold: 0.0,
                hold_remaining: 0,
            },
            loudness: crate::loudness::ShortTermLoudness::new(),
            telemetry,
        })
    }

    pub fn settings(&self) -> &ChainSettings {
        &self.settings
    }

    /// The engine actually running. Studio initializes the embedded model before construction.
    pub fn noise_engine(&self) -> Option<NoiseEngine> {
        self.noise.as_ref().map(NoiseSuppression::engine)
    }

    pub fn noise_error(&self) -> Option<&str> {
        self.noise
            .as_ref()
            .and_then(NoiseSuppression::failure_reason)
    }

    pub fn telemetry(&self) -> Arc<Telemetry> {
        self.telemetry.clone()
    }

    /// Algorithmic latency of the enabled modules; mute does not change it.
    pub fn latency_samples(&self) -> u32 {
        self.noise
            .as_ref()
            .map_or(0, NoiseSuppression::latency_samples)
            + self.limiter.as_ref().map_or(0, Limiter::latency_samples)
    }

    /// Apply continuous parameters and mute with smoothing. RT-safe: no allocation or locks.
    pub fn retune(&mut self, settings: &ChainSettings) -> Result<(), RetuneError> {
        settings.validate()?;
        if self.settings.requires_rebuild(settings) {
            return Err(RetuneError::RebuildRequired);
        }
        self.settings = *settings;
        let s = settings;
        if let Some(gain) = &mut self.gain {
            gain.set_db(s.gain.params.gain_db);
        }
        if let Some(module) = &mut self.high_pass {
            module.set(&s.high_pass.params);
        }
        if let Some(module) = &mut self.noise {
            module.set(&s.noise_suppression.params);
        }
        if let Some(module) = &mut self.expander {
            module.set(&s.expander.params);
        }
        if let Some(module) = &mut self.eq {
            module.set(&s.eq.params);
        }
        if let Some(module) = &mut self.compressor {
            module.set(&s.compressor.params);
        }
        if let Some(module) = &mut self.de_esser {
            module.set(&s.de_esser.params);
        }
        if let Some(module) = &mut self.limiter {
            module.set(&s.limiter.params);
        }
        if !s.bypass {
            self.output.set_db(s.output_gain_db);
        }
        self.mute.set_target([if s.muted { 0.0 } else { 1.0 }]);
        Ok(())
    }

    #[inline]
    pub fn process(&mut self, io: &mut [f32]) {
        self.process_inner::<false>(io, &mut |_, _| {});
    }

    /// RT-safe borrowed observation. A tap runs even when the final output is muted.
    #[inline]
    pub fn process_with_tap(&mut self, io: &mut [f32], tap: &mut impl FnMut(TapPoint, &[f32])) {
        self.process_inner::<true>(io, tap);
    }

    #[inline]
    fn process_inner<const TAPPING: bool>(
        &mut self,
        io: &mut [f32],
        tap: &mut impl FnMut(TapPoint, &[f32]),
    ) {
        if io.is_empty() {
            return;
        }
        let t = &*self.telemetry;
        let metering = t.enabled.load(Ordering::Relaxed);
        let silenced = !self.mute.is_ramping() && self.mute.value()[0] == 0.0;
        // Muted costs nothing, unless someone watches the meters: the input stays visible
        // while the output is digital silence (MIC-013, MTR-001).
        if silenced && !metering && !TAPPING {
            io.fill(0.0);
            return;
        }
        let mut raw_peak = 0.0f32;
        for sample in io.iter_mut() {
            *sample = sanitize(*sample);
            raw_peak = raw_peak.max(sample.abs());
        }
        record(metering, &t.raw_peak, raw_peak);
        if let Some(gain) = &mut self.gain {
            gain.process(io);
        }
        if TAPPING {
            tap(TapPoint::Raw, io);
        }
        if metering {
            self.input_meter.measure(
                io,
                &t.input_peak,
                &t.input_rms,
                &t.input_peak_hold,
                &t.input_clipped,
            );
        }
        if let Some(module) = &mut self.high_pass {
            module.process(io);
        }
        if let Some(module) = &mut self.noise {
            module.process(io);
            record(metering, &t.noise_activity, module.activity());
            if module.overloaded() {
                t.studio_overloaded.store(true, Ordering::Release);
            }
        }
        if TAPPING {
            tap(TapPoint::Clean, io);
        }
        if let Some(module) = &mut self.expander {
            module.process(io);
            record(metering, &t.expander_reduction, -module.reduction_db());
        }
        if let Some(module) = &mut self.eq {
            module.process(io);
        }
        if let Some(module) = &mut self.compressor {
            module.process(io);
            record(metering, &t.compressor_reduction, -module.reduction_db());
        }
        if let Some(module) = &mut self.de_esser {
            module.process(io);
            record(metering, &t.de_esser_reduction, -module.reduction_db());
        }
        if let Some(module) = &mut self.limiter {
            module.process(io);
            record(metering, &t.limiter_reduction, -module.reduction_db());
        }
        self.output.process(io);
        if TAPPING {
            tap(TapPoint::Processed, io);
        }
        if silenced {
            io.fill(0.0);
        } else if self.mute.is_ramping() || self.mute.value()[0] != 1.0 {
            io.iter_mut()
                .for_each(|sample| *sample *= self.mute.next()[0]);
        }
        let mut poisoned = false;
        for sample in io.iter_mut() {
            poisoned |= !sample.is_finite();
            *sample = sanitize(*sample);
        }
        if poisoned {
            self.reset();
        }
        let t = &*self.telemetry;
        if metering {
            self.output_meter.measure(
                io,
                &t.output_peak,
                &t.output_rms,
                &t.output_peak_hold,
                &t.output_clipped,
            );
            t.output_loudness_power
                .store(self.loudness.measure(io).to_bits(), Ordering::Relaxed);
        }
    }

    /// Clear filter and envelope state after invalid output (NFR-S4). RNNoise keeps its state:
    /// its input is sanitised and clamped, and rebuilding it would allocate.
    fn reset(&mut self) {
        if let Some(module) = &mut self.high_pass {
            module.reset();
        }
        if let Some(module) = &mut self.expander {
            module.reset();
        }
        if let Some(module) = &mut self.eq {
            module.reset();
        }
        if let Some(module) = &mut self.compressor {
            module.reset();
        }
        if let Some(module) = &mut self.de_esser {
            module.reset();
        }
        if let Some(module) = &mut self.limiter {
            module.reset();
        }
    }
}

#[cfg(feature = "serde")]
fn silent_level() -> f32 {
    -120.0
}
#[cfg(feature = "serde")]
fn is_silent_level(value: &f32) -> bool {
    *value <= -120.0
}
#[cfg(feature = "serde")]
fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tap_tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    #[test]
    fn borrowed_taps_are_allocation_free_bit_exact_and_pre_mute() {
        let settings = crate::Preset::Broadcast.settings();
        let mut plain = MicrophoneProcessor::new(&settings).expect("processor");
        let mut tapped = MicrophoneProcessor::new(&settings).expect("processor");
        let mut a = [0.1; 480];
        let mut b = a;
        let mut counts = [0; 3];
        assert_no_alloc(|| {
            plain.process(&mut a);
            tapped.process_with_tap(&mut b, &mut |point, samples| {
                counts[match point {
                    TapPoint::Raw => 0,
                    TapPoint::Clean => 1,
                    TapPoint::Processed => 2,
                }] += samples.len();
            });
        });
        assert_eq!(a, b);
        assert_eq!(counts, [480; 3]);
        let mut settings = ChainSettings::raw();
        settings.muted = true;
        let mut muted = MicrophoneProcessor::new(&settings).expect("muted processor");
        let mut signal = [0.25; 480];
        let mut before_mute = 0.0;
        assert_no_alloc(|| {
            muted.process_with_tap(&mut signal, &mut |point, samples| {
                if point == TapPoint::Processed {
                    before_mute = samples[0];
                }
            })
        });
        assert_eq!(before_mute, 0.25);
        assert!(signal.iter().all(|x| *x == 0.0));
    }
}
