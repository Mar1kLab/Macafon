//! Built-in read-only microphone presets (PRD §6.3.10).

use crate::settings::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Natural,
    DiscordClear,
    NoisyRoom,
    Broadcast,
    WarmPodcast,
    Raw,
}

impl Preset {
    pub const ALL: [Self; 6] = [
        Self::Natural,
        Self::DiscordClear,
        Self::NoisyRoom,
        Self::Broadcast,
        Self::WarmPodcast,
        Self::Raw,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Natural => "Natural",
            Self::DiscordClear => "Discord Clear",
            Self::NoisyRoom => "Noisy Room",
            Self::Broadcast => "Broadcast",
            Self::WarmPodcast => "Warm Podcast",
            Self::Raw => "Raw",
        }
    }

    /// Stable identifier for CLI, IPC and config.
    pub fn id(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::DiscordClear => "discord-clear",
            Self::NoisyRoom => "noisy-room",
            Self::Broadcast => "broadcast",
            Self::WarmPodcast => "warm-podcast",
            Self::Raw => "raw",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.id() == id)
    }

    pub fn settings(self) -> ChainSettings {
        use EqCharacter::*;
        use NoiseEngine::*;
        let macros = match self {
            Self::Natural => PresetMacros {
                high_pass_freq_hz: 80.0,
                noise_engine: Lite,
                noise_intensity_pct: 60.0,
                expander_sensitivity_pct: Some(40.0),
                eq_character: Flat,
                eq_amount_pct: 100.0,
                compressor_amount_pct: 30.0,
                de_esser_intensity_pct: Some(30.0),
                limiter_ceiling_db: -1.0,
            },
            Self::DiscordClear => PresetMacros {
                high_pass_freq_hz: 100.0,
                noise_engine: Lite,
                noise_intensity_pct: 80.0,
                expander_sensitivity_pct: Some(60.0),
                eq_character: Clarity,
                eq_amount_pct: 50.0,
                compressor_amount_pct: 50.0,
                de_esser_intensity_pct: Some(40.0),
                limiter_ceiling_db: -1.0,
            },
            Self::NoisyRoom => PresetMacros {
                high_pass_freq_hz: 120.0,
                noise_engine: Studio,
                noise_intensity_pct: 90.0,
                expander_sensitivity_pct: Some(70.0),
                eq_character: Presence,
                eq_amount_pct: 40.0,
                compressor_amount_pct: 50.0,
                de_esser_intensity_pct: None,
                limiter_ceiling_db: -1.0,
            },
            Self::Broadcast => PresetMacros {
                high_pass_freq_hz: 80.0,
                noise_engine: Lite,
                noise_intensity_pct: 50.0,
                expander_sensitivity_pct: Some(40.0),
                eq_character: EqCharacter::Broadcast,
                eq_amount_pct: 70.0,
                compressor_amount_pct: 65.0,
                de_esser_intensity_pct: Some(50.0),
                limiter_ceiling_db: -1.0,
            },
            Self::WarmPodcast => PresetMacros {
                high_pass_freq_hz: 70.0,
                noise_engine: Lite,
                noise_intensity_pct: 50.0,
                expander_sensitivity_pct: None,
                eq_character: Warmth,
                eq_amount_pct: 60.0,
                compressor_amount_pct: 55.0,
                de_esser_intensity_pct: Some(45.0),
                limiter_ceiling_db: -1.5,
            },
            Self::Raw => return ChainSettings::raw(),
        };
        macros.settings()
    }
}

/// Simple-mode values used to build a preset; absent dynamics modules stay disabled.
struct PresetMacros {
    high_pass_freq_hz: f32,
    noise_engine: NoiseEngine,
    noise_intensity_pct: f32,
    expander_sensitivity_pct: Option<f32>,
    eq_character: EqCharacter,
    eq_amount_pct: f32,
    compressor_amount_pct: f32,
    de_esser_intensity_pct: Option<f32>,
    limiter_ceiling_db: f32,
}

impl PresetMacros {
    fn settings(self) -> ChainSettings {
        ChainSettings {
            high_pass: Module::on(HighPassParams::at(self.high_pass_freq_hz)),
            noise_suppression: Module::on(NoiseSuppressionParams::new(
                self.noise_engine,
                self.noise_intensity_pct,
            )),
            expander: match self.expander_sensitivity_pct {
                Some(percent) => Module::on(ExpanderParams::from_sensitivity(
                    percent,
                    DEFAULT_NOISE_FLOOR_DBFS,
                )),
                None => Module::off(ExpanderParams::default()),
            },
            eq: Module::on(EqParams::from_character(
                self.eq_character,
                self.eq_amount_pct,
            )),
            compressor: Module::on(CompressorParams::from_amount(self.compressor_amount_pct)),
            de_esser: match self.de_esser_intensity_pct {
                Some(percent) => Module::on(DeEsserParams::from_intensity(percent)),
                None => Module::off(DeEsserParams::default()),
            },
            limiter: Module::on(LimiterParams::ceiling(self.limiter_ceiling_db)),
            ..ChainSettings::raw()
        }
    }
}
