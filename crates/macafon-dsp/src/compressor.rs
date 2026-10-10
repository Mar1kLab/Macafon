//! Feed-forward compressor without look-ahead (PRD §6.3.6): soft-knee gain computer,
//! peak or RMS detector, branching attack/release on the gain reduction.

use crate::settings::{CompressorDetector, CompressorParams, SPEECH_REFERENCE_DBFS};
use crate::util::{
    Ramp, SNAP_DB, SNAP_POWER, db_to_lin, follow, lin_to_db, power_to_db, time_coefficient,
};

const RMS_WINDOW_MS: f32 = 10.0;

pub struct Compressor {
    params: CompressorParams,
    rms: f32,
    mean_square: f32,
    attack: f32,
    release: f32,
    envelope_db: f32,
    makeup: Ramp<1>,
    block_reduction_db: f32,
}

/// Static output level for an input level (Giannoulis, Massberg, Reiss 2012).
pub fn static_curve_db(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let over = level_db - threshold_db;
    if 2.0 * over < -knee_db {
        level_db
    } else if 2.0 * over.abs() <= knee_db && knee_db > 0.0 {
        let x = over + knee_db * 0.5;
        level_db + (1.0 / ratio - 1.0) * x * x / (2.0 * knee_db)
    } else {
        threshold_db + over / ratio
    }
}

fn makeup_db(params: &CompressorParams) -> f32 {
    match params.makeup_db {
        Some(gain_db) => gain_db,
        None => {
            SPEECH_REFERENCE_DBFS
                - static_curve_db(
                    SPEECH_REFERENCE_DBFS,
                    params.threshold_db,
                    params.ratio,
                    params.knee_db,
                )
        }
    }
}

impl Compressor {
    pub fn new(params: &CompressorParams) -> Self {
        let mut compressor = Self {
            params: *params,
            rms: time_coefficient(RMS_WINDOW_MS),
            mean_square: 0.0,
            attack: 0.0,
            release: 0.0,
            envelope_db: 0.0,
            makeup: Ramp::new([db_to_lin(makeup_db(params))]),
            block_reduction_db: 0.0,
        };
        compressor.set(params);
        compressor
    }

    pub fn set(&mut self, params: &CompressorParams) {
        self.params = *params;
        self.attack = time_coefficient(params.attack_ms);
        self.release = time_coefficient(params.release_ms);
        self.makeup.set_target([db_to_lin(makeup_db(params))]);
    }

    pub fn reset(&mut self) {
        self.mean_square = 0.0;
        self.envelope_db = 0.0;
    }

    pub fn reduction_db(&self) -> f32 {
        self.block_reduction_db
    }

    pub fn process(&mut self, io: &mut [f32]) {
        let CompressorParams {
            threshold_db,
            ratio,
            knee_db,
            detector,
            ..
        } = self.params;
        let mut deepest = 0.0f32;
        for sample in io {
            let level_db = match detector {
                CompressorDetector::Peak => lin_to_db(sample.abs()),
                CompressorDetector::Rms => {
                    self.mean_square =
                        follow(self.mean_square, *sample * *sample, self.rms, SNAP_POWER);
                    power_to_db(self.mean_square)
                }
            };
            let target = static_curve_db(level_db, threshold_db, ratio, knee_db) - level_db;
            // Attack while the reduction deepens, release while it recovers.
            let coefficient = if target < self.envelope_db {
                self.attack
            } else {
                self.release
            };
            self.envelope_db = follow(self.envelope_db, target, coefficient, SNAP_DB);
            deepest = deepest.min(self.envelope_db);
            *sample *= db_to_lin(self.envelope_db) * self.makeup.next()[0];
        }
        self.block_reduction_db = deepest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_curve_is_continuous_soft_knee_with_ratio_above() {
        let (threshold, ratio, knee) = (-20.0, 4.0, 6.0);
        assert_eq!(static_curve_db(-40.0, threshold, ratio, knee), -40.0);
        assert_eq!(static_curve_db(-8.0, threshold, ratio, knee), -17.0);
        // Knee edges meet the linear segments.
        let below = static_curve_db(-23.0, threshold, ratio, knee);
        let above = static_curve_db(-17.0, threshold, ratio, knee);
        assert!((below + 23.0).abs() < 1e-4);
        assert!((above - (threshold + 3.0 / ratio)).abs() < 1e-4);
        // Hard knee.
        assert_eq!(static_curve_db(-10.0, threshold, ratio, 0.0), -17.5);
    }

    #[test]
    fn steady_tone_settles_to_static_reduction_and_auto_makeup_restores_reference() {
        let params = CompressorParams {
            threshold_db: -30.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector: CompressorDetector::Peak,
            makeup_db: Some(0.0),
            ..CompressorParams::default()
        };
        let mut compressor = Compressor::new(&params);
        for _ in 0..2 {
            compressor.process(&mut [0.1f32; 4800]); // −20 dBFS DC
        }
        // −20 dBFS is 10 dB over: 4:1 leaves 2.5 dB, so the reduction is −7.5 dB.
        assert!((compressor.envelope_db - -7.5).abs() < 0.05);

        let auto = Compressor::new(&CompressorParams {
            makeup_db: None,
            ..params
        });
        // Reference −18 dBFS is 12 dB over: −9 dB reduction restored by +9 dB makeup.
        assert!((lin_to_db(auto.makeup.value()[0]) - 9.0).abs() < 1e-3);
    }
}
