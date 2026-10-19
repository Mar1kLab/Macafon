//! Lock-free control of the real-time microphone chain (PRD §11.4, NFR-S2).
//!
//! `MicControl` lives on the control thread, `MicRt` inside the audio callback. Parameter
//! changes travel through an SPSC queue and are retuned in place; structural changes build a
//! new processor off the RT thread, which `MicRt` crossfades in. Replaced processors return
//! through a second queue so they are freed outside the RT thread.
//!
//! While armed, `MicRt` also copies the raw input (before the chain) into a third queue for
//! calibration and the A/B recorder (PRD §6.4, §6.5).

mod ab;
mod channel;
mod dictation;
mod drift;
pub use dictation::DictationReader;

pub use ab::{AbControl, AbRt, SWITCH_FADE_SAMPLES, ab_player};
pub use channel::{ChannelControl, ChannelRt, channel};
pub use drift::{DriftReader, DriftStats, DriftWriter, MAX_RATIO_DEVIATION, drift_buffer};

use macafon_dsp::{
    ChainSettings, MicrophoneProcessor, ParameterError, ProcessorBuildError, Telemetry,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Crossfade between an old and a rebuilt chain: 10 ms (DSP-003).
pub const SWAP_FADE_SAMPLES: u32 = macafon_dsp::SAMPLE_RATE_HZ / 100;
const QUEUE_CAPACITY: usize = 64;
const GARBAGE_CAPACITY: usize = 8;
const SCRATCH_SAMPLES: usize = 1024;
/// Raw capture buffer: one second, drained by the control loop every 100 ms.
const CAPTURE_SAMPLES: usize = macafon_dsp::SAMPLE_RATE_HZ as usize;

// Settings travel inline: popping a slot must not free memory on the RT thread.
#[allow(clippy::large_enum_variant)]
enum Command {
    Retune(ChainSettings),
    Dictation(Option<Box<dictation::DictationTap>>),
    Replace(Box<MicrophoneProcessor>),
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Invalid(#[from] ParameterError),
    #[error("the audio thread is not consuming changes; try again")]
    Busy,
    #[error(transparent)]
    Build(#[from] ProcessorBuildError),
    #[error(transparent)]
    Studio(#[from] macafon_studio::StudioError),
}

/// The control loop did not drain the raw capture in time; samples were lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("raw capture overflowed")]
pub struct CaptureOverrun;

#[derive(Default)]
struct CaptureFlags {
    armed: AtomicBool,
    overrun: AtomicBool,
}

/// Build a complete microphone processor, including an embedded Studio worker when selected.
pub fn prepare_processor(
    settings: &ChainSettings,
    telemetry: Arc<Telemetry>,
) -> Result<MicrophoneProcessor, EngineError> {
    prepare_with_studio(
        settings,
        telemetry,
        macafon_studio::StudioNoiseSuppression::new,
    )
}

/// Offline analysis only: waits for each Studio frame, preserving the live model and latency.
/// The returned processor must never be placed in `MicRt` or an audio callback.
pub fn prepare_offline_processor(
    settings: &ChainSettings,
    telemetry: Arc<Telemetry>,
) -> Result<MicrophoneProcessor, EngineError> {
    prepare_with_studio(
        settings,
        telemetry,
        macafon_studio::StudioNoiseSuppression::offline,
    )
}

fn prepare_with_studio(
    settings: &ChainSettings,
    telemetry: Arc<Telemetry>,
    build_studio: impl FnOnce(
        &macafon_dsp::settings::NoiseSuppressionParams,
    ) -> Result<
        macafon_studio::StudioNoiseSuppression,
        macafon_studio::StudioError,
    >,
) -> Result<MicrophoneProcessor, EngineError> {
    settings.validate()?;
    let studio: Option<Box<dyn macafon_dsp::StudioProcessor>> = if !settings.bypass
        && settings.noise_suppression.enabled
        && settings.noise_suppression.params.engine == macafon_dsp::settings::NoiseEngine::Studio
    {
        Some(Box::new(build_studio(&settings.noise_suppression.params)?))
    } else {
        None
    };
    Ok(MicrophoneProcessor::with_studio_processor(
        settings, telemetry, studio,
    )?)
}

/// Create a connected control/RT pair for one microphone chain. Pass the same telemetry to
/// successive pairs (e.g. after a PipeWire restart) so meter readers keep working.
pub fn microphone(
    settings: &ChainSettings,
    telemetry: Arc<Telemetry>,
) -> Result<(MicControl, MicRt), EngineError> {
    let processor = prepare_processor(settings, telemetry.clone())?;
    let (commands, command_queue) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
    let (garbage, garbage_queue) = rtrb::RingBuffer::new(GARBAGE_CAPACITY);
    let (capture, capture_queue) = rtrb::RingBuffer::new(CAPTURE_SAMPLES);
    let capture_flags = Arc::new(CaptureFlags::default());
    let (tap_garbage, tap_retired) = rtrb::RingBuffer::new(8);
    let dictation_mute = Arc::new(AtomicBool::new(false));
    Ok((
        MicControl {
            settings: *settings,
            over_budget_cycles: 0,
            commands,
            garbage: garbage_queue,
            telemetry,
            capture: capture_queue,
            tap_retired,
            dictation_mute: dictation_mute.clone(),
            capture_flags: capture_flags.clone(),
        },
        MicRt {
            current: Box::new(processor),
            outgoing: None,
            retired: None,
            fade_position: 0,
            fade_samples: SWAP_FADE_SAMPLES,
            warmup_remaining: 0,
            scratch: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
            commands: command_queue,
            garbage,
            capture,
            capture_flags,
            tap: None,
            tap_garbage,
            retired_tap: None,
            dictation_mute,
        },
    ))
}

pub struct MicControl {
    settings: ChainSettings,
    over_budget_cycles: u8,
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Box<MicrophoneProcessor>>,
    telemetry: Arc<Telemetry>,
    capture: rtrb::Consumer<f32>,
    tap_retired: rtrb::Consumer<Box<dictation::DictationTap>>,
    dictation_mute: Arc<AtomicBool>,
    capture_flags: Arc<CaptureFlags>,
}

impl MicControl {
    /// Allocate while enabling; removed taps return for off-thread destruction.
    pub fn enable_dictation(
        &mut self,
        pre_roll: bool,
        point: macafon_dsp::TapPoint,
    ) -> Result<DictationReader, EngineError> {
        while self.tap_retired.pop().is_ok() {}
        let (reader, tap) = dictation::tap(pre_roll, point);
        self.commands
            .push(Command::Dictation(Some(tap)))
            .map_err(|_| EngineError::Busy)?;
        Ok(reader)
    }
    pub fn disable_dictation(&mut self) -> Result<(), EngineError> {
        self.dictation_mute.store(false, Ordering::Release);
        self.commands
            .push(Command::Dictation(None))
            .map_err(|_| EngineError::Busy)
    }
    /// Independent gate: microphone settings and an existing mute remain untouched.
    pub fn mute_for_dictation(&self, mute: bool) {
        self.dictation_mute.store(mute, Ordering::Release);
    }

    /// Settings most recently sent to the audio thread.
    pub fn settings(&self) -> &ChainSettings {
        &self.settings
    }

    pub fn latency_samples(&self) -> u32 {
        self.settings.latency_samples()
    }

    pub fn telemetry(&self) -> Arc<Telemetry> {
        self.telemetry.clone()
    }

    /// Call from the control loop; inference timing is measured by the Studio worker.
    /// Returns true exactly when a Lite fallback is queued (caller emits the UI event).
    pub fn poll_studio_fallback(&mut self) -> Result<bool, EngineError> {
        if !self.telemetry.studio_overloaded() {
            return Ok(false);
        }
        self.fallback_to_lite()
    }

    /// Optional external cycle measurements, collected outside the audio callback.
    pub fn observe_processing_budget(
        &mut self,
        elapsed_ns: u64,
        budget_ns: u64,
    ) -> Result<bool, EngineError> {
        if budget_ns == 0 {
            return Ok(false);
        }
        self.over_budget_cycles = if u128::from(elapsed_ns) * 5 > u128::from(budget_ns) * 4 {
            self.over_budget_cycles.saturating_add(1)
        } else {
            0
        };
        if self.over_budget_cycles < 3 {
            return Ok(false);
        }
        self.fallback_to_lite()
    }

    fn fallback_to_lite(&mut self) -> Result<bool, EngineError> {
        use macafon_dsp::settings::NoiseEngine;
        if !self.settings.noise_suppression.enabled
            || self.settings.noise_suppression.params.engine != NoiseEngine::Studio
        {
            self.telemetry.clear_studio_overload();
            return Ok(false);
        }
        let mut settings = self.settings;
        settings.noise_suppression.params.engine = NoiseEngine::Lite;
        self.apply(&settings)?;
        self.over_budget_cycles = 0;
        self.telemetry.clear_studio_overload();
        Ok(true)
    }

    /// Send new settings to the audio thread: retuned in place, or rebuilt and crossfaded.
    pub fn apply(&mut self, settings: &ChainSettings) -> Result<(), EngineError> {
        settings.validate()?;
        self.collect_garbage();
        if self.commands.is_full() {
            return Err(EngineError::Busy);
        }
        let rebuild = self.settings.requires_rebuild(settings);
        let command = if rebuild {
            Command::Replace(Box::new(prepare_processor(
                settings,
                self.telemetry.clone(),
            )?))
        } else {
            Command::Retune(*settings)
        };
        self.commands.push(command).map_err(|_| EngineError::Busy)?;
        if rebuild {
            self.over_budget_cycles = 0;
        }
        self.settings = *settings;
        Ok(())
    }

    /// Start copying the raw input; anything left from an earlier capture is discarded.
    pub fn start_capture(&mut self) {
        self.stop_capture();
        let stale = self.capture.slots();
        if let Ok(chunk) = self.capture.read_chunk(stale) {
            chunk.commit_all();
        }
        self.capture_flags.overrun.store(false, Ordering::Relaxed);
        self.capture_flags.armed.store(true, Ordering::Release);
    }

    pub fn stop_capture(&mut self) {
        self.capture_flags.armed.store(false, Ordering::Release);
    }

    /// Append captured raw samples to `samples`. Call at least every few hundred ms.
    pub fn read_capture(&mut self, samples: &mut Vec<f32>) -> Result<(), CaptureOverrun> {
        let available = self.capture.slots();
        if let Ok(chunk) = self.capture.read_chunk(available) {
            let (first, second) = chunk.as_slices();
            samples.extend_from_slice(first);
            samples.extend_from_slice(second);
            chunk.commit_all();
        }
        if self.capture_flags.overrun.load(Ordering::Relaxed) {
            Err(CaptureOverrun)
        } else {
            Ok(())
        }
    }

    /// Free processors the audio thread has swapped out.
    pub fn collect_garbage(&mut self) {
        while self.tap_retired.pop().is_ok() {}
        while let Ok(processor) = self.garbage.pop() {
            drop(processor);
        }
    }
}

pub struct MicRt {
    current: Box<MicrophoneProcessor>,
    outgoing: Option<Box<MicrophoneProcessor>>,
    retired: Option<Box<MicrophoneProcessor>>,
    fade_position: u32,
    fade_samples: u32,
    warmup_remaining: u32,
    scratch: Box<[f32]>,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Box<MicrophoneProcessor>>,
    capture: rtrb::Producer<f32>,
    tap: Option<Box<dictation::DictationTap>>,
    tap_garbage: rtrb::Producer<Box<dictation::DictationTap>>,
    retired_tap: Option<Box<dictation::DictationTap>>,
    dictation_mute: Arc<AtomicBool>,
    capture_flags: Arc<CaptureFlags>,
}

impl MicRt {
    pub fn latency_samples(&self) -> u32 {
        self.current.latency_samples()
    }

    /// RT-safe: no allocation, locks or deallocation.
    pub fn process(&mut self, io: &mut [f32]) {
        self.drain();
        if self.capture_flags.armed.load(Ordering::Acquire) {
            self.capture(io);
        }
        if self.outgoing.is_none() {
            if let Some(tap) = &mut self.tap {
                self.current.process_with_tap(io, &mut |point, audio| {
                    if point == tap.point {
                        tap.process(audio);
                    }
                });
            } else {
                self.current.process(io);
            }
            if self.dictation_mute.load(Ordering::Acquire) {
                io.fill(0.0);
            }
            return;
        }
        for chunk in io.chunks_mut(SCRATCH_SAMPLES) {
            let Some(old) = &mut self.outgoing else {
                if let Some(tap) = &mut self.tap {
                    self.current.process_with_tap(chunk, &mut |point, audio| {
                        if point == tap.point {
                            tap.process(audio);
                        }
                    });
                } else {
                    self.current.process(chunk);
                }
                continue;
            };
            let previous = &mut self.scratch[..chunk.len()];
            previous.copy_from_slice(chunk);
            old.process(previous);
            if let Some(tap) = &mut self.tap {
                self.current.process_with_tap(chunk, &mut |point, audio| {
                    if point == tap.point {
                        tap.process(audio);
                    }
                });
            } else {
                self.current.process(chunk);
            }
            for (sample, old_sample) in chunk.iter_mut().zip(previous.iter()) {
                if self.warmup_remaining > 0 {
                    self.warmup_remaining -= 1;
                    *sample = *old_sample;
                    continue;
                }
                let weight = (self.fade_position as f32 / self.fade_samples as f32).min(1.0);
                *sample = old_sample + weight * (*sample - old_sample);
                self.fade_position = self.fade_position.saturating_add(1);
            }
            if self.fade_position >= self.fade_samples {
                self.retired = self.outgoing.take();
                self.release_retired();
            }
        }
        if self.dictation_mute.load(Ordering::Acquire) {
            io.fill(0.0);
        }
    }

    fn capture(&mut self, io: &[f32]) {
        let count = io.len().min(self.capture.slots());
        if count < io.len() {
            self.capture_flags.overrun.store(true, Ordering::Relaxed);
        }
        if let Ok(chunk) = self.capture.write_chunk_uninit(count) {
            chunk.fill_from_iter(io.iter().copied());
        }
    }

    fn release_retired(&mut self) {
        if let Some(processor) = self.retired.take() {
            if let Err(rtrb::PushError::Full(processor)) = self.garbage.push(processor) {
                // The control thread is behind; keep it and retry on the next block.
                self.retired = Some(processor);
            }
        }
    }

    fn drain(&mut self) {
        self.release_retired();
        if let Some(tap) = self.retired_tap.take() {
            if let Err(rtrb::PushError::Full(tap)) = self.tap_garbage.push(tap) {
                self.retired_tap = Some(tap);
            }
        }
        loop {
            let busy = self.outgoing.is_some() || self.retired.is_some();
            match self.commands.peek() {
                Ok(Command::Replace(_)) if busy => return,
                Ok(Command::Dictation(_)) if self.retired_tap.is_some() => return,
                Ok(_) => {}
                Err(_) => return,
            }
            match self.commands.pop() {
                Ok(Command::Dictation(tap)) => {
                    self.retired_tap = std::mem::replace(&mut self.tap, tap);
                }
                Ok(Command::Retune(settings)) => {
                    // The control side only sends retunes that keep the topology.
                    let _ = self.current.retune(&settings);
                }
                Ok(Command::Replace(processor)) => {
                    self.fade_samples = if matches!((self.current.noise_engine(), processor.noise_engine()), (Some(old), Some(new)) if old != new)
                    {
                        macafon_dsp::SAMPLE_RATE_HZ / 20
                    } else {
                        SWAP_FADE_SAMPLES
                    };
                    // Fill the incoming model's delay before fading; otherwise its startup
                    // silence would mute the old path and the first wet sample could click.
                    self.warmup_remaining = if self.fade_samples > SWAP_FADE_SAMPLES {
                        processor.latency_samples()
                    } else {
                        0
                    };
                    self.outgoing = Some(std::mem::replace(&mut self.current, processor));
                    self.fade_position = 0;
                }
                Err(_) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};
    use macafon_dsp::Preset;

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    fn run(rt: &mut MicRt, blocks: usize, value: f32) -> Vec<f32> {
        let mut output = vec![value; blocks * 128];
        assert_no_alloc(|| output.chunks_mut(128).for_each(|block| rt.process(block)));
        output
    }

    #[test]
    fn studio_uses_embedded_model_and_falls_back_after_three_budget_violations() {
        use macafon_dsp::settings::{Module, NoiseEngine, NoiseSuppressionParams};
        let settings = ChainSettings {
            noise_suppression: Module::on(NoiseSuppressionParams {
                engine: NoiseEngine::Studio,
                intensity_pct: 80.0,
                ..NoiseSuppressionParams::default()
            }),
            ..ChainSettings::raw()
        };
        let (mut control, mut rt) =
            microphone(&settings, Arc::default()).expect("embedded DFN3 initializes");
        assert_eq!(rt.current.noise_engine(), Some(NoiseEngine::Studio));
        assert_eq!(rt.latency_samples(), macafon_dsp::STUDIO_LATENCY_SAMPLES);
        // Drive the adapter on a fresh thread under the allocator guard. Burst processing
        // deliberately misses inference deadlines and must remain bounded and allocation-free.
        let mut rt = std::thread::spawn(move || {
            let mut block = [0.1; 480];
            for _ in 0..10 {
                assert_no_alloc(|| rt.process(&mut block));
            }
            assert!(block.iter().all(|x| x.is_finite()));
            rt
        })
        .join()
        .expect("audio thread succeeds");
        assert!(
            !control
                .observe_processing_budget(800, 1000)
                .expect("under budget")
        );
        assert!(!control.observe_processing_budget(801, 1000).expect("first"));
        assert!(
            !control
                .observe_processing_budget(900, 1000)
                .expect("second")
        );
        assert!(
            control
                .observe_processing_budget(900, 1000)
                .expect("fallback queued")
        );
        assert_eq!(
            control.settings().noise_suppression.params.engine,
            NoiseEngine::Lite
        );
        assert_no_alloc(|| rt.process(&mut [0.1; 128]));
        assert_eq!(rt.fade_samples, macafon_dsp::SAMPLE_RATE_HZ / 20);
        assert_eq!(rt.fade_position, 0);
        assert_eq!(rt.warmup_remaining, macafon_dsp::LITE_LATENCY_SAMPLES - 128);
        assert_no_alloc(|| {
            rt.process(&mut [0.1; macafon_dsp::LITE_LATENCY_SAMPLES as usize - 128])
        });
        assert_no_alloc(|| rt.process(&mut [0.1; 128]));
        assert_eq!(rt.fade_position, 128);
        assert!(
            !control
                .poll_studio_fallback()
                .expect("fallback emitted once")
        );
    }

    #[test]
    fn offline_studio_preserves_frames_and_its_reported_latency_at_zero_intensity() {
        use macafon_dsp::settings::{Module, NoiseEngine, NoiseSuppressionParams};
        let mut settings = ChainSettings {
            noise_suppression: Module::on(NoiseSuppressionParams {
                engine: NoiseEngine::Studio,
                intensity_pct: 0.0,
                ..NoiseSuppressionParams::default()
            }),
            ..ChainSettings::raw()
        };
        let mut processor =
            prepare_offline_processor(&settings, Arc::default()).expect("embedded model");
        let input: Vec<f32> = (0..9600).map(|n| (n as f32 * 0.032).sin() * 0.1).collect();
        let mut output = input.clone();
        for block in output.chunks_mut(128) {
            processor.process(block);
        }
        let latency = processor.latency_samples() as usize;
        assert_eq!(latency, macafon_dsp::STUDIO_LATENCY_SAMPLES as usize);
        assert!(output[..latency].iter().all(|sample| *sample == 0.0));
        assert_eq!(&output[latency..], &input[..input.len() - latency]);
        settings.noise_suppression.params.intensity_pct = 80.0;
        processor
            .retune(&settings)
            .expect("continuous attenuation parameter");
        let mut wet = input.clone();
        processor.process(&mut wet);
        assert!(
            wet[latency..].iter().any(|x| x.abs() > 1e-9),
            "Studio must produce actual audio: {:?}",
            processor.noise_error()
        );
        assert!(
            wet[latency..]
                .iter()
                .zip(&output[latency..])
                .any(|(a, b)| (a - b).abs() > 1e-4),
            "Studio attenuation must use the neural model"
        );
    }

    #[test]
    fn retune_reaches_the_audio_thread_without_allocation() {
        let (mut control, mut rt) =
            microphone(&ChainSettings::raw(), Arc::default()).expect("valid settings");
        let mut louder = ChainSettings::raw();
        louder.gain.params.gain_db = 6.0;
        control.apply(&louder).expect("queued");
        let output = run(&mut rt, 8, 0.1);
        assert!((output[output.len() - 1] - 0.1 * 10f32.powf(0.3)).abs() < 1e-5);
        assert_eq!(control.latency_samples(), 0);
    }

    #[test]
    fn structural_change_crossfades_and_returns_the_old_chain_off_the_rt_thread() {
        let (mut control, mut rt) =
            microphone(&ChainSettings::raw(), Arc::default()).expect("valid settings");
        let mut muted = ChainSettings::raw();
        muted.bypass = true;
        muted.muted = true;
        control.apply(&muted).expect("rebuild queued");
        // A second rebuild waits until the first crossfade has finished.
        control
            .apply(&ChainSettings::raw())
            .expect("second rebuild queued");
        let output = run(&mut rt, 4, 0.5);
        let max_step = output
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.01, "crossfade jumped by {max_step}");
        // First fade (raw → muted) completes after 480 samples, then the second starts.
        assert!(output[..480].iter().all(|y| *y > 0.0));
        assert!(output[470..490].iter().any(|y| *y < 0.02));
        let output = run(&mut rt, 8, 0.5);
        assert!(output[output.len() - 1] == 0.5);
        control.collect_garbage();
        assert!(control.garbage.is_empty());
        assert_eq!(control.settings(), &ChainSettings::raw());
    }

    #[test]
    fn invalid_settings_never_reach_the_audio_thread() {
        let (mut control, _rt) =
            microphone(&Preset::Natural.settings(), Arc::default()).expect("valid settings");
        let mut invalid = Preset::Natural.settings();
        invalid.limiter.params.ceiling_db = 3.0;
        assert!(matches!(
            control.apply(&invalid),
            Err(EngineError::Invalid(_))
        ));
        assert_eq!(control.settings(), &Preset::Natural.settings());
    }

    #[test]
    fn raw_capture_copies_the_input_before_the_chain_only_while_armed() {
        let (mut control, mut rt) =
            microphone(&Preset::Broadcast.settings(), Arc::default()).expect("valid settings");
        let mut samples = Vec::new();
        run(&mut rt, 2, 0.25);
        control.start_capture();
        run(&mut rt, 3, 0.25);
        control.stop_capture();
        run(&mut rt, 2, 0.25);
        assert_eq!(control.read_capture(&mut samples), Ok(()));
        assert_eq!(samples, vec![0.25; 3 * 128]);
        // An undrained capture reports the loss instead of blocking the audio thread.
        control.start_capture();
        run(&mut rt, CAPTURE_SAMPLES / 128 + 2, 0.5);
        samples.clear();
        assert_eq!(control.read_capture(&mut samples), Err(CaptureOverrun));
        assert_eq!(samples.len(), CAPTURE_SAMPLES);
        control.start_capture();
        assert_eq!(control.read_capture(&mut samples), Ok(()));
    }

    #[test]
    fn a_stalled_audio_thread_reports_busy_instead_of_blocking() {
        let (mut control, _rt) =
            microphone(&ChainSettings::raw(), Arc::default()).expect("valid settings");
        let mut settings = ChainSettings::raw();
        let result = (0..=QUEUE_CAPACITY).try_for_each(|step| {
            settings.gain.params.gain_db = step as f32 * 0.1;
            control.apply(&settings)
        });
        assert!(matches!(result, Err(EngineError::Busy)));
    }
}
