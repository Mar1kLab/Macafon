//! Plain-data settings of the microphone chain, the macro mappings (DSP-002) and validation.
//!
//! Every type is `Copy` so a complete chain description can cross a lock-free queue.

use crate::params::*;
use crate::util::interpolate;

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

    fn validate(&self) -> Result<(), ParameterError> {
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
    /// DeepFilterNet 3; not implemented yet, the chain falls back to Lite.
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

    fn validate(&self) -> Result<(), ParameterError> {
        INTENSITY.check(self.intensity_pct)?;
        VOICE_THRESHOLD.check(self.voice_threshold_pct)?;
        Ok(())
    }
}

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

    fn validate(&self) -> Result<(), ParameterError> {
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

    fn validate(&self) -> Result<(), ParameterError> {
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

#[derive(Debug, Clone, Copy, PartialEq)]
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

    fn validate(&self) -> Result<(), ParameterError> {
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

    fn validate(&self) -> Result<(), ParameterError> {
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

    fn validate(&self) -> Result<(), ParameterError> {
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

    fn validate(&self) -> Result<(), ParameterError> {
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
            crate::LITE_LATENCY_SAMPLES
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
