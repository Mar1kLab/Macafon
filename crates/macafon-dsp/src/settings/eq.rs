//! Eq parameters and validation.

use crate::params::*;

/// EQ band types of PRD §6.3.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EqFilter {
    Bell,
    LowShelf,
    HighShelf,
    LowCut,
    HighCut,
    Notch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EqBand {
    pub filter: EqFilter,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl EqBand {
    pub const fn new(filter: EqFilter, freq_hz: f32, gain_db: f32, q: f32) -> Self {
        Self {
            filter,
            freq_hz,
            gain_db,
            q,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        EQ_FREQ_HZ.check(self.freq_hz)?;
        EQ_GAIN_DB.check(self.gain_db)?;
        EQ_Q.check(self.q)?;
        Ok(())
    }
}

/// Default five bands: low shelf 120 Hz, bells 250 Hz / 1 kHz / 4 kHz, high shelf 10 kHz.
pub const EQ_DEFAULT_LAYOUT: [EqBand; EQ_DEFAULT_BANDS] = [
    EqBand::new(EqFilter::LowShelf, 120.0, 0.0, 0.707),
    EqBand::new(EqFilter::Bell, 250.0, 0.0, 1.0),
    EqBand::new(EqFilter::Bell, 1000.0, 0.0, 1.0),
    EqBand::new(EqFilter::Bell, 4000.0, 0.0, 1.0),
    EqBand::new(EqFilter::HighShelf, 10_000.0, 0.0, 0.707),
];

/// Simple-mode EQ characters (EQ-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EqCharacter {
    Flat,
    Clarity,
    Warmth,
    Broadcast,
    Presence,
    ThinMicFix,
}

impl EqCharacter {
    pub const ALL: [Self; 6] = [
        Self::Flat,
        Self::Clarity,
        Self::Warmth,
        Self::Broadcast,
        Self::Presence,
        Self::ThinMicFix,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Clarity => "Clarity",
            Self::Warmth => "Warmth",
            Self::Broadcast => "Broadcast",
            Self::Presence => "Presence",
            Self::ThinMicFix => "Thin mic fix",
        }
    }

    /// Band gains at Amount 100% over `EQ_DEFAULT_LAYOUT`.
    pub fn gains_db(self) -> [f32; EQ_DEFAULT_BANDS] {
        match self {
            Self::Flat => [0.0; 5],
            Self::Clarity => [-2.0, -3.0, 0.0, 3.0, 2.0],
            Self::Warmth => [3.0, 1.5, 0.0, -1.0, -2.0],
            Self::Broadcast => [2.0, -2.0, 0.0, 2.0, 3.0],
            Self::Presence => [0.0, -1.0, 1.0, 4.0, 1.0],
            Self::ThinMicFix => [4.0, 2.0, 0.0, -1.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(into = "EqRepr", try_from = "EqRepr")
)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(type = "{ bands: Array<EqBand>, output_gain_db: number }")
)]
pub struct EqParams {
    pub bands: [EqBand; EQ_MAX_BANDS],
    pub band_count: usize,
    pub output_gain_db: f32,
}

/// Only the active bands matter; the unused tail is storage.
impl PartialEq for EqParams {
    fn eq(&self, other: &Self) -> bool {
        self.active_bands() == other.active_bands() && self.output_gain_db == other.output_gain_db
    }
}

impl Default for EqParams {
    fn default() -> Self {
        Self::from_character(EqCharacter::Flat, EQ_AMOUNT.default)
    }
}

impl EqParams {
    pub fn from_character(character: EqCharacter, amount_pct: f32) -> Self {
        let mut bands = [EQ_DEFAULT_LAYOUT[0]; EQ_MAX_BANDS];
        for ((band, layout), gain_db) in bands
            .iter_mut()
            .zip(EQ_DEFAULT_LAYOUT)
            .zip(character.gains_db())
        {
            *band = EqBand {
                gain_db: gain_db * amount_pct * 0.01,
                ..layout
            };
        }
        Self {
            bands,
            band_count: EQ_DEFAULT_BANDS,
            output_gain_db: EQ_OUTPUT_GAIN_DB.default,
        }
    }

    pub fn active_bands(&self) -> &[EqBand] {
        &self.bands[..self.band_count.min(EQ_MAX_BANDS)]
    }

    pub(crate) fn validate(&self) -> Result<(), ParameterError> {
        if self.band_count > EQ_MAX_BANDS {
            return Err(ParameterError {
                name: "EQ band count",
                min: 0.0,
                max: EQ_MAX_BANDS as f32,
            });
        }
        self.active_bands().iter().try_for_each(EqBand::validate)?;
        EQ_OUTPUT_GAIN_DB.check(self.output_gain_db)?;
        Ok(())
    }
}

/// Serialized EQ: only the active bands.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct EqRepr {
    bands: Vec<EqBand>,
    output_gain_db: f32,
}

#[cfg(feature = "serde")]
impl Default for EqRepr {
    fn default() -> Self {
        EqParams::default().into()
    }
}

#[cfg(feature = "serde")]
impl From<EqParams> for EqRepr {
    fn from(params: EqParams) -> Self {
        Self {
            bands: params.active_bands().to_vec(),
            output_gain_db: params.output_gain_db,
        }
    }
}

#[cfg(feature = "serde")]
impl TryFrom<EqRepr> for EqParams {
    type Error = String;

    fn try_from(repr: EqRepr) -> Result<Self, String> {
        if repr.bands.len() > EQ_MAX_BANDS {
            return Err(format!("at most {EQ_MAX_BANDS} EQ bands are supported"));
        }
        let mut bands = [EQ_DEFAULT_LAYOUT[0]; EQ_MAX_BANDS];
        bands[..repr.bands.len()].copy_from_slice(&repr.bands);
        Ok(Self {
            bands,
            band_count: repr.bands.len(),
            output_gain_db: repr.output_gain_db,
        })
    }
}
