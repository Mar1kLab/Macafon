//! Automatic setup from a short recording (PRD §6.5): silence, then speech, both captured
//! before the chain. Runs off the RT thread; allocation is fine here.

use crate::params::{EXPANDER_THRESHOLD_DBFS, GAIN_DB, INTENSITY};
use crate::settings::{ChainSettings, HighPassParams, SPEECH_REFERENCE_DBFS};
use crate::svf::{FilterKind, Svf, coefficients};
use crate::util::{interpolate, power_to_db};
use crate::{MicrophoneProcessor, SAMPLE_RATE_HZ};

/// CAL-001: "Stay silent" phase.
pub const SILENCE_SAMPLES: usize = 4 * SAMPLE_RATE_HZ as usize;
/// CAL-002: "Read this sentence" phase.
pub const SPEECH_SAMPLES: usize = 6 * SAMPLE_RATE_HZ as usize;

/// One downward endpoint-volume step, on the device's dB grid, while the raw input clips.
pub const INPUT_LEVEL_STEP_DB: f32 = 6.0;

/// Samples at or above this magnitude count as clipped (−0.1 dBFS).
pub const CLIP_LEVEL: f32 = 0.989;
/// A single overshoot (a click) is not clipping; this many clipped samples are.
pub const CLIPPED_SAMPLES: usize = 3;

/// Flat tops shorter than this are ordinary low-frequency peaks or quantisation.
const FLAT_TOP_SAMPLES: usize = 10;
/// Several flat tops, not one odd pop, make clipping.
const FLAT_TOPS: usize = 3;
/// Samples this close to the recording's peak count as the same clipped value.
const FLAT_TOP_TOLERANCE: f32 = 2e-5;
/// Flat tops quieter than this (-20 dBFS) are not a clipping problem worth a level change.
const FLAT_TOP_MIN_PEAK: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clipping {
    /// Samples reach full scale: a lower capture level helps.
    FullScale,
    /// Repeated flat tops below full scale: the converter clipped before a digital level
    /// stage (often the system volume of a USB microphone), so lowering that stage only
    /// moves the flat tops down. Only the analog gain (a knob, a vendor app) can fix it.
    FlatTop { level: f32 },
}

/// CAL-005: whether the raw input clipped, and where.
pub fn clipping(samples: &[f32]) -> Option<Clipping> {
    if samples.iter().filter(|x| x.abs() >= CLIP_LEVEL).count() >= CLIPPED_SAMPLES {
        return Some(Clipping::FullScale);
    }
    let peak = samples.iter().fold(0.0f32, |peak, x| peak.max(x.abs()));
    if peak < FLAT_TOP_MIN_PEAK {
        return None;
    }
    let near = peak * (1.0 - FLAT_TOP_TOLERANCE);
    let (mut run, mut tops) = (0usize, 0usize);
    for x in samples {
        if x.abs() >= near {
            run += 1;
            if run == FLAT_TOP_SAMPLES {
                tops += 1;
            }
        } else {
            run = 0;
        }
    }
    (tops >= FLAT_TOPS).then_some(Clipping::FlatTop { level: peak })
}

pub fn is_clipped(samples: &[f32]) -> bool {
    clipping(samples).is_some()
}

/// One downward endpoint-volume adjustment, on the device's dB grid. Never raises a level.
/// Callers use it only for a clipped input and check fresh capture between steps.
pub fn input_level_target_db(current_db: f32, minimum_db: f32, increment_db: f32) -> Option<f32> {
    if ![current_db, minimum_db, increment_db]
        .iter()
        .all(|value| value.is_finite())
        || increment_db <= 0.0
        || current_db <= minimum_db
    {
        return None;
    }
    let target_db = minimum_db
        + ((current_db - INPUT_LEVEL_STEP_DB - minimum_db) / increment_db).floor() * increment_db;
    Some(target_db.max(minimum_db))
}

/// Analysis windows of 20 ms.
const WINDOW_SAMPLES: usize = SAMPLE_RATE_HZ as usize / 50;
/// The loudest fifth of the silence (a cough, a click) does not count towards the floor.
const FLOOR_QUANTILE: f32 = 0.8;
/// Windows this far above the floor count as speech.
const SPEECH_MARGIN_DB: f32 = 10.0;
/// Relative gate (as BS.1770): windows this far under the gated mean are pauses and word
/// tails. Without it a quiet room counts more tails than a noisy one and Gain drifts.
const RELATIVE_GATE_DB: f32 = 10.0;
/// Calibrated speech peaks stay at or below full scale; the noise models keep another 6 dB
/// of headroom and the limiter catches the rest.
const SPEECH_PEAK_CEILING_DBFS: f32 = 0.0;
/// At least this much detected speech is needed for a meaningful level.
const MIN_SPEECH_WINDOWS: usize = 30;
/// HPF moves up when most of the noise power sits below this frequency (CAL-003).
const LOW_FREQUENCY_HZ: f32 = 100.0;
const LOW_FREQUENCY_SHARE: f32 = 0.5;
const LOW_FREQUENCY_HPF_HZ: f32 = 120.0;
/// Noise floors below this are inaudible; their spectrum does not matter.
const AUDIBLE_FLOOR_DBFS: f32 = -75.0;
/// CAL-003: expander threshold = floor + 6 dB, but at least 12 dB under the speech level.
const EXPANDER_FLOOR_MARGIN_DB: f32 = 6.0;
const EXPANDER_SPEECH_MARGIN_DB: f32 = 12.0;
/// CAL-003: SNR above 40 dB → NS 40%, below 20 dB → NS 90% and a Studio suggestion.
const CLEAN_SNR_DB: f32 = 40.0;
const NOISY_SNR_DB: f32 = 20.0;
/// The chain settles (NS, expander envelopes) before the processed floor is measured.
const SETTLE_SAMPLES: usize = SAMPLE_RATE_HZ as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum NoiseLevel {
    Low,
    Medium,
    High,
}

/// What calibration measured and changed (CAL-004).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CalibrationReport {
    /// Levels of the physical microphone, before Gain.
    pub noise_floor_dbfs: f32,
    pub speech_rms_dbfs: f32,
    pub speech_peak_dbfs: f32,
    pub snr_db: f32,
    /// Same recording through Gain, High-pass and Noise suppression (the expander would only
    /// close in pauses and overstate it).
    pub snr_after_db: f32,
    pub noise_level: NoiseLevel,
    /// Most of the noise is below 100 Hz (hum, rumble), so the high-pass moved up.
    pub low_frequency_noise: bool,
    /// SNR below 20 dB: Studio noise suppression would help.
    pub suggest_studio: bool,
    pub gain_db: f32,
    pub high_pass_freq_hz: f32,
    pub noise_intensity_pct: f32,
    pub expander_threshold_db: f32,
    /// Noise floor after Noise suppression, before Gain: the expander's reference.
    #[cfg_attr(feature = "serde", serde(default))]
    pub expander_floor_dbfs: f32,
    /// CAL-005: how far calibration lowered the system capture level (<= 0; 0 = untouched).
    #[cfg_attr(feature = "serde", serde(default))]
    pub input_level_change_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum CalibrationError {
    /// CAL-005: the input clipped before Macafon; only the system/interface level can fix it.
    #[error("the microphone clipped; lower its level in the system or on the interface")]
    Clipped { peak_dbfs: f32 },
    #[error("no speech was detected; check the microphone and speak during the second step")]
    NoSpeech,
    #[error("the recording is shorter than both calibration steps")]
    TooShort,
}

/// Analyse a recording of `SILENCE_SAMPLES` of silence followed by `SPEECH_SAMPLES` of speech
/// and return `current` with Gain, High-pass, Noise suppression and Expander calibrated.
pub fn calibrate(
    recording: &[f32],
    current: &ChainSettings,
) -> Result<(ChainSettings, CalibrationReport), CalibrationError> {
    if recording.len() < SILENCE_SAMPLES + SPEECH_SAMPLES {
        return Err(CalibrationError::TooShort);
    }
    let recording = &recording[..SILENCE_SAMPLES + SPEECH_SAMPLES];
    let (silence, speech) = recording.split_at(SILENCE_SAMPLES);
    let peak = recording.iter().fold(0.0f32, |peak, x| peak.max(x.abs()));
    if is_clipped(recording) {
        return Err(CalibrationError::Clipped {
            peak_dbfs: amplitude_to_db(peak),
        });
    }

    let noise_floor_dbfs = floor_dbfs(silence);
    let speech_windows = speech_windows(speech, noise_floor_dbfs + SPEECH_MARGIN_DB);
    if speech_windows.len() < MIN_SPEECH_WINDOWS {
        return Err(CalibrationError::NoSpeech);
    }
    let speech_rms_dbfs = level_dbfs(speech, &speech_windows);
    let speech_peak_dbfs = amplitude_to_db(speech.iter().fold(0.0f32, |p, x| p.max(x.abs())));
    let snr_db = speech_rms_dbfs - noise_floor_dbfs;

    // Speech RMS to the reference, but never push the measured peaks over full scale:
    // voices with a high crest factor get the rest from the compressor's makeup.
    let level_gain_db =
        ((SPEECH_REFERENCE_DBFS - speech_rms_dbfs) / GAIN_DB.step).round() * GAIN_DB.step;
    let peak_gain_db =
        ((SPEECH_PEAK_CEILING_DBFS - speech_peak_dbfs) / GAIN_DB.step).floor() * GAIN_DB.step;
    let gain_db = level_gain_db
        .min(peak_gain_db)
        .clamp(GAIN_DB.min, GAIN_DB.max);
    let low_frequency_noise = noise_floor_dbfs > AUDIBLE_FLOOR_DBFS
        && low_frequency_share(silence) >= LOW_FREQUENCY_SHARE;
    let high_pass_freq_hz = if low_frequency_noise {
        LOW_FREQUENCY_HPF_HZ
    } else {
        HighPassParams::default().freq_hz
    };
    let noise_intensity_pct = interpolate(snr_db, &[(NOISY_SNR_DB, 90.0), (CLEAN_SNR_DB, 40.0)])
        .clamp(INTENSITY.min, INTENSITY.max)
        .round();

    let mut settings = *current;
    settings.gain.enabled = true;
    settings.gain.params.gain_db = gain_db;
    settings.high_pass.enabled = true;
    settings.high_pass.params.freq_hz = high_pass_freq_hz;
    settings.noise_suppression.enabled = true;
    settings.noise_suppression.params.intensity_pct = noise_intensity_pct;

    // The expander follows Noise suppression: its floor is what is left after NS.
    let (expander_floor_dbfs, snr_after_db) = match clean_signal(recording, &settings) {
        Some(output) => {
            let (silence, speech) = output.split_at(SILENCE_SAMPLES);
            let floor = floor_dbfs(&silence[SETTLE_SAMPLES..]) - gain_db;
            (floor, level_dbfs(speech, &speech_windows) - gain_db - floor)
        }
        None => (noise_floor_dbfs, snr_db),
    };
    let expander_threshold_db = (expander_floor_dbfs + gain_db + EXPANDER_FLOOR_MARGIN_DB)
        .min(speech_rms_dbfs + gain_db - EXPANDER_SPEECH_MARGIN_DB)
        .clamp(EXPANDER_THRESHOLD_DBFS.min, EXPANDER_THRESHOLD_DBFS.max)
        .round();
    settings.expander.enabled = true;
    settings.expander.params.threshold_db = expander_threshold_db;

    let noise_level = if snr_db >= CLEAN_SNR_DB {
        NoiseLevel::Low
    } else if snr_db >= NOISY_SNR_DB {
        NoiseLevel::Medium
    } else {
        NoiseLevel::High
    };
    Ok((
        settings,
        CalibrationReport {
            noise_floor_dbfs,
            speech_rms_dbfs,
            speech_peak_dbfs,
            snr_db,
            snr_after_db,
            noise_level,
            low_frequency_noise,
            suggest_studio: snr_db < NOISY_SNR_DB,
            gain_db,
            high_pass_freq_hz,
            noise_intensity_pct,
            expander_threshold_db,
            expander_floor_dbfs,
            input_level_change_db: 0.0,
        },
    ))
}

fn amplitude_to_db(amplitude: f32) -> f32 {
    power_to_db(amplitude * amplitude)
}

fn window_powers(signal: &[f32]) -> impl Iterator<Item = f32> + '_ {
    signal
        .chunks_exact(WINDOW_SAMPLES)
        .map(|window| window.iter().map(|x| x * x).sum::<f32>() / WINDOW_SAMPLES as f32)
}

/// Mean power of the quieter windows, in dBFS.
fn floor_dbfs(silence: &[f32]) -> f32 {
    let mut powers: Vec<f32> = window_powers(silence).collect();
    powers.sort_by(f32::total_cmp);
    let kept = &powers[..((powers.len() as f32 * FLOOR_QUANTILE) as usize).max(1)];
    power_to_db(kept.iter().sum::<f32>() / kept.len() as f32)
}

/// Indices of speech windows: louder than `threshold_dbfs` and within the relative gate of
/// their mean.
fn speech_windows(speech: &[f32], threshold_dbfs: f32) -> Vec<usize> {
    let powers: Vec<f32> = window_powers(speech).collect();
    let gated: Vec<usize> = (0..powers.len())
        .filter(|&index| power_to_db(powers[index]) > threshold_dbfs)
        .collect();
    if gated.is_empty() {
        return gated;
    }
    let mean = gated.iter().map(|&index| powers[index]).sum::<f32>() / gated.len() as f32;
    let relative_dbfs = power_to_db(mean) - RELATIVE_GATE_DB;
    gated
        .into_iter()
        .filter(|&index| power_to_db(powers[index]) > relative_dbfs)
        .collect()
}

/// Mean power of the given windows, in dBFS.
fn level_dbfs(signal: &[f32], windows: &[usize]) -> f32 {
    let powers: Vec<f32> = window_powers(signal).collect();
    let total: f32 = windows.iter().map(|&index| powers[index]).sum();
    power_to_db(total / windows.len() as f32)
}

/// Share of the noise power below 100 Hz (24 dB/oct low-pass).
fn low_frequency_share(silence: &[f32]) -> f32 {
    let lowpass = coefficients(
        FilterKind::HighCut,
        LOW_FREQUENCY_HZ,
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
    );
    let (mut first, mut second) = (Svf::new(lowpass), Svf::new(lowpass));
    let (mut low, mut total) = (0.0f64, 0.0f64);
    for &x in silence {
        let y = second.tick(first.tick(x));
        low += f64::from(y * y);
        total += f64::from(x * x);
    }
    if total > 0.0 {
        (low / total) as f32
    } else {
        0.0
    }
}

/// The recording through Gain, High-pass and Noise suppression only, latency removed.
/// Studio runs on a worker the analysis cannot build, so Lite stands in for it.
fn clean_signal(recording: &[f32], settings: &ChainSettings) -> Option<Vec<f32>> {
    let mut settings = ChainSettings {
        muted: false,
        bypass: false,
        gain: settings.gain,
        high_pass: settings.high_pass,
        noise_suppression: settings.noise_suppression,
        ..ChainSettings::raw()
    };
    settings.noise_suppression.params.engine = crate::settings::NoiseEngine::Lite;
    let mut processor = MicrophoneProcessor::new(&settings).ok()?;
    let latency = processor.latency_samples() as usize;
    let mut output = recording.to_vec();
    output.extend(std::iter::repeat_n(0.0, latency));
    output
        .chunks_mut(crate::FRAME_SAMPLES)
        .for_each(|block| processor.process(block));
    output.drain(..latency);
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Preset;
    use crate::util::db_to_lin;

    #[test]
    fn flat_tops_below_full_scale_count_as_clipping() {
        // A converter clip after a digital −7 dB level: identical runs at 0.45.
        let mut clipped: Vec<f32> = (0..SAMPLE_RATE_HZ as usize)
            .map(|n| 0.9 * (n as f32 * 200.0 * std::f32::consts::TAU / SAMPLE_RATE_HZ as f32).sin())
            .map(|x| x.clamp(-0.45, 0.45))
            .collect();
        assert_eq!(clipping(&clipped), Some(Clipping::FlatTop { level: 0.45 }));
        // A clean low tone at the same level has no flat tops.
        for (n, x) in clipped.iter_mut().enumerate() {
            *x = 0.45 * (n as f32 * 30.0 * std::f32::consts::TAU / SAMPLE_RATE_HZ as f32).sin();
        }
        assert_eq!(clipping(&clipped), None);
    }

    #[test]
    fn endpoint_steps_down_only_for_real_clipping() {
        // Loud but clean speech (peaks at −3 dBFS) leaves the user's level alone.
        let mut speech = vec![0.0f32; SAMPLE_RATE_HZ as usize];
        speech[100] = 0.7;
        assert!(!is_clipped(&speech));
        // A single full-scale click is not clipping either.
        speech[200] = 1.0;
        assert!(!is_clipped(&speech));
        speech[300] = -1.0;
        speech[301] = 0.995;
        assert!(is_clipped(&speech));

        assert_no_alloc::assert_no_alloc(|| {
            let mut endpoint_db = 0.0;
            for expected_db in [-6.0, -12.0, -18.0] {
                let next_db = input_level_target_db(endpoint_db, -60.0, 1.5)
                    .expect("a clipped input can be lowered");
                assert_eq!(next_db, expected_db);
                endpoint_db = next_db;
            }
            // The grid floors the step, the minimum bounds it, and nothing ever rises.
            assert_eq!(input_level_target_db(-1.0, -60.0, 1.5), Some(-7.5));
            assert_eq!(input_level_target_db(-58.0, -60.0, 1.5), Some(-60.0));
            assert_eq!(input_level_target_db(-60.0, -60.0, 1.5), None);
            assert_eq!(input_level_target_db(-3.0, -60.0, 0.0), None);
            assert_eq!(input_level_target_db(f32::NAN, -60.0, 1.5), None);
        });
    }

    /// Deterministic white noise in [−1, 1].
    fn noise(len: usize, seed: u32) -> impl Iterator<Item = f32> {
        let mut state = seed;
        (0..len).map(move |_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        })
    }

    /// White noise at `floor_dbfs` RMS, then 3 Hz-gated harmonic "speech" at `speech_dbfs`
    /// RMS during its active half over the same noise.
    fn recording(floor_dbfs: f32, speech_dbfs: f32, hum_dbfs: Option<f32>) -> Vec<f32> {
        let noise_gain = db_to_lin(floor_dbfs) * 3f32.sqrt();
        let hum_gain = hum_dbfs.map_or(0.0, |db| db_to_lin(db) * 2f32.sqrt());
        let voice_gain = db_to_lin(speech_dbfs) * 2f32.sqrt() / 0.5f32.sqrt();
        noise(SILENCE_SAMPLES + SPEECH_SAMPLES, 7)
            .enumerate()
            .map(|(i, n)| {
                let t = i as f32 / SAMPLE_RATE_HZ as f32;
                let hum = hum_gain * (t * 50.0 * std::f32::consts::TAU).sin();
                let voice = if i >= SILENCE_SAMPLES && (t * 3.0).fract() < 0.5 {
                    voice_gain
                        * 0.5
                        * ((t * 200.0 * std::f32::consts::TAU).sin()
                            + (t * 400.0 * std::f32::consts::TAU).sin())
                } else {
                    0.0
                };
                n * noise_gain + hum + voice
            })
            .collect()
    }

    #[test]
    fn measures_levels_and_applies_the_cal_003_rules() {
        let current = Preset::WarmPodcast.settings();
        let (settings, report) =
            calibrate(&recording(-65.0, -30.0, None), &current).expect("calibrates");
        assert!((report.noise_floor_dbfs + 65.0).abs() < 1.0, "{report:?}");
        assert!((report.speech_rms_dbfs + 30.0).abs() < 1.0, "{report:?}");
        assert!((report.snr_db - 35.0).abs() < 1.5);
        assert_eq!(report.noise_level, NoiseLevel::Medium);
        // Speech to −18 dBFS: +12 dB in 0.5 dB steps (partial windows at phrase edges).
        assert!((report.gain_db - 12.0).abs() <= 0.5, "{report:?}");
        assert_eq!(report.gain_db % 0.5, 0.0);
        // The expander sees the floor left after NS: post-NS floor + gain + 6, under
        // speech − 12 = −30.
        assert!(
            report.expander_floor_dbfs <= report.noise_floor_dbfs,
            "{report:?}"
        );
        assert_eq!(
            report.expander_threshold_db,
            (report.expander_floor_dbfs + report.gain_db + 6.0).round(),
            "{report:?}"
        );
        // SNR 35 dB → between 40% (40 dB) and 90% (20 dB).
        assert!((report.noise_intensity_pct - 52.0).abs() <= 2.0);
        assert!(!report.low_frequency_noise && !report.suggest_studio);
        assert_eq!(report.high_pass_freq_hz, 80.0);
        // NS alone never makes it worse (the synthetic tones are not speech to RNNoise).
        assert!(report.snr_after_db.is_finite() && report.snr_after_db > report.snr_db - 1.0);
        assert_eq!(report.input_level_change_db, 0.0);
        // Calibration enables what it tuned and keeps the rest of the chain.
        assert!(settings.expander.enabled && settings.noise_suppression.enabled);
        assert_eq!(settings.gain.params.gain_db, report.gain_db);
        assert_eq!(settings.eq, current.eq);
        assert_eq!(settings.compressor, current.compressor);
        assert_eq!(settings.validate(), Ok(()));
    }

    #[test]
    fn noisy_rooms_with_hum_get_strong_suppression_and_a_higher_cut() {
        let (_, report) = calibrate(
            &recording(-45.0, -28.0, Some(-40.0)),
            &Preset::Natural.settings(),
        )
        .expect("calibrates");
        assert!(report.low_frequency_noise, "{report:?}");
        assert_eq!(report.high_pass_freq_hz, 120.0);
        assert!(report.snr_db < 20.0);
        assert_eq!(report.noise_intensity_pct, 90.0);
        assert!(report.suggest_studio);
        assert_eq!(report.noise_level, NoiseLevel::High);
        // Before NS the floor rule would put the threshold above the speech (−45 + 10 + 6);
        // after strong NS it sits far under the speech and no longer eats word endings.
        assert!(
            report.expander_floor_dbfs < report.noise_floor_dbfs - 10.0,
            "{report:?}"
        );
        assert!(report.expander_threshold_db < -30.0 - 6.0, "{report:?}");
    }

    #[test]
    fn gain_keeps_peaky_speech_under_full_scale_and_ignores_room_tails() {
        // A short burst 20 dB over the phrase sets the peak: Gain stops at full scale.
        let mut peaky = recording(-65.0, -30.0, None);
        let burst = SILENCE_SAMPLES + SAMPLE_RATE_HZ as usize;
        for (offset, sample) in peaky[burst..burst + 96].iter_mut().enumerate() {
            *sample = 0.5 * (offset as f32 * 0.3).sin();
        }
        let (_, report) = calibrate(&peaky, &ChainSettings::raw()).expect("calibrates");
        assert!(
            report.speech_peak_dbfs + report.gain_db <= 0.0,
            "{report:?}"
        );
        assert!(report.gain_db < 12.0, "{report:?}");
        // Quiet tails 15 dB under the phrase do not drag the measured speech level down.
        let mut tails = recording(-80.0, -30.0, None);
        for (index, sample) in tails[SILENCE_SAMPLES..].iter_mut().enumerate() {
            let t = index as f32 / SAMPLE_RATE_HZ as f32;
            if (t * 3.0).fract() >= 0.5 && (t * 3.0).fract() < 0.8 {
                *sample += 0.03 * (t * 300.0 * std::f32::consts::TAU).sin();
            }
        }
        let (_, report) = calibrate(&tails, &ChainSettings::raw()).expect("calibrates");
        assert!((report.speech_rms_dbfs + 30.0).abs() < 1.5, "{report:?}");
    }

    #[test]
    fn clipping_and_missing_speech_are_reported() {
        let mut clipped = recording(-65.0, -30.0, None);
        clipped[SILENCE_SAMPLES + 100..SILENCE_SAMPLES + 104].fill(1.0);
        assert!(matches!(
            calibrate(&clipped, &ChainSettings::raw()),
            Err(CalibrationError::Clipped { .. })
        ));
        let silent = recording(-65.0, -120.0, None);
        assert_eq!(
            calibrate(&silent, &ChainSettings::raw()),
            Err(CalibrationError::NoSpeech)
        );
        assert_eq!(
            calibrate(&silent[..1000], &ChainSettings::raw()),
            Err(CalibrationError::TooShort)
        );
    }
}
