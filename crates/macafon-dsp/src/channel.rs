//! Output channel chain (PRD §7.3): stereo `EQ → Noise suppression → Compressor → Limiter →
//! Volume`, with the channel templates (CFX-005/006) and EQ presets (CFX-002).
//! Built outside the RT thread; `process` never allocates or locks.

use crate::compressor::Compressor;
use crate::eq::Equalizer;
use crate::limiter::Limiter;
use crate::noise::{LITE_LATENCY_SAMPLES, LiteNoiseSuppression};
use crate::params::*;
use crate::settings::{
    CompressorParams, EqBand, EqFilter, EqParams, LimiterParams, Module, NoiseEngine,
    NoiseSuppressionParams,
};
use crate::util::{Ramp, lin_to_db};
use crate::{SAMPLE_RATE_HZ, sanitize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const CHANNEL_VOLUME_PCT: ParameterSpec = ParameterSpec {
    name: "channel volume",
    unit: Unit::Percent,
    min: 0.0,
    max: 100.0,
    default: 100.0,
    step: 1.0,
};

/// CH-004: the cubic volume curve of PulseAudio/PipeWire; 0% is silence.
pub fn volume_gain(volume_pct: f32) -> f32 {
    let x = (volume_pct * 0.01).clamp(0.0, 1.0);
    x * x * x
}

/// The volume shown in dB next to the percentage (CH-004); `-inf` at 0%.
pub fn volume_db(volume_pct: f32) -> f32 {
    20.0 * volume_gain(volume_pct).log10()
}

/// EQ preset categories of CFX-002.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EqPresetCategory {
    Basic,
    Voice,
    Music,
    Game,
}

/// Built-in channel EQ presets (CFX-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ChannelEqPreset {
    Flat,
    VoiceClarity,
    BassCut,
    WarmVoice,
    BassBoost,
    VShape,
    Vocal,
    Rock,
    Electronic,
    Classical,
    Podcast,
    Footsteps,
    Immersive,
    Competitive,
}

const fn bell(freq_hz: f32, gain_db: f32, q: f32) -> EqBand {
    EqBand::new(EqFilter::Bell, freq_hz, gain_db, q)
}
const fn low_shelf(freq_hz: f32, gain_db: f32) -> EqBand {
    EqBand::new(EqFilter::LowShelf, freq_hz, gain_db, 0.707)
}
const fn high_shelf(freq_hz: f32, gain_db: f32) -> EqBand {
    EqBand::new(EqFilter::HighShelf, freq_hz, gain_db, 0.707)
}
const fn low_cut(freq_hz: f32) -> EqBand {
    EqBand::new(EqFilter::LowCut, freq_hz, 0.0, 0.707)
}

impl ChannelEqPreset {
    pub const ALL: [Self; 14] = [
        Self::Flat,
        Self::VoiceClarity,
        Self::BassCut,
        Self::WarmVoice,
        Self::BassBoost,
        Self::VShape,
        Self::Vocal,
        Self::Rock,
        Self::Electronic,
        Self::Classical,
        Self::Podcast,
        Self::Footsteps,
        Self::Immersive,
        Self::Competitive,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::VoiceClarity => "Voice Clarity",
            Self::BassCut => "Bass Cut",
            Self::WarmVoice => "Warm Voice",
            Self::BassBoost => "Bass+",
            Self::VShape => "V-Shape",
            Self::Vocal => "Vocal",
            Self::Rock => "Rock",
            Self::Electronic => "Electronic",
            Self::Classical => "Classical",
            Self::Podcast => "Podcast",
            Self::Footsteps => "Footsteps",
            Self::Immersive => "Immersive",
            Self::Competitive => "Competitive",
        }
    }

    pub fn category(self) -> EqPresetCategory {
        match self {
            Self::Flat => EqPresetCategory::Basic,
            Self::VoiceClarity | Self::BassCut | Self::WarmVoice => EqPresetCategory::Voice,
            Self::Footsteps | Self::Immersive | Self::Competitive => EqPresetCategory::Game,
            _ => EqPresetCategory::Music,
        }
    }

    fn bands(self) -> &'static [EqBand] {
        const FLAT: [EqBand; 5] = crate::settings::EQ_DEFAULT_LAYOUT;
        match self {
            Self::Flat => &FLAT,
            Self::VoiceClarity => {
                &const {
                    [
                        low_cut(80.0),
                        bell(250.0, -2.0, 1.0),
                        bell(2500.0, 3.0, 1.0),
                        high_shelf(8000.0, 2.0),
                    ]
                }
            }
            Self::BassCut => &const { [low_cut(120.0), low_shelf(250.0, -3.0)] },
            Self::WarmVoice => {
                &const {
                    [
                        low_shelf(150.0, 3.0),
                        bell(3000.0, -1.5, 1.0),
                        high_shelf(9000.0, -2.0),
                    ]
                }
            }
            Self::BassBoost => &const { [low_shelf(90.0, 6.0), bell(250.0, -1.0, 1.0)] },
            Self::VShape => {
                &const {
                    [
                        low_shelf(100.0, 5.0),
                        bell(1000.0, -3.0, 0.7),
                        high_shelf(8000.0, 5.0),
                    ]
                }
            }
            Self::Vocal => {
                &const {
                    [
                        bell(200.0, -2.0, 1.0),
                        bell(1500.0, 2.0, 0.8),
                        bell(3500.0, 3.0, 1.0),
                        high_shelf(10_000.0, 1.0),
                    ]
                }
            }
            Self::Rock => {
                &const {
                    [
                        low_shelf(100.0, 4.0),
                        bell(800.0, -2.0, 1.0),
                        bell(3000.0, 3.0, 1.0),
                        high_shelf(10_000.0, 3.0),
                    ]
                }
            }
            Self::Electronic => {
                &const {
                    [
                        low_shelf(60.0, 6.0),
                        bell(400.0, -2.0, 1.0),
                        bell(2000.0, 1.0, 1.0),
                        high_shelf(10_000.0, 4.0),
                    ]
                }
            }
            Self::Classical => {
                &const {
                    [
                        low_shelf(120.0, 2.0),
                        bell(500.0, -1.0, 0.7),
                        high_shelf(10_000.0, 2.0),
                    ]
                }
            }
            Self::Podcast => {
                &const {
                    [
                        low_cut(70.0),
                        bell(200.0, -2.0, 1.0),
                        bell(3000.0, 3.0, 1.0),
                        high_shelf(10_000.0, 1.0),
                    ]
                }
            }
            Self::Footsteps => {
                &const {
                    [
                        low_shelf(120.0, -4.0),
                        bell(250.0, -2.0, 1.0),
                        bell(2000.0, 5.0, 1.0),
                        bell(4000.0, 4.0, 1.0),
                        high_shelf(10_000.0, -2.0),
                    ]
                }
            }
            Self::Immersive => {
                &const {
                    [
                        low_shelf(80.0, 4.0),
                        bell(1000.0, -2.0, 0.5),
                        high_shelf(9000.0, 4.0),
                    ]
                }
            }
            Self::Competitive => {
                &const {
                    [
                        low_shelf(150.0, -6.0),
                        bell(3000.0, 6.0, 0.8),
                        high_shelf(12_000.0, -2.0),
                    ]
                }
            }
        }
    }

    pub fn params(self) -> EqParams {
        let mut params = EqParams::default();
        let bands = self.bands();
        params.bands[..bands.len()].copy_from_slice(bands);
        params.band_count = bands.len();
        params
    }
}

/// Starting point of a channel: CFX-005 (Voice), CFX-006 (Main, Music) and a user channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelTemplate {
    Main,
    Voice,
    Music,
    Custom,
}

/// One output channel: the stereo chain of §7.3 and the fader (CH-004).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ChannelSettings {
    pub volume_pct: f32,
    pub muted: bool,
    /// CFX-007: every module off at once, the fader still applies.
    pub bypass: bool,
    pub eq: Module<EqParams>,
    /// The preset the EQ was loaded from; cleared when bands are edited.
    pub eq_preset: Option<ChannelEqPreset>,
    /// RNNoise Lite per channel (CFX-003); the engine field is ignored.
    pub noise_suppression: Module<NoiseSuppressionParams>,
    pub compressor: Module<CompressorParams>,
    pub limiter: Module<LimiterParams>,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self::template(ChannelTemplate::Custom)
    }
}

impl ChannelSettings {
    pub fn template(template: ChannelTemplate) -> Self {
        let off = Self {
            volume_pct: 70.0,
            muted: false,
            bypass: false,
            eq: Module::off(ChannelEqPreset::Flat.params()),
            eq_preset: Some(ChannelEqPreset::Flat),
            noise_suppression: Module::off(NoiseSuppressionParams::new(NoiseEngine::Lite, 60.0)),
            compressor: Module::off(CompressorParams::from_amount(COMPRESSOR_AMOUNT.default)),
            limiter: Module::off(LimiterParams::ceiling(-3.0)),
        };
        match template {
            ChannelTemplate::Main => Self {
                volume_pct: 30.0,
                ..off
            },
            ChannelTemplate::Voice => Self {
                volume_pct: 50.0,
                eq: Module::on(ChannelEqPreset::VoiceClarity.params()),
                eq_preset: Some(ChannelEqPreset::VoiceClarity),
                noise_suppression: Module::on(off.noise_suppression.params),
                compressor: Module::on(CompressorParams::from_amount(40.0)),
                limiter: Module::on(LimiterParams::ceiling(-3.0)),
                ..off
            },
            ChannelTemplate::Music => Self {
                eq: Module::on(ChannelEqPreset::Flat.params()),
                ..off
            },
            ChannelTemplate::Custom => off,
        }
    }

    pub fn validate(&self) -> Result<(), ParameterError> {
        CHANNEL_VOLUME_PCT.check(self.volume_pct)?;
        self.eq.params.validate()?;
        self.noise_suppression.params.validate()?;
        self.compressor.params.validate()?;
        self.limiter.params.validate()?;
        Ok(())
    }

    /// Topology or latency changes need a new processor; the rest is retuned in place.
    pub fn requires_rebuild(&self, new: &Self) -> bool {
        self.bypass != new.bypass
            || self.eq.enabled != new.eq.enabled
            || self.noise_suppression.enabled != new.noise_suppression.enabled
            || self.compressor.enabled != new.compressor.enabled
            || self.limiter.enabled != new.limiter.enabled
            || self.limiter.params.lookahead_ms != new.limiter.params.lookahead_ms
    }

    pub fn latency_samples(&self) -> u32 {
        if self.bypass {
            return 0;
        }
        let noise = if self.noise_suppression.enabled {
            LITE_LATENCY_SAMPLES
        } else {
            0
        };
        let limiter = if self.limiter.enabled {
            u32::from(self.limiter.params.lookahead_ms) * SAMPLE_RATE_HZ / 1000
        } else {
            0
        };
        noise + limiter
    }

    /// Any module on and not bypassed: the `●` on the FX/EQ buttons of the mixer (§9.4).
    pub fn fx_active(&self) -> bool {
        !self.bypass
            && (self.noise_suppression.enabled || self.compressor.enabled || self.limiter.enabled)
    }
}

/// Lock-free meters of one channel, written once per block (CH-005, DSP-004).
#[derive(Debug, Default)]
pub struct ChannelTelemetry {
    enabled: AtomicBool,
    peak_left: AtomicU32,
    peak_right: AtomicU32,
    compressor_reduction: AtomicU32,
    limiter_reduction: AtomicU32,
}

/// One channel meter reading: post-fader peaks in dBFS, reductions in dB (≤ 0).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChannelMeters {
    pub peak_left_dbfs: f32,
    pub peak_right_dbfs: f32,
    pub compressor_reduction_db: f32,
    pub limiter_reduction_db: f32,
}

fn raise(slot: &AtomicU32, value: f32) {
    slot.fetch_max(value.to_bits(), Ordering::Relaxed);
}

fn take(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.swap(0, Ordering::Relaxed))
}

impl ChannelTelemetry {
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn take(&self) -> ChannelMeters {
        ChannelMeters {
            peak_left_dbfs: lin_to_db(take(&self.peak_left)),
            peak_right_dbfs: lin_to_db(take(&self.peak_right)),
            compressor_reduction_db: -take(&self.compressor_reduction),
            limiter_reduction_db: -take(&self.limiter_reduction),
        }
    }
}

pub struct ChannelProcessor {
    settings: ChannelSettings,
    eq: Option<[Equalizer; 2]>,
    noise: Option<[LiteNoiseSuppression; 2]>,
    compressor: Option<Compressor>,
    limiter: Option<Limiter>,
    /// Fader × master × mute, ramped over 10 ms (DSP-003).
    gain: Ramp<1>,
    master_gain: f32,
    telemetry: Arc<ChannelTelemetry>,
}

impl ChannelProcessor {
    /// `master_gain` is the linear Master fader (CH-006), already including its mute.
    pub fn new(
        settings: &ChannelSettings,
        master_gain: f32,
        telemetry: Arc<ChannelTelemetry>,
    ) -> Result<Self, ParameterError> {
        settings.validate()?;
        let build = |enabled: bool| !settings.bypass && enabled;
        let s = settings;
        Ok(Self {
            settings: *settings,
            eq: build(s.eq.enabled)
                .then(|| [Equalizer::new(&s.eq.params), Equalizer::new(&s.eq.params)]),
            noise: build(s.noise_suppression.enabled).then(|| {
                [
                    LiteNoiseSuppression::new(&s.noise_suppression.params),
                    LiteNoiseSuppression::new(&s.noise_suppression.params),
                ]
            }),
            compressor: build(s.compressor.enabled).then(|| Compressor::new(&s.compressor.params)),
            limiter: build(s.limiter.enabled).then(|| Limiter::new(&s.limiter.params)),
            gain: Ramp::new([fader_gain(settings, master_gain)]),
            master_gain,
            telemetry,
        })
    }

    pub fn settings(&self) -> &ChannelSettings {
        &self.settings
    }

    pub fn latency_samples(&self) -> u32 {
        self.noise.as_ref().map_or(0, |_| LITE_LATENCY_SAMPLES)
            + self.limiter.as_ref().map_or(0, Limiter::latency_samples)
    }

    /// Continuous changes with smoothing; returns false when a rebuild is required.
    /// RT-safe: no allocation or locks.
    pub fn retune(&mut self, settings: &ChannelSettings, master_gain: f32) -> bool {
        if self.settings.requires_rebuild(settings) {
            return false;
        }
        self.settings = *settings;
        self.master_gain = master_gain;
        if let Some(eq) = &mut self.eq {
            eq.iter_mut().for_each(|eq| eq.set(&settings.eq.params));
        }
        if let Some(noise) = &mut self.noise {
            noise
                .iter_mut()
                .for_each(|noise| noise.set(&settings.noise_suppression.params));
        }
        if let Some(module) = &mut self.compressor {
            module.set(&settings.compressor.params);
        }
        if let Some(module) = &mut self.limiter {
            module.set(&settings.limiter.params);
        }
        self.gain.set_target([fader_gain(settings, master_gain)]);
        true
    }

    /// Process one block in place; both slices have the same length.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let (left, right) = (&mut left[..len], &mut right[..len]);
        let silent = !self.gain.is_ramping() && self.gain.value()[0] == 0.0;
        let t = &*self.telemetry;
        let metering = t.enabled.load(Ordering::Relaxed);
        if silent {
            // A muted or zeroed channel costs nothing (NFR-R7); its meter shows silence.
            left.fill(0.0);
            right.fill(0.0);
            return;
        }
        for sample in left.iter_mut().chain(right.iter_mut()) {
            *sample = sanitize(*sample);
        }
        if let Some([l, r]) = &mut self.eq {
            l.process(left);
            r.process(right);
        }
        if let Some([l, r]) = &mut self.noise {
            l.process(left);
            r.process(right);
        }
        if let Some(module) = &mut self.compressor {
            module.process_stereo(left, right);
            if metering {
                raise(&t.compressor_reduction, -module.reduction_db());
            }
        }
        if let Some(module) = &mut self.limiter {
            module.process_stereo(left, right);
            if metering {
                raise(&t.limiter_reduction, -module.reduction_db());
            }
        }
        let (mut peak_l, mut peak_r) = (0.0f32, 0.0f32);
        let mut poisoned = false;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let [gain] = self.gain.next();
            poisoned |= !l.is_finite() || !r.is_finite();
            *l = sanitize(*l * gain);
            *r = sanitize(*r * gain);
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
        }
        if metering {
            raise(&t.peak_left, peak_l);
            raise(&t.peak_right, peak_r);
        }
        if poisoned {
            self.reset();
        }
    }

    /// Clear filter and envelope state after invalid output (NFR-S4).
    fn reset(&mut self) {
        if let Some(eq) = &mut self.eq {
            eq.iter_mut().for_each(Equalizer::reset);
        }
        if let Some(module) = &mut self.compressor {
            module.reset();
        }
        if let Some(module) = &mut self.limiter {
            module.reset();
        }
    }
}

fn fader_gain(settings: &ChannelSettings, master_gain: f32) -> f32 {
    if settings.muted {
        0.0
    } else {
        volume_gain(settings.volume_pct) * master_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;

    #[test]
    fn templates_match_cfx_005_and_006() {
        let voice = ChannelSettings::template(ChannelTemplate::Voice);
        assert_eq!(voice.volume_pct, 50.0);
        assert!(voice.eq.enabled && voice.eq_preset == Some(ChannelEqPreset::VoiceClarity));
        assert!(voice.noise_suppression.enabled);
        assert_eq!(voice.noise_suppression.params.intensity_pct, 60.0);
        assert_eq!(voice.compressor.params, CompressorParams::from_amount(40.0));
        assert!(voice.limiter.enabled && voice.limiter.params.ceiling_db == -3.0);
        let main = ChannelSettings::template(ChannelTemplate::Main);
        assert_eq!(main.volume_pct, 30.0);
        assert!(!main.eq.enabled && !main.fx_active());
        let music = ChannelSettings::template(ChannelTemplate::Music);
        assert_eq!(music.volume_pct, 70.0);
        assert!(music.eq.enabled && !music.fx_active());
        for preset in ChannelEqPreset::ALL {
            let mut settings = music;
            settings.eq.params = preset.params();
            assert_eq!(settings.validate(), Ok(()), "{preset:?}");
        }
    }

    #[test]
    fn cubic_fader_and_master_scale_a_bypassed_channel_exactly() {
        assert_eq!(volume_gain(100.0), 1.0);
        assert!((volume_db(50.0) - 60.0 * 0.5f32.log10()).abs() < 1e-4);
        assert_eq!(volume_gain(0.0), 0.0);
        let mut settings = ChannelSettings::template(ChannelTemplate::Main);
        settings.volume_pct = 50.0;
        let mut channel =
            ChannelProcessor::new(&settings, 0.5, Arc::default()).expect("valid channel");
        let mut left = [0.8f32; 512];
        let mut right = [-0.4f32; 512];
        assert_no_alloc(|| channel.process(&mut left, &mut right));
        assert!((left[511] - 0.8 * 0.125 * 0.5).abs() < 1e-6);
        assert!((right[511] + 0.4 * 0.125 * 0.5).abs() < 1e-6);
        assert_eq!(channel.latency_samples(), 0);
    }

    #[test]
    fn voice_template_runs_without_allocation_and_reports_its_latency() {
        let settings = ChannelSettings::template(ChannelTemplate::Voice);
        let telemetry = Arc::new(ChannelTelemetry::default());
        telemetry.set_enabled(true);
        let mut channel =
            ChannelProcessor::new(&settings, 1.0, telemetry.clone()).expect("valid channel");
        assert_eq!(channel.latency_samples(), LITE_LATENCY_SAMPLES);
        assert_eq!(settings.latency_samples(), LITE_LATENCY_SAMPLES);
        let mut left: Vec<f32> = (0..4800).map(|n| (n as f32 * 0.07).sin() * 0.9).collect();
        let mut right = left.clone();
        right[100] = f32::NAN;
        // Odd block sizes straddle the 480-sample RNNoise frame.
        assert_no_alloc(|| {
            for (l, r) in left.chunks_mut(333).zip(right.chunks_mut(333)) {
                channel.process(l, r);
            }
        });
        assert!(left.iter().chain(&right).all(|x| x.is_finite()));
        let meters = telemetry.take();
        assert!(meters.peak_left_dbfs > -60.0);
        let mut muted = settings;
        muted.muted = true;
        assert!(channel.retune(&muted, 1.0));
        let mut off = settings;
        off.noise_suppression.enabled = false;
        assert!(!channel.retune(&off, 1.0));
    }
}
