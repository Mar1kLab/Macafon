//! The M0 mono/48 kHz processing path. All storage is allocated at construction.

use nnnoiseless::DenoiseState;

pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const FRAME_SAMPLES: usize = DenoiseState::FRAME_SIZE;
// RNNoise overlap adds one frame; the streaming adapter adds another.
pub const LITE_LATENCY_SAMPLES: u32 = (2 * FRAME_SAMPLES) as u32;

#[derive(Debug, Clone, Copy)]
pub struct ParameterSpec {
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

pub const INTENSITY: ParameterSpec = ParameterSpec {
    min: 0.0,
    max: 100.0,
    default: 70.0,
};
pub const VOICE_THRESHOLD: ParameterSpec = ParameterSpec {
    min: 0.0,
    max: 100.0,
    default: 50.0,
};

#[derive(Debug, thiserror::Error)]
#[error("{name} must be finite and between {min} and {max}")]
pub struct ParameterError {
    name: &'static str,
    min: f32,
    max: f32,
}

fn validate(name: &'static str, value: f32, spec: ParameterSpec) -> Result<f32, ParameterError> {
    if value.is_finite() && (spec.min..=spec.max).contains(&value) {
        Ok(value * 0.01)
    } else {
        Err(ParameterError {
            name,
            min: spec.min,
            max: spec.max,
        })
    }
}

/// Reject invalid audio before it can poison the recurrent model, and flush denormals.
#[inline]
pub fn sanitize(sample: f32) -> f32 {
    if sample.is_finite() && !sample.is_subnormal() {
        sample
    } else {
        0.0
    }
}

pub struct LiteNoiseSuppression {
    state: Box<DenoiseState<'static>>,
    input: [f32; FRAME_SAMPLES],
    previous_dry: [f32; FRAME_SAMPLES],
    output: [f32; FRAME_SAMPLES],
    position: usize,
    intensity: f32,
    voice_threshold: f32,
    voice_gain: f32,
}

impl LiteNoiseSuppression {
    pub fn new(
        intensity_percent: f32,
        voice_threshold_percent: f32,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            intensity: validate("intensity", intensity_percent, INTENSITY)?,
            voice_threshold: validate("voice threshold", voice_threshold_percent, VOICE_THRESHOLD)?,
            state: DenoiseState::new(),
            input: [0.0; FRAME_SAMPLES],
            previous_dry: [0.0; FRAME_SAMPLES],
            output: [0.0; FRAME_SAMPLES],
            position: 0,
            voice_gain: 1.0,
        })
    }

    /// Accept any quantum, including blocks that straddle a 480-sample frame.
    pub fn process(&mut self, io: &mut [f32]) {
        for sample in io {
            self.input[self.position] = sanitize(*sample).clamp(-1.0, 1.0) * 32768.0;
            *sample = self.output[self.position];
            self.position += 1;
            if self.position == FRAME_SAMPLES {
                let voice_probability = self.state.process_frame(&mut self.output, &self.input);
                // M0: linear extra attenuation below the configured VAD threshold.
                let target_voice_gain = if self.voice_threshold > 0.0 {
                    (voice_probability / self.voice_threshold).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let voice_gain_step = (target_voice_gain - self.voice_gain) / FRAME_SAMPLES as f32;
                for ((output, dry), input) in self
                    .output
                    .iter_mut()
                    .zip(&mut self.previous_dry)
                    .zip(&self.input)
                {
                    self.voice_gain += voice_gain_step;
                    *output = sanitize(
                        (*dry * (1.0 - self.intensity)
                            + *output * self.intensity * self.voice_gain)
                            / 32768.0,
                    );
                    *dry = *input;
                }
                self.voice_gain = target_voice_gain;
                self.position = 0;
            }
        }
    }
}

/// M0 chain, deliberately separate from the eventual eight-module M1 chain.
pub struct MicrophoneProcessor {
    lite: Option<LiteNoiseSuppression>,
    muted: bool,
}

impl MicrophoneProcessor {
    pub fn raw(muted: bool) -> Self {
        Self { lite: None, muted }
    }

    pub fn lite(
        intensity_percent: f32,
        voice_threshold_percent: f32,
        muted: bool,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            lite: Some(LiteNoiseSuppression::new(
                intensity_percent,
                voice_threshold_percent,
            )?),
            muted,
        })
    }

    pub fn latency_samples(&self) -> u32 {
        if self.muted || self.lite.is_none() {
            0
        } else {
            LITE_LATENCY_SAMPLES
        }
    }

    pub fn process(&mut self, io: &mut [f32]) {
        if self.muted {
            io.fill(0.0);
            return;
        }
        if let Some(lite) = &mut self.lite {
            lite.process(io);
        }
        for sample in io {
            *sample = sanitize(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    #[test]
    fn raw_is_bit_exact_and_mute_is_digital_silence_without_allocation() {
        let input = [-0.5, 0.0, -0.0, 0.123, 1.25];
        let mut output = input;
        let mut raw = MicrophoneProcessor::raw(false);
        assert_no_alloc(|| raw.process(&mut output));
        assert_eq!(input.map(f32::to_bits), output.map(f32::to_bits));
        let mut muted = MicrophoneProcessor::raw(true);
        assert_no_alloc(|| muted.process(&mut output));
        assert_eq!(output, [0.0; 5]);
        let mut invalid = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(1),
        ];
        assert_no_alloc(|| raw.process(&mut invalid));
        assert_eq!(invalid, [0.0; 4]);
    }

    #[test]
    fn lite_handles_variable_quantum_and_aligns_dry_without_allocation() {
        let input: Vec<_> = (0..4096).map(|i| (i as f32 * 0.07).sin() * 0.3).collect();
        let mut output = input.clone();
        let mut processor =
            MicrophoneProcessor::lite(0.0, 50.0, false).expect("valid test parameters");
        assert_no_alloc(|| {
            for block in output.chunks_mut(128) {
                processor.process(block);
            }
        });
        let latency = processor.latency_samples() as usize;
        assert!(output[..latency].iter().all(|sample| *sample == 0.0));
        assert_eq!(&output[latency..], &input[..input.len() - latency]);

        let mut a = MicrophoneProcessor::lite(70.0, 50.0, false).expect("valid test parameters");
        let mut b = MicrophoneProcessor::lite(70.0, 50.0, false).expect("valid test parameters");
        let mut whole = input.clone();
        let mut split = input;
        assert_no_alloc(|| {
            a.process(&mut whole);
            for block in split.chunks_mut(73) {
                b.process(block);
            }
        });
        assert_eq!(whole, split);
        assert!(
            whole
                .iter()
                .all(|sample| sample.is_finite() && !sample.is_subnormal())
        );
        // Construct on the control thread, then run the first frame on a fresh RT thread.
        let mut fresh =
            MicrophoneProcessor::lite(70.0, 50.0, false).expect("valid test parameters");
        std::thread::spawn(move || {
            let mut block = [0.25; 1024];
            block[0] = f32::NAN;
            block[1] = f32::INFINITY;
            assert_no_alloc(|| fresh.process(&mut block));
            assert!(block.iter().all(|sample| sample.is_finite()));
        })
        .join()
        .expect("RT test thread succeeds");
    }

    #[test]
    fn invalid_parameters_are_rejected_before_processing() {
        for value in [f32::NAN, f32::INFINITY, -1.0, 101.0] {
            assert!(MicrophoneProcessor::lite(value, 50.0, false).is_err());
            assert!(MicrophoneProcessor::lite(70.0, value, false).is_err());
        }
    }
}
