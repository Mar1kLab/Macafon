//! Noise suppression Lite: RNNoise via `nnnoiseless` behind a 480-sample frame adapter
//! (PRD §6.3.3, NS-001). Storage is allocated at construction.

use crate::sanitize;
use crate::settings::NoiseSuppressionParams;
use nnnoiseless::DenoiseState;

pub const FRAME_SAMPLES: usize = DenoiseState::FRAME_SIZE;
// RNNoise overlap adds one frame; the streaming adapter adds another.
pub const LITE_LATENCY_SAMPLES: u32 = (2 * FRAME_SAMPLES) as u32;

pub struct LiteNoiseSuppression {
    state: Box<DenoiseState<'static>>,
    input: [f32; FRAME_SAMPLES],
    previous_dry: [f32; FRAME_SAMPLES],
    output: [f32; FRAME_SAMPLES],
    position: usize,
    intensity: f32,
    target_intensity: f32,
    voice_threshold: f32,
    voice_gain: f32,
}

impl LiteNoiseSuppression {
    /// Parameters must be validated (`ChainSettings::validate`).
    pub fn new(params: &NoiseSuppressionParams) -> Self {
        let intensity = params.intensity_pct * 0.01;
        Self {
            state: DenoiseState::new(),
            input: [0.0; FRAME_SAMPLES],
            previous_dry: [0.0; FRAME_SAMPLES],
            output: [0.0; FRAME_SAMPLES],
            position: 0,
            intensity,
            target_intensity: intensity,
            voice_threshold: params.voice_threshold_pct * 0.01,
            voice_gain: 1.0,
        }
    }

    /// Intensity is ramped across the next 10 ms frame.
    pub fn set(&mut self, params: &NoiseSuppressionParams) {
        self.target_intensity = params.intensity_pct * 0.01;
        self.voice_threshold = params.voice_threshold_pct * 0.01;
    }

    pub fn latency_samples(&self) -> u32 {
        LITE_LATENCY_SAMPLES
    }

    /// Extra VAD attenuation currently applied to the wet signal, 0 (none) … 1 (silenced).
    pub fn activity(&self) -> f32 {
        1.0 - self.voice_gain
    }

    /// Accept any quantum, including blocks that straddle a 480-sample frame.
    pub fn process(&mut self, io: &mut [f32]) {
        for sample in io {
            self.input[self.position] = sanitize(*sample).clamp(-1.0, 1.0) * 32768.0;
            *sample = self.output[self.position];
            self.position += 1;
            if self.position == FRAME_SAMPLES {
                self.process_frame();
                self.position = 0;
            }
        }
    }

    fn process_frame(&mut self) {
        let voice_probability = self.state.process_frame(&mut self.output, &self.input);
        // Linear extra attenuation below the configured VAD threshold.
        let target_voice_gain = if self.voice_threshold > 0.0 {
            (voice_probability / self.voice_threshold).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let voice_gain_step = (target_voice_gain - self.voice_gain) / FRAME_SAMPLES as f32;
        let intensity_step = (self.target_intensity - self.intensity) / FRAME_SAMPLES as f32;
        for ((output, dry), input) in self
            .output
            .iter_mut()
            .zip(&mut self.previous_dry)
            .zip(&self.input)
        {
            self.voice_gain += voice_gain_step;
            self.intensity += intensity_step;
            *output = sanitize(
                (*dry * (1.0 - self.intensity) + *output * self.intensity * self.voice_gain)
                    / 32768.0,
            );
            *dry = *input;
        }
        self.voice_gain = target_voice_gain;
        self.intensity = self.target_intensity;
    }
}

/// Engine-specific storage is built off the audio thread.
/// Implemented by an inference worker adapter prepared outside the DSP/audio thread.
pub trait StudioProcessor: Send {
    fn set(&mut self, params: &NoiseSuppressionParams);
    fn process(&mut self, io: &mut [f32]);
    fn activity(&self) -> f32;
    fn overloaded(&self) -> bool;
    /// Control-side diagnostic; runtime inference failures are published off the audio thread.
    fn failure_reason(&self) -> Option<&str> {
        None
    }
}

/// Embedded DFN3 model delay (30 ms) plus bounded worker adapter delay (20 ms).
pub const STUDIO_LATENCY_SAMPLES: u32 = 2400;

pub enum NoiseSuppression {
    Lite(Box<LiteNoiseSuppression>),
    Studio(Box<dyn StudioProcessor>),
}
impl NoiseSuppression {
    pub fn new(
        params: &NoiseSuppressionParams,
        studio: Option<Box<dyn StudioProcessor>>,
    ) -> Result<Self, crate::ProcessorBuildError> {
        match params.engine {
            crate::settings::NoiseEngine::Lite => {
                Ok(Self::Lite(Box::new(LiteNoiseSuppression::new(params))))
            }
            crate::settings::NoiseEngine::Studio => studio
                .map(Self::Studio)
                .ok_or(crate::ProcessorBuildError::StudioUnavailable),
        }
    }
    pub fn engine(&self) -> crate::settings::NoiseEngine {
        match self {
            Self::Lite(_) => crate::settings::NoiseEngine::Lite,
            Self::Studio(_) => crate::settings::NoiseEngine::Studio,
        }
    }
    pub fn latency_samples(&self) -> u32 {
        match self {
            Self::Lite(module) => module.latency_samples(),
            Self::Studio(_) => crate::STUDIO_LATENCY_SAMPLES,
        }
    }
    pub fn set(&mut self, params: &NoiseSuppressionParams) {
        match self {
            Self::Lite(module) => module.set(params),
            Self::Studio(module) => module.set(params),
        }
    }
    pub fn process(&mut self, io: &mut [f32]) {
        match self {
            Self::Lite(module) => module.process(io),
            Self::Studio(module) => module.process(io),
        }
    }
    pub fn activity(&self) -> f32 {
        match self {
            Self::Lite(module) => module.activity(),
            Self::Studio(module) => module.activity(),
        }
    }
    pub fn failure_reason(&self) -> Option<&str> {
        match self {
            Self::Lite(_) => None,
            Self::Studio(module) => module.failure_reason(),
        }
    }
    pub fn overloaded(&self) -> bool {
        matches!(self, Self::Studio(module) if module.overloaded())
    }
}
