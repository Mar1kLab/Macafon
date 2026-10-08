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

    pub fn settings(self) -> ChainSettings {
        use EqCharacter::*;
        use NoiseEngine::*;
        // (HPF Hz, NS engine and %, expander %, EQ character and amount %, comp %, de-ess %,
        // limiter ceiling dBFS); None leaves the module off.
        let (hpf, ns, expander, eq, compressor, de_esser, limiter) = match self {
            Self::Natural => (
                80.0,
                (Lite, 60.0),
                Some(40.0),
                (Flat, 100.0),
                30.0,
                Some(30.0),
                -1.0,
            ),
            Self::DiscordClear => (
                100.0,
                (Lite, 80.0),
                Some(60.0),
                (Clarity, 50.0),
                50.0,
                Some(40.0),
                -1.0,
            ),
            Self::NoisyRoom => (
                120.0,
                (Studio, 90.0),
                Some(70.0),
                (Presence, 40.0),
                50.0,
                None,
                -1.0,
            ),
            Self::Broadcast => (
                80.0,
                (Lite, 50.0),
                Some(40.0),
                (EqCharacter::Broadcast, 70.0),
                65.0,
                Some(50.0),
                -1.0,
            ),
            Self::WarmPodcast => (
                70.0,
                (Lite, 50.0),
                None,
                (Warmth, 60.0),
                55.0,
                Some(45.0),
                -1.5,
            ),
            Self::Raw => return ChainSettings::raw(),
        };
        ChainSettings {
            high_pass: Module::on(HighPassParams::at(hpf)),
            noise_suppression: Module::on(NoiseSuppressionParams::new(ns.0, ns.1)),
            expander: match expander {
                Some(percent) => Module::on(ExpanderParams::from_sensitivity(
                    percent,
                    DEFAULT_NOISE_FLOOR_DBFS,
                )),
                None => Module::off(ExpanderParams::default()),
            },
            eq: Module::on(EqParams::from_character(eq.0, eq.1)),
            compressor: Module::on(CompressorParams::from_amount(compressor)),
            de_esser: match de_esser {
                Some(percent) => Module::on(DeEsserParams::from_intensity(percent)),
                None => Module::off(DeEsserParams::default()),
            },
            limiter: Module::on(LimiterParams::ceiling(limiter)),
            ..ChainSettings::raw()
        }
    }
}
