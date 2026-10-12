//! Dynamics parameters and validation.

use super::macros;
use crate::params::*;
use crate::util::interpolate;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ExpanderParams {
    pub threshold_db: f32,
    /// Expansion ratio 1:x; `f32::INFINITY` turns the expander into a gate.
    #[cfg_attr(feature = "serde", serde(with = "gate_ratio"))]
    #[cfg_attr(feature = "ts", ts(type = r#"number | "gate""#))]
    pub ratio: f32,
    pub range_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    pub hysteresis_db: f32,
}

impl Default for ExpanderParams {
    fn default() -> Self {
        Self {
            threshold_db: EXPANDER_THRESHOLD_DBFS.default,
            ratio: EXPANDER_RATIO.default,
            range_db: EXPANDER_RANGE_DB.default,
            attack_ms: EXPANDER_ATTACK_MS.default,
            hold_ms: EXPANDER_HOLD_MS.default,
            release_ms: EXPANDER_RELEASE_MS.default,
            hysteresis_db: EXPANDER_HYSTERESIS_DB.default,
        }
    }
}

impl ExpanderParams {
    /// Sensitivity macro: the threshold descends towards the noise floor while ratio and
    /// range grow with the slider.
    pub fn from_sensitivity(sensitivity_pct: f32, noise_floor_dbfs: f32) -> Self {
        let s = sensitivity_pct * 0.01;
        Self {
            threshold_db: (noise_floor_dbfs
                + interpolate(s, &macros::EXPANDER_THRESHOLD_OFFSET_DB))
            .clamp(EXPANDER_THRESHOLD_DBFS.min, EXPANDER_THRESHOLD_DBFS.max),
            ratio: interpolate(s, &macros::EXPANDER_RATIO),
            range_db: interpolate(s, &macros::EXPANDER_RANGE_DB),
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        EXPANDER_THRESHOLD_DBFS.check(self.threshold_db)?;
        if self.ratio != f32::INFINITY {
            EXPANDER_RATIO.check(self.ratio)?;
        }
        EXPANDER_RANGE_DB.check(self.range_db)?;
        EXPANDER_ATTACK_MS.check(self.attack_ms)?;
        EXPANDER_HOLD_MS.check(self.hold_ms)?;
        EXPANDER_RELEASE_MS.check(self.release_ms)?;
        EXPANDER_HYSTERESIS_DB.check(self.hysteresis_db)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum CompressorDetector {
    Peak,
    Rms,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct CompressorParams {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    /// Program-dependent release: fast for transients, slow for sustained reduction.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub auto_release: bool,
    pub knee_db: f32,
    /// `None` = auto: restores the static gain lost at the speech reference level.
    pub makeup_db: Option<f32>,
    pub detector: CompressorDetector,
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold_db: COMPRESSOR_THRESHOLD_DBFS.default,
            ratio: COMPRESSOR_RATIO.default,
            attack_ms: COMPRESSOR_ATTACK_MS.default,
            release_ms: COMPRESSOR_RELEASE_MS.default,
            auto_release: false,
            knee_db: COMPRESSOR_KNEE_DB.default,
            makeup_db: None,
            detector: CompressorDetector::Rms,
        }
    }
}

impl CompressorParams {
    /// Amount macro: threshold −10 → −35 dBFS; ratio 2:1 → 6:1 through 3:1 at the 40% default.
    pub fn from_amount(amount_pct: f32) -> Self {
        let a = amount_pct * 0.01;
        Self {
            threshold_db: interpolate(a, &macros::COMPRESSOR_THRESHOLD_DBFS),
            ratio: interpolate(a, &macros::COMPRESSOR_RATIO),
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        COMPRESSOR_THRESHOLD_DBFS.check(self.threshold_db)?;
        COMPRESSOR_RATIO.check(self.ratio)?;
        COMPRESSOR_ATTACK_MS.check(self.attack_ms)?;
        COMPRESSOR_RELEASE_MS.check(self.release_ms)?;
        COMPRESSOR_KNEE_DB.check(self.knee_db)?;
        if let Some(gain_db) = self.makeup_db {
            COMPRESSOR_MAKEUP_DB.check(gain_db)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum DeEsserMode {
    SplitBand,
    Wideband,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct DeEsserParams {
    pub freq_hz: f32,
    pub threshold_db: f32,
    pub max_reduction_db: f32,
    pub mode: DeEsserMode,
    /// Output only the sidechain band to find the sibilance frequency.
    pub listen: bool,
}

impl Default for DeEsserParams {
    fn default() -> Self {
        Self::from_intensity(DE_ESSER_INTENSITY.default)
    }
}

impl DeEsserParams {
    /// Intensity macro: threshold −10 dBFS (0%) → −50 dBFS (100%).
    pub fn from_intensity(intensity_pct: f32) -> Self {
        Self {
            freq_hz: DE_ESSER_FREQ_HZ.default,
            threshold_db: interpolate(intensity_pct * 0.01, &macros::DE_ESSER_THRESHOLD_DBFS),
            max_reduction_db: DE_ESSER_MAX_REDUCTION_DB.default,
            mode: DeEsserMode::SplitBand,
            listen: false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        DE_ESSER_FREQ_HZ.check(self.freq_hz)?;
        DE_ESSER_THRESHOLD_DBFS.check(self.threshold_db)?;
        DE_ESSER_MAX_REDUCTION_DB.check(self.max_reduction_db)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LimiterParams {
    pub ceiling_db: f32,
    pub release_ms: f32,
    pub lookahead_ms: u8,
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self {
            ceiling_db: LIMITER_CEILING_DBFS.default,
            release_ms: LIMITER_RELEASE_MS.default,
            lookahead_ms: 0,
        }
    }
}

impl LimiterParams {
    pub fn ceiling(ceiling_db: f32) -> Self {
        Self {
            ceiling_db,
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        LIMITER_CEILING_DBFS.check(self.ceiling_db)?;
        LIMITER_RELEASE_MS.check(self.release_ms)?;
        if LIMITER_LOOKAHEAD_MS.contains(&self.lookahead_ms) {
            Ok(())
        } else {
            Err(ParameterError {
                name: "limiter look-ahead",
                min: 0.0,
                max: 5.0,
            })
        }
    }
}

/// JSON has no infinity: a gate ratio is written as the string `"gate"`.
#[cfg(feature = "serde")]
mod gate_ratio {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(ratio: &f32, serializer: S) -> Result<S::Ok, S::Error> {
        if *ratio == f32::INFINITY {
            serializer.serialize_str("gate")
        } else {
            serializer.serialize_f32(*ratio)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Ratio {
            Number(f32),
            Name(String),
        }
        match Ratio::deserialize(deserializer)? {
            Ratio::Number(ratio) => Ok(ratio),
            Ratio::Name(name) if name == "gate" => Ok(f32::INFINITY),
            Ratio::Name(name) => Err(serde::de::Error::custom(format!(
                "expander ratio must be a number or \"gate\", got {name:?}"
            ))),
        }
    }
}

#[cfg(feature = "serde")]
fn is_false(value: &bool) -> bool {
    !*value
}
