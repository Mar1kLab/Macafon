//! Input parameters and validation.

use crate::params::*;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GainParams {
    pub gain_db: f32,
}

impl Default for GainParams {
    fn default() -> Self {
        Self {
            gain_db: GAIN_DB.default,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct HighPassParams {
    pub freq_hz: f32,
    pub slope_db_oct: u8,
}

impl Default for HighPassParams {
    fn default() -> Self {
        Self {
            freq_hz: HIGH_PASS_FREQ_HZ.default,
            slope_db_oct: HIGH_PASS_DEFAULT_SLOPE_DB_OCT,
        }
    }
}

impl HighPassParams {
    pub fn at(freq_hz: f32) -> Self {
        Self {
            freq_hz,
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        HIGH_PASS_FREQ_HZ.check(self.freq_hz)?;
        if HIGH_PASS_SLOPES_DB_OCT.contains(&self.slope_db_oct) {
            Ok(())
        } else {
            Err(ParameterError {
                name: "high-pass slope",
                min: 12.0,
                max: 24.0,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum NoiseEngine {
    /// RNNoise (`nnnoiseless`).
    Lite,
    /// Embedded DeepFilterNet 3, prepared by macafon-studio outside the audio callback.
    Studio,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct NoiseSuppressionParams {
    pub engine: NoiseEngine,
    pub intensity_pct: f32,
    pub voice_threshold_pct: f32,
    pub post_filter: bool,
}

impl Default for NoiseSuppressionParams {
    fn default() -> Self {
        Self {
            engine: NoiseEngine::Lite,
            intensity_pct: INTENSITY.default,
            voice_threshold_pct: VOICE_THRESHOLD.default,
            post_filter: true,
        }
    }
}

impl NoiseSuppressionParams {
    pub fn new(engine: NoiseEngine, intensity_pct: f32) -> Self {
        Self {
            engine,
            intensity_pct,
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        INTENSITY.check(self.intensity_pct)?;
        VOICE_THRESHOLD.check(self.voice_threshold_pct)?;
        Ok(())
    }
}

/// Physical capture channel; the backend implements automatic detection and mixing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum InputChannel {
    #[default]
    Auto,
    Left,
    Right,
    Mix,
}
