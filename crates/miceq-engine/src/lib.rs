//! Lock-free control of the real-time microphone chain (PRD §11.4, NFR-S2).
//!
//! `MicControl` lives on the control thread, `MicRt` inside the audio callback. Parameter
//! changes travel through an SPSC queue and are retuned in place; structural changes build a
//! new processor off the RT thread, which `MicRt` crossfades in. Replaced processors return
//! through a second queue so they are freed outside the RT thread.

use miceq_dsp::{ChainSettings, MicrophoneProcessor, ParameterError, Telemetry};
use std::sync::Arc;

/// Crossfade between an old and a rebuilt chain: 10 ms (DSP-003).
pub const SWAP_FADE_SAMPLES: u32 = miceq_dsp::SAMPLE_RATE_HZ / 100;
const QUEUE_CAPACITY: usize = 64;
const GARBAGE_CAPACITY: usize = 8;
const SCRATCH_SAMPLES: usize = 1024;

// Settings travel inline: popping a slot must not free memory on the RT thread.
#[allow(clippy::large_enum_variant)]
enum Command {
    Retune(ChainSettings),
    Replace(Box<MicrophoneProcessor>),
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Invalid(#[from] ParameterError),
    #[error("the audio thread is not consuming changes; try again")]
    Busy,
}

/// Create a connected control/RT pair for one microphone chain. Pass the same telemetry to
/// successive pairs (e.g. after a PipeWire restart) so meter readers keep working.
pub fn microphone(
    settings: &ChainSettings,
    telemetry: Arc<Telemetry>,
) -> Result<(MicControl, MicRt), ParameterError> {
    let processor = MicrophoneProcessor::with_telemetry(settings, telemetry.clone())?;
    let (commands, command_queue) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
    let (garbage, garbage_queue) = rtrb::RingBuffer::new(GARBAGE_CAPACITY);
    Ok((
        MicControl {
            settings: *settings,
            commands,
            garbage: garbage_queue,
            telemetry,
        },
        MicRt {
            current: Box::new(processor),
            outgoing: None,
            retired: None,
            fade_position: 0,
            scratch: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
            commands: command_queue,
            garbage,
        },
    ))
}

pub struct MicControl {
    settings: ChainSettings,
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Box<MicrophoneProcessor>>,
    telemetry: Arc<Telemetry>,
}

impl MicControl {
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

    /// Send new settings to the audio thread: retuned in place, or rebuilt and crossfaded.
    pub fn apply(&mut self, settings: &ChainSettings) -> Result<(), EngineError> {
        settings.validate()?;
        self.collect_garbage();
        if self.commands.is_full() {
            return Err(EngineError::Busy);
        }
        let command = if self.settings.requires_rebuild(settings) {
            Command::Replace(Box::new(MicrophoneProcessor::with_telemetry(
                settings,
                self.telemetry.clone(),
            )?))
        } else {
            Command::Retune(*settings)
        };
        self.commands.push(command).map_err(|_| EngineError::Busy)?;
        self.settings = *settings;
        Ok(())
    }

    /// Free processors the audio thread has swapped out.
    pub fn collect_garbage(&mut self) {
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
    scratch: Box<[f32]>,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Box<MicrophoneProcessor>>,
}

impl MicRt {
    pub fn latency_samples(&self) -> u32 {
        self.current.latency_samples()
    }

    /// RT-safe: no allocation, locks or deallocation.
    pub fn process(&mut self, io: &mut [f32]) {
        self.drain();
        if self.outgoing.is_none() {
            self.current.process(io);
            return;
        }
        for chunk in io.chunks_mut(SCRATCH_SAMPLES) {
            let Some(old) = &mut self.outgoing else {
                self.current.process(chunk);
                continue;
            };
            let previous = &mut self.scratch[..chunk.len()];
            previous.copy_from_slice(chunk);
            old.process(previous);
            self.current.process(chunk);
            for (sample, old_sample) in chunk.iter_mut().zip(previous.iter()) {
                let weight = (self.fade_position as f32 / SWAP_FADE_SAMPLES as f32).min(1.0);
                *sample = old_sample + weight * (*sample - old_sample);
                self.fade_position = self.fade_position.saturating_add(1);
            }
            if self.fade_position >= SWAP_FADE_SAMPLES {
                self.retired = self.outgoing.take();
                self.release_retired();
            }
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
        loop {
            let busy = self.outgoing.is_some() || self.retired.is_some();
            match self.commands.peek() {
                Ok(Command::Replace(_)) if busy => return,
                Ok(_) => {}
                Err(_) => return,
            }
            match self.commands.pop() {
                Ok(Command::Retune(settings)) => {
                    // The control side only sends retunes that keep the topology.
                    let _ = self.current.retune(&settings);
                }
                Ok(Command::Replace(processor)) => {
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
    use miceq_dsp::Preset;

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    fn run(rt: &mut MicRt, blocks: usize, value: f32) -> Vec<f32> {
        let mut output = vec![value; blocks * 128];
        assert_no_alloc(|| output.chunks_mut(128).for_each(|block| rt.process(block)));
        output
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
