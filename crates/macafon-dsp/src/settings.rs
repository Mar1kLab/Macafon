//! Plain-data settings of the microphone chain, the macro mappings (DSP-002) and validation.
//!
//! Every type is `Copy` so a complete chain description can cross a lock-free queue.

use crate::params::*;

mod dynamics;
mod eq;
mod input;

pub use dynamics::{
    CompressorDetector, CompressorParams, DeEsserMode, DeEsserParams, ExpanderParams, LimiterParams,
};
pub use eq::{EQ_DEFAULT_LAYOUT, EqBand, EqCharacter, EqFilter, EqParams};
pub use input::{GainParams, HighPassParams, InputChannel, NoiseEngine, NoiseSuppressionParams};

/// Noise floor assumed until calibration measures one; places Sensitivity 50% at −50 dBFS.
pub const DEFAULT_NOISE_FLOOR_DBFS: f32 = -56.0;
/// Speech level targeted by calibration (CAL-003), used as the auto-makeup reference.
pub const SPEECH_REFERENCE_DBFS: f32 = -18.0;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Module<T> {
    pub enabled: bool,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub params: T,
}

impl<T> Module<T> {
    pub fn on(params: T) -> Self {
        Self {
            enabled: true,
            params,
        }
    }

    pub fn off(params: T) -> Self {
        Self {
            enabled: false,
            params,
        }
    }
}

/// Macro curves (DSP-002): `(slider 0…1, value)` points, linearly interpolated. Exported to
/// the UI, which inverts them to place the main slider.
pub mod macros {
    /// Expander threshold above the noise floor: floor + 30 dB (0%), + 6 dB (50%), floor (100%).
    pub const EXPANDER_THRESHOLD_OFFSET_DB: [(f32, f32); 3] = [(0.0, 30.0), (0.5, 6.0), (1.0, 0.0)];
    pub const EXPANDER_RATIO: [(f32, f32); 3] = [(0.0, 1.5), (0.5, 3.0), (1.0, 6.0)];
    pub const EXPANDER_RANGE_DB: [(f32, f32); 3] = [(0.0, -15.0), (0.5, -30.0), (1.0, -50.0)];
    pub const COMPRESSOR_THRESHOLD_DBFS: [(f32, f32); 2] = [(0.0, -10.0), (1.0, -35.0)];
    /// 3:1 at the 40% default.
    pub const COMPRESSOR_RATIO: [(f32, f32); 3] = [(0.0, 2.0), (0.4, 3.0), (1.0, 6.0)];
    pub const DE_ESSER_THRESHOLD_DBFS: [(f32, f32); 2] = [(0.0, -10.0), (1.0, -50.0)];
}

/// The fixed-order chain of PRD §6.3: Gain → HPF → NS → Expander → EQ → Compressor →
/// De-esser → Limiter → Output gain.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ChainSettings {
    pub muted: bool,
    /// Global bypass (MIC-014): the chain passes the input at 0 dB.
    pub bypass: bool,
    pub gain: Module<GainParams>,
    pub high_pass: Module<HighPassParams>,
    pub noise_suppression: Module<NoiseSuppressionParams>,
    pub expander: Module<ExpanderParams>,
    pub eq: Module<EqParams>,
    pub compressor: Module<CompressorParams>,
    pub de_esser: Module<DeEsserParams>,
    pub limiter: Module<LimiterParams>,
    pub output_gain_db: f32,
}

impl Default for ChainSettings {
    fn default() -> Self {
        crate::Preset::Natural.settings()
    }
}

impl ChainSettings {
    /// Everything off except Gain at 0 dB: a bit-exact pass-through.
    pub fn raw() -> Self {
        Self {
            muted: false,
            bypass: false,
            gain: Module::on(GainParams::default()),
            high_pass: Module::off(HighPassParams::default()),
            noise_suppression: Module::off(NoiseSuppressionParams::default()),
            expander: Module::off(ExpanderParams::default()),
            eq: Module::off(EqParams::default()),
            compressor: Module::off(CompressorParams::default()),
            de_esser: Module::off(DeEsserParams::default()),
            limiter: Module::off(LimiterParams::default()),
            output_gain_db: OUTPUT_GAIN_DB.default,
        }
    }

    /// Module on/off, bypass, the NS engine and look-ahead change topology or latency and
    /// need a new processor; everything else can be retuned in place.
    pub fn requires_rebuild(&self, new: &Self) -> bool {
        self.bypass != new.bypass
            || self.gain.enabled != new.gain.enabled
            || self.high_pass.enabled != new.high_pass.enabled
            || self.noise_suppression.enabled != new.noise_suppression.enabled
            || self.noise_suppression.params.engine != new.noise_suppression.params.engine
            || self.expander.enabled != new.expander.enabled
            || self.eq.enabled != new.eq.enabled
            || self.compressor.enabled != new.compressor.enabled
            || self.de_esser.enabled != new.de_esser.enabled
            || self.limiter.enabled != new.limiter.enabled
            || self.limiter.params.lookahead_ms != new.limiter.params.lookahead_ms
    }

    /// Algorithmic latency of a processor built from these settings.
    pub fn latency_samples(&self) -> u32 {
        if self.bypass {
            return 0;
        }
        let noise = if self.noise_suppression.enabled {
            match self.noise_suppression.params.engine {
                NoiseEngine::Lite => crate::LITE_LATENCY_SAMPLES,
                NoiseEngine::Studio => crate::STUDIO_LATENCY_SAMPLES,
            }
        } else {
            0
        };
        let limiter = if self.limiter.enabled {
            u32::from(self.limiter.params.lookahead_ms) * crate::SAMPLE_RATE_HZ / 1000
        } else {
            0
        };
        noise + limiter
    }

    pub fn validate(&self) -> Result<(), ParameterError> {
        GAIN_DB.check(self.gain.params.gain_db)?;
        self.high_pass.params.validate()?;
        self.noise_suppression.params.validate()?;
        self.expander.params.validate()?;
        self.eq.params.validate()?;
        self.compressor.params.validate()?;
        self.de_esser.params.validate()?;
        self.limiter.params.validate()?;
        OUTPUT_GAIN_DB.check(self.output_gain_db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macros_land_on_prd_defaults_and_ranges() {
        let expander = ExpanderParams::from_sensitivity(50.0, DEFAULT_NOISE_FLOOR_DBFS);
        assert_eq!(expander.threshold_db, EXPANDER_THRESHOLD_DBFS.default);
        assert_eq!(expander.ratio, EXPANDER_RATIO.default);
        assert_eq!(expander.range_db, EXPANDER_RANGE_DB.default);
        let compressor = CompressorParams::from_amount(COMPRESSOR_AMOUNT.default);
        assert!((compressor.threshold_db - COMPRESSOR_THRESHOLD_DBFS.default).abs() < 1e-4);
        assert!((compressor.ratio - COMPRESSOR_RATIO.default).abs() < 1e-4);
        assert_eq!(CompressorParams::from_amount(0.0).threshold_db, -10.0);
        assert_eq!(CompressorParams::from_amount(100.0).ratio, 6.0);
        for percent in [0.0, 13.0, 50.0, 87.0, 100.0] {
            let mut settings = ChainSettings::default();
            settings.expander.params = ExpanderParams::from_sensitivity(percent, -90.0);
            settings.compressor.params = CompressorParams::from_amount(percent);
            settings.de_esser.params = DeEsserParams::from_intensity(percent);
            for character in EqCharacter::ALL {
                settings.eq.params = EqParams::from_character(character, percent);
                assert_eq!(settings.validate(), Ok(()), "{percent}% {character:?}");
            }
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn settings_round_trip_through_json_and_toml() {
        for preset in crate::Preset::ALL {
            let mut settings = preset.settings();
            settings.expander.params.ratio = f32::INFINITY;
            settings.compressor.params.makeup_db = Some(3.5);
            let json = serde_json::to_string(&settings).expect("serialize JSON");
            assert!(json.contains(r#""ratio":"gate""#), "{json}");
            let back: ChainSettings = serde_json::from_str(&json).expect("parse JSON");
            assert_eq!(back, settings);
            let text = toml::to_string(&settings).expect("serialize TOML");
            assert_eq!(
                toml::from_str::<ChainSettings>(&text),
                Ok(settings),
                "{text}"
            );
        }
        // Missing fields fall back to defaults; unknown values are rejected.
        let partial: ChainSettings =
            toml::from_str("muted = true\n[compressor]\nenabled = false\n").expect("partial");
        assert!(partial.muted && !partial.compressor.enabled);
        assert_eq!(partial.limiter, ChainSettings::default().limiter);
        assert!(serde_json::from_str::<ExpanderParams>(r#"{"ratio":"soft"}"#).is_err());
    }
}
