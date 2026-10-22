//! Macafon microphone DSP: mono, 48 kHz, 32-bit float (DSP-006/DSP-007).
//!
//! All storage is allocated when a processor is built; `process` never allocates or locks.

pub mod calibration;
mod chain;
pub mod channel;
mod compressor;
mod de_esser;
mod eq;
mod expander;
mod high_pass;
mod limiter;
mod loudness;
mod noise;
pub mod params;
mod presets;
pub mod settings;
mod svf;
mod util;

pub use chain::{
    Meters, MicrophoneProcessor, ProcessorBuildError, RetuneError, TapPoint, Telemetry,
};
pub use channel::{ChannelMeters, ChannelProcessor, ChannelSettings, ChannelTelemetry};
pub use compressor::static_curve_db;
pub use noise::{FRAME_SAMPLES, LITE_LATENCY_SAMPLES};
pub use noise::{STUDIO_LATENCY_SAMPLES, StudioProcessor};
pub use params::{INTENSITY, ParameterError, ParameterSpec, VOICE_THRESHOLD};
pub use presets::Preset;
pub use settings::{ChainSettings, InputChannel};
pub use util::Ramp;

pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// Reject invalid audio before it can poison filters or the recurrent model; flush denormals.
#[inline]
pub fn sanitize(sample: f32) -> f32 {
    if sample.is_finite() && !sample.is_subnormal() {
        sample
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{LimiterParams, Module, NoiseSuppressionParams};
    use assert_no_alloc::{AllocDisabler, assert_no_alloc};

    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    fn processor(settings: ChainSettings) -> MicrophoneProcessor {
        MicrophoneProcessor::new(&settings).expect("valid test settings")
    }

    fn speech_like(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE_HZ as f32;
                let envelope = (t * 3.0 * std::f32::consts::TAU).sin().max(0.0);
                envelope
                    * (0.3 * (t * 180.0 * std::f32::consts::TAU).sin()
                        + 0.05 * (t * 6500.0 * std::f32::consts::TAU).sin())
                    + 0.001 * ((i * 7919 % 1000) as f32 / 500.0 - 1.0)
            })
            .collect()
    }

    #[test]
    fn clip_is_latched_until_read_and_peak_holds_for_one_second() {
        let mut processor = processor(ChainSettings::raw());
        let telemetry = processor.telemetry();
        telemetry.set_enabled(true);
        assert_no_alloc(|| processor.process(&mut [1.25; 128]));
        let peak = telemetry.take();
        assert!(peak.input_clipped && peak.output_clipped);
        assert_eq!(peak.output_peak_hold_dbfs, peak.output_peak_dbfs);
        assert_no_alloc(|| processor.process(&mut [0.1; 4800]));
        let held = telemetry.take();
        assert!(!held.input_clipped && !held.output_clipped);
        assert_eq!(held.output_peak_hold_dbfs, peak.output_peak_hold_dbfs);
        for _ in 0..10 {
            assert_no_alloc(|| processor.process(&mut [0.1; 4800]));
        }
        assert!((telemetry.take().output_peak_hold_dbfs + 20.0).abs() < 1e-4);
    }

    #[test]
    fn raw_is_bit_exact_and_mute_is_digital_silence_without_allocation() {
        let input = [-0.5, 0.0, -0.0, 0.123, 1.25];
        let mut output = input;
        let mut raw = processor(ChainSettings::raw());
        assert_no_alloc(|| raw.process(&mut output));
        assert_eq!(input.map(f32::to_bits), output.map(f32::to_bits));
        let mut muted = processor(ChainSettings {
            muted: true,
            ..ChainSettings::default()
        });
        assert_no_alloc(|| muted.process(&mut output));
        assert_eq!(output, [0.0; 5]);
        let mut invalid = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(1),
        ];
        assert_no_alloc(|| raw.process(&mut invalid));
        assert_eq!(invalid, [0.0; 4]);
        let mut bypass = processor(ChainSettings {
            bypass: true,
            ..Preset::Broadcast.settings()
        });
        assert_eq!(bypass.latency_samples(), 0);
        let mut output = input;
        assert_no_alloc(|| bypass.process(&mut output));
        assert_eq!(input.map(f32::to_bits), output.map(f32::to_bits));
    }

    #[test]
    fn muted_chain_still_meters_the_input_while_watched() {
        let mut chain = processor(ChainSettings {
            muted: true,
            ..ChainSettings::raw()
        });
        let telemetry = chain.telemetry();
        telemetry.set_enabled(true);
        let mut block = [0.5f32; 256];
        assert_no_alloc(|| chain.process(&mut block));
        assert_eq!(block.map(f32::to_bits), [0.0f32; 256].map(f32::to_bits));
        let meters = telemetry.take();
        assert!((meters.raw_peak_dbfs + 6.02).abs() < 0.01, "{meters:?}");
        assert!((meters.input_peak_dbfs + 6.02).abs() < 0.01, "{meters:?}");
        assert!(meters.output_peak_dbfs <= -119.0, "{meters:?}");
    }

    #[test]
    fn lite_handles_variable_quantum_and_aligns_dry_without_allocation() {
        let lite = |intensity_pct| ChainSettings {
            noise_suppression: Module::on(NoiseSuppressionParams {
                intensity_pct,
                ..NoiseSuppressionParams::default()
            }),
            ..ChainSettings::raw()
        };
        let input: Vec<_> = (0..4096).map(|i| (i as f32 * 0.07).sin() * 0.3).collect();
        let mut output = input.clone();
        let mut dry = processor(lite(0.0));
        assert_no_alloc(|| {
            for block in output.chunks_mut(128) {
                dry.process(block);
            }
        });
        let latency = dry.latency_samples() as usize;
        assert_eq!(latency, LITE_LATENCY_SAMPLES as usize);
        assert!(output[..latency].iter().all(|sample| *sample == 0.0));
        assert_eq!(&output[latency..], &input[..input.len() - latency]);
        // Construct on the control thread, then run the first frame on a fresh RT thread.
        let mut fresh = processor(lite(70.0));
        std::thread::spawn(move || {
            let mut block = [0.25; 1024];
            block[0] = f32::NAN;
            block[1] = f32::INFINITY;
            assert_no_alloc(|| fresh.process(&mut block));
            assert!(block.iter().all(|sample| sample.is_finite()));
        })
        .join()
        .expect("RT test thread succeeds");
    }

    #[test]
    fn lite_presets_are_block_size_invariant_and_allocation_free_on_a_fresh_thread() {
        let input = speech_like(SAMPLE_RATE_HZ as usize);
        for preset in Preset::ALL {
            if preset.settings().noise_suppression.params.engine == settings::NoiseEngine::Studio {
                assert!(matches!(
                    MicrophoneProcessor::new(&preset.settings()),
                    Err(ProcessorBuildError::StudioUnavailable)
                ));
                continue;
            }
            let mut whole = processor(preset.settings());
            let mut split = processor(preset.settings());
            let telemetry = split.telemetry();
            telemetry.set_enabled(true);
            let (a, b) = std::thread::spawn({
                let (mut a, mut b) = (input.clone(), input.clone());
                move || {
                    assert_no_alloc(|| {
                        whole.process(&mut a);
                        for block in b.chunks_mut(73) {
                            split.process(block);
                        }
                    });
                    (a, b)
                }
            })
            .join()
            .expect("RT test thread succeeds");
            assert_eq!(a, b, "{preset:?}");
            assert!(
                b.iter()
                    .all(|y| y.is_finite() && !y.is_subnormal() && y.abs() <= 1.0),
                "{preset:?}"
            );
            let meters = telemetry.take();
            assert!(meters.output_peak_dbfs > -30.0, "{preset:?}: {meters:?}");
            assert!(meters.compressor_reduction_db <= 0.0 && meters.limiter_reduction_db <= 0.0);
        }
    }

    #[test]
    fn retune_ramps_parameters_and_mute_without_clicks() {
        let mut chain = processor(ChainSettings::raw());
        let mut louder = ChainSettings::raw();
        louder.gain.params.gain_db = 12.0;
        chain.retune(&louder).expect("continuous change");
        let mut block = [0.25f32; 960];
        assert_no_alloc(|| chain.process(&mut block));
        let max_step = block
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.002, "gain jumped by {max_step}");
        assert!((block[959] - 0.25 * 10f32.powf(0.6)).abs() < 1e-4);

        louder.muted = true;
        chain.retune(&louder).expect("mute is continuous");
        assert_no_alloc(|| chain.process(&mut block));
        assert!(block[0] > 0.9 && block[479] == 0.0 && block[959] == 0.0);
        assert_no_alloc(|| chain.process(&mut block));
        assert_eq!(block, [0.0; 960]);

        let mut structural = louder;
        structural.limiter.enabled = true;
        assert_eq!(chain.retune(&structural), Err(RetuneError::RebuildRequired));
        structural.gain.params.gain_db = 30.0;
        assert!(matches!(
            chain.retune(&structural),
            Err(RetuneError::Invalid(_))
        ));
    }

    #[test]
    fn latency_is_the_sum_of_enabled_modules() {
        let mut settings = Preset::Natural.settings();
        assert_eq!(processor(settings).latency_samples(), LITE_LATENCY_SAMPLES);
        for preset in Preset::ALL {
            let mut settings = preset.settings();
            settings.limiter.params.lookahead_ms = 5;
            // The standalone DSP crate requires a separately prepared Studio worker.
            settings.noise_suppression.params.engine = settings::NoiseEngine::Lite;
            assert_eq!(
                processor(settings).latency_samples(),
                settings.latency_samples()
            );
        }
        settings.limiter.params = LimiterParams {
            lookahead_ms: 2,
            ..settings.limiter.params
        };
        assert_eq!(
            processor(settings).latency_samples(),
            LITE_LATENCY_SAMPLES + 96
        );
        settings.noise_suppression.enabled = false;
        settings.limiter.enabled = false;
        assert_eq!(processor(settings).latency_samples(), 0);
    }

    #[test]
    fn invalid_parameters_are_rejected_before_processing() {
        for value in [f32::NAN, f32::INFINITY, -1.0, 101.0] {
            let mut settings = ChainSettings::default();
            settings.noise_suppression.params.intensity_pct = value;
            assert!(MicrophoneProcessor::new(&settings).is_err());
            let mut settings = ChainSettings::default();
            settings.noise_suppression.params.voice_threshold_pct = value;
            assert!(MicrophoneProcessor::new(&settings).is_err());
        }
        let mut settings = ChainSettings::default();
        settings.limiter.params.lookahead_ms = 3;
        assert!(MicrophoneProcessor::new(&settings).is_err());
        settings = ChainSettings::default();
        settings.expander.params.ratio = f32::INFINITY;
        assert!(MicrophoneProcessor::new(&settings).is_ok());
    }
}
