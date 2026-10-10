//! Parameter ranges and defaults of the microphone chain (PRD §6.3).
//!
//! This is the single source for UI, config and CLI validation. Percent macros use 0–100.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Percent,
    Decibel,
    Dbfs,
    Hertz,
    Milliseconds,
    Ratio,
    Factor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterSpec {
    pub name: &'static str,
    pub unit: Unit,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// UI step; 0 means continuous.
    pub step: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{name} must be finite and between {min} and {max}")]
pub struct ParameterError {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
}

impl ParameterSpec {
    pub fn check(&self, value: f32) -> Result<f32, ParameterError> {
        if value.is_finite() && (self.min..=self.max).contains(&value) {
            Ok(value)
        } else {
            Err(self.error())
        }
    }

    pub fn error(&self) -> ParameterError {
        ParameterError {
            name: self.name,
            min: self.min,
            max: self.max,
        }
    }
}

const fn spec(name: &'static str, unit: Unit, min: f32, max: f32, default: f32) -> ParameterSpec {
    ParameterSpec {
        name,
        unit,
        min,
        max,
        default,
        step: 0.0,
    }
}

pub const GAIN_DB: ParameterSpec = ParameterSpec {
    step: 0.5,
    ..spec("gain", Unit::Decibel, -24.0, 24.0, 0.0)
};

pub const HIGH_PASS_FREQ_HZ: ParameterSpec =
    spec("high-pass frequency", Unit::Hertz, 20.0, 300.0, 80.0);
/// Allowed slopes in dB/oct; the default is the second one.
pub const HIGH_PASS_SLOPES_DB_OCT: [u8; 2] = [12, 24];
pub const HIGH_PASS_DEFAULT_SLOPE_DB_OCT: u8 = 24;

pub const INTENSITY: ParameterSpec = spec(
    "noise suppression intensity",
    Unit::Percent,
    0.0,
    100.0,
    70.0,
);
pub const VOICE_THRESHOLD: ParameterSpec =
    spec("voice activity threshold", Unit::Percent, 0.0, 100.0, 50.0);

pub const EXPANDER_SENSITIVITY: ParameterSpec =
    spec("expander sensitivity", Unit::Percent, 0.0, 100.0, 50.0);
pub const EXPANDER_THRESHOLD_DBFS: ParameterSpec =
    spec("expander threshold", Unit::Dbfs, -80.0, -10.0, -50.0);
/// Finite expansion ratios (1:x). `f32::INFINITY` is additionally accepted as a gate.
pub const EXPANDER_RATIO: ParameterSpec = spec("expander ratio", Unit::Ratio, 1.5, 10.0, 3.0);
pub const EXPANDER_RANGE_DB: ParameterSpec =
    spec("expander range", Unit::Decibel, -80.0, 0.0, -30.0);
pub const EXPANDER_ATTACK_MS: ParameterSpec =
    spec("expander attack", Unit::Milliseconds, 0.1, 50.0, 2.0);
pub const EXPANDER_HOLD_MS: ParameterSpec =
    spec("expander hold", Unit::Milliseconds, 0.0, 500.0, 80.0);
pub const EXPANDER_RELEASE_MS: ParameterSpec =
    spec("expander release", Unit::Milliseconds, 10.0, 1000.0, 150.0);
pub const EXPANDER_HYSTERESIS_DB: ParameterSpec =
    spec("expander hysteresis", Unit::Decibel, 0.0, 12.0, 4.0);

pub const EQ_MAX_BANDS: usize = 10;
pub const EQ_DEFAULT_BANDS: usize = 5;
pub const EQ_AMOUNT: ParameterSpec = spec("EQ amount", Unit::Percent, 0.0, 100.0, 100.0);
pub const EQ_FREQ_HZ: ParameterSpec = spec("EQ frequency", Unit::Hertz, 20.0, 20_000.0, 1000.0);
pub const EQ_GAIN_DB: ParameterSpec = spec("EQ gain", Unit::Decibel, -18.0, 18.0, 0.0);
pub const EQ_Q: ParameterSpec = spec("EQ Q", Unit::Factor, 0.1, 10.0, 1.0);
pub const EQ_OUTPUT_GAIN_DB: ParameterSpec =
    spec("EQ output gain", Unit::Decibel, -12.0, 12.0, 0.0);

pub const COMPRESSOR_AMOUNT: ParameterSpec =
    spec("compressor amount", Unit::Percent, 0.0, 100.0, 40.0);
pub const COMPRESSOR_THRESHOLD_DBFS: ParameterSpec =
    spec("compressor threshold", Unit::Dbfs, -60.0, 0.0, -20.0);
pub const COMPRESSOR_RATIO: ParameterSpec = spec("compressor ratio", Unit::Ratio, 1.0, 20.0, 3.0);
pub const COMPRESSOR_ATTACK_MS: ParameterSpec =
    spec("compressor attack", Unit::Milliseconds, 0.1, 100.0, 5.0);
pub const COMPRESSOR_RELEASE_MS: ParameterSpec =
    spec("compressor release", Unit::Milliseconds, 10.0, 1000.0, 80.0);
pub const COMPRESSOR_KNEE_DB: ParameterSpec =
    spec("compressor knee", Unit::Decibel, 0.0, 12.0, 6.0);
/// Manual makeup range; the default makeup mode is automatic.
pub const COMPRESSOR_MAKEUP_DB: ParameterSpec =
    spec("compressor makeup", Unit::Decibel, -12.0, 24.0, 0.0);

pub const DE_ESSER_INTENSITY: ParameterSpec =
    spec("de-esser intensity", Unit::Percent, 0.0, 100.0, 40.0);
pub const DE_ESSER_FREQ_HZ: ParameterSpec =
    spec("de-esser frequency", Unit::Hertz, 3000.0, 12_000.0, 6500.0);
/// The default threshold is derived from intensity (see `DeEsserParams::from_intensity`).
pub const DE_ESSER_THRESHOLD_DBFS: ParameterSpec =
    spec("de-esser threshold", Unit::Dbfs, -60.0, 0.0, -26.0);
pub const DE_ESSER_MAX_REDUCTION_DB: ParameterSpec =
    spec("de-esser max reduction", Unit::Decibel, 0.0, 18.0, 8.0);

pub const LIMITER_CEILING_DBFS: ParameterSpec =
    spec("limiter ceiling", Unit::Dbfs, -12.0, 0.0, -1.0);
pub const LIMITER_RELEASE_MS: ParameterSpec =
    spec("limiter release", Unit::Milliseconds, 1.0, 500.0, 50.0);
pub const LIMITER_LOOKAHEAD_MS: [u8; 4] = [0, 1, 2, 5];

pub const OUTPUT_GAIN_DB: ParameterSpec = spec("output gain", Unit::Decibel, -24.0, 12.0, 0.0);
