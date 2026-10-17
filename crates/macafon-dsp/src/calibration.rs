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

/// Raw speech must leave room for coughs, laughter and other transients before digital Gain.
pub const RAW_SPEECH_PEAK_DBFS: f32 = -15.0;

/// One downward endpoint-volume adjustment, on the device's dB grid. Never raises a level.
/// Limit each adjustment to roughly 6 dB so the caller can check fresh capture between steps.
pub fn input_level_target_db(
    current_db: f32,
    minimum_db: f32,
    increment_db: f32,
    raw_peak_dbfs: f32,
) -> Option<f32> {
    if ![current_db, minimum_db, increment_db, raw_peak_dbfs]
        .iter()
        .all(|value| value.is_finite())
        || increment_db <= 0.0
        || current_db <= minimum_db
        || raw_peak_dbfs <= RAW_SPEECH_PEAK_DBFS
    {
        return None;
    }
    let reduction_db = (raw_peak_dbfs - RAW_SPEECH_PEAK_DBFS).min(6.0);
    let target_db = minimum_db
        + ((current_db - reduction_db - minimum_db) / increment_db).floor() * increment_db;
    Some(target_db.max(minimum_db))
}

/// Analysis windows of 20 ms.
const WINDOW_SAMPLES: usize = SAMPLE_RATE_HZ as usize / 50;
/// The loudest fifth of the silence (a cough, a click) does not count towards the floor.
const FLOOR_QUANTILE: f32 = 0.8;
/// Windows this far above the floor count as speech.
const SPEECH_MARGIN_DB: f32 = 10.0;
/// At least this much detected speech is needed for a meaningful level.
const MIN_SPEECH_WINDOWS: usize = 30;
/// Samples at or above this magnitude count as clipped (−0.1 dBFS).
const CLIP_LEVEL: f32 = 0.989;
const CLIPPED_SAMPLES: usize = 3;
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
    /// Same recording through the calibrated chain.
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
    if recording.iter().filter(|x| x.abs() >= CLIP_LEVEL).count() >= CLIPPED_SAMPLES {
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

    let gain_db = ((SPEECH_REFERENCE_DBFS - speech_rms_dbfs) / GAIN_DB.step).round() * GAIN_DB.step;
    let gain_db = gain_db.clamp(GAIN_DB.min, GAIN_DB.max);
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
    let expander_threshold_db = (noise_floor_dbfs + gain_db + EXPANDER_FLOOR_MARGIN_DB)
        .min(speech_rms_dbfs + gain_db - EXPANDER_SPEECH_MARGIN_DB)
        .clamp(EXPANDER_THRESHOLD_DBFS.min, EXPANDER_THRESHOLD_DBFS.max)
        .round();

    let mut settings = *current;
    settings.gain.enabled = true;
    settings.gain.params.gain_db = gain_db;
    settings.high_pass.enabled = true;
    settings.high_pass.params.freq_hz = high_pass_freq_hz;
    settings.noise_suppression.enabled = true;
    settings.noise_suppression.params.intensity_pct = noise_intensity_pct;
    settings.expander.enabled = true;
    settings.expander.params.threshold_db = expander_threshold_db;

    let snr_after_db = processed_snr_db(recording, &settings, &speech_windows);
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

/// Indices of windows louder than `threshold_dbfs`.
fn speech_windows(speech: &[f32], threshold_dbfs: f32) -> Vec<usize> {
    window_powers(speech)
        .enumerate()
        .filter(|(_, power)| power_to_db(*power) > threshold_dbfs)
        .map(|(index, _)| index)
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

/// SNR of the recording after the calibrated chain, on the same speech windows.
fn processed_snr_db(recording: &[f32], settings: &ChainSettings, speech_windows: &[usize]) -> f32 {
    let settings = ChainSettings {
        muted: false,
        bypass: false,
        ..*settings
    };
    let Ok(mut processor) = MicrophoneProcessor::new(&settings) else {
        return f32::NAN;
    };
    let latency = processor.latency_samples() as usize;
    let mut output = recording.to_vec();
    output.extend(std::iter::repeat_n(0.0, latency));
    output
        .chunks_mut(crate::FRAME_SAMPLES)
        .for_each(|block| processor.process(block));
    let output = &output[latency..];
    let (silence, speech) = output.split_at(SILENCE_SAMPLES);
    level_dbfs(speech, speech_windows) - floor_dbfs(&silence[SETTLE_SAMPLES..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Preset;
    use crate::util::db_to_lin;

    #[test]
    fn endpoint_steps_leave_fifteen_db_raw_headroom_without_raising_the_level() {
        let mut endpoint_db = -3.0;
        let mut raw_peak_dbfs = -1.0;
        assert_no_alloc::assert_no_alloc(|| {
            for expected_db in [-9.0, -15.0, -18.0] {
                let next_db = input_level_target_db(endpoint_db, -60.0, 1.5, raw_peak_dbfs)
                    .expect("hot speech needs attenuation");
                assert_eq!(next_db, expected_db);
                raw_peak_dbfs += next_db - endpoint_db;
                endpoint_db = next_db;
            }
            assert!(raw_peak_dbfs <= -15.0);
            assert_eq!(
                input_level_target_db(endpoint_db, -60.0, 1.5, raw_peak_dbfs),
                None
            );
            assert_eq!(input_level_target_db(-60.0, -60.0, 1.5, -3.0), None);
            assert_eq!(input_level_target_db(-3.0, -60.0, 0.0, -3.0), None);
            assert_eq!(input_level_target_db(-3.0, -60.0, 1.5, f32::NAN), None);
            // Make-up remains digital: a 15 dB reduction at the endpoint requires 15 dB
            // more Gain to keep the existing speech RMS target.
            let original_speech_rms_dbfs = -12.0;
            let adjusted_speech_rms_dbfs = original_speech_rms_dbfs + endpoint_db + 3.0;
            let gain_db = SPEECH_REFERENCE_DBFS - adjusted_speech_rms_dbfs;
            assert_eq!(gain_db, 9.0);
            assert_eq!(adjusted_speech_rms_dbfs + gain_db, -18.0);
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
        // Floor + gain + 6 ≈ −47 dBFS, under speech − 12 = −30.
        assert!(
            (report.expander_threshold_db + 47.0).abs() <= 1.0,
            "{report:?}"
        );
        // SNR 35 dB → between 40% (40 dB) and 90% (20 dB).
        assert!((report.noise_intensity_pct - 52.0).abs() <= 2.0);
        assert!(!report.low_frequency_noise && !report.suggest_studio);
        assert_eq!(report.high_pass_freq_hz, 80.0);
        assert!(report.snr_after_db > report.snr_db + 6.0, "{report:?}");
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
        // The floor rule would put the threshold above the speech; capped at speech − 12.
        assert_eq!(report.expander_threshold_db, -30.0);
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
