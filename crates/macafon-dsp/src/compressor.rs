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
    sustained_reduction_db: f32,
    auto_fast_release: f32,
    auto_slow_release: f32,
    auto_memory: f32,
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
            sustained_reduction_db: 0.0,
            auto_fast_release: time_coefficient(40.0),
            auto_slow_release: time_coefficient(400.0),
            auto_memory: time_coefficient(200.0),
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
        self.sustained_reduction_db = 0.0;
    }

    pub fn reduction_db(&self) -> f32 {
        self.block_reduction_db
    }

    pub fn process(&mut self, io: &mut [f32]) {
        self.block_reduction_db = 0.0;
        for sample in io {
            *sample *= self.step(sample.abs(), *sample * *sample);
        }
    }

    /// Stereo-linked (CFX-004): one detector on both channels, the same gain on each, so the
    /// stereo image does not shift.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.block_reduction_db = 0.0;
        for (l, r) in left.iter_mut().zip(right) {
            let gain = self.step(l.abs().max(r.abs()), 0.5 * (*l * *l + *r * *r));
            *l *= gain;
            *r *= gain;
        }
    }

    /// One sample of the gain computer: detector level in, linear gain (with makeup) out.
    #[inline]
    fn step(&mut self, peak: f32, power: f32) -> f32 {
        let CompressorParams {
            threshold_db,
            ratio,
            knee_db,
            detector,
            ..
        } = self.params;
        let level_db = match detector {
            CompressorDetector::Peak => lin_to_db(peak),
            CompressorDetector::Rms => {
                self.mean_square = follow(self.mean_square, power, self.rms, SNAP_POWER);
                power_to_db(self.mean_square)
            }
        };
        let target = static_curve_db(level_db, threshold_db, ratio, knee_db) - level_db;
        if self.params.auto_release {
            self.sustained_reduction_db = follow(
                self.sustained_reduction_db,
                -target,
                self.auto_memory,
                SNAP_DB,
            );
        }
        // Attack while the reduction deepens, release while it recovers.
        let coefficient = if target < self.envelope_db {
            self.attack
        } else if self.params.auto_release {
            let sustained =
                (self.sustained_reduction_db / (-self.envelope_db).max(0.1)).clamp(0.0, 1.0);
            self.auto_fast_release + sustained * (self.auto_slow_release - self.auto_fast_release)
        } else {
            self.release
        };
        self.envelope_db = follow(self.envelope_db, target, coefficient, SNAP_DB);
        self.block_reduction_db = self.block_reduction_db.min(self.envelope_db);
        db_to_lin(self.envelope_db) * self.makeup.next()[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_release_recovers_transients_faster_than_sustained_reduction() {
        use assert_no_alloc::assert_no_alloc;
        let params = CompressorParams {
            auto_release: true,
            attack_ms: 0.1,
            threshold_db: -30.0,
            knee_db: 0.0,
            detector: CompressorDetector::Peak,
            makeup_db: Some(0.0),
            ..CompressorParams::default()
        };
        let mut transient = Compressor::new(&params);
        let mut sustained = Compressor::new(&params);
        assert_no_alloc(|| {
            transient.process(&mut [0.5; 480]);
            sustained.process(&mut [0.5; 48000]);
            transient.process(&mut [0.0; 4800]);
            sustained.process(&mut [0.0; 4800]);
        });
        assert!(
            transient.envelope_db > sustained.envelope_db + 2.0,
            "transient {}, sustained {}",
            transient.envelope_db,
            sustained.envelope_db
        );
    }

    #[test]
    fn stereo_link_applies_one_gain_to_both_channels() {
        let params = CompressorParams {
            threshold_db: -30.0,
            ratio: 4.0,
            knee_db: 0.0,
            detector: CompressorDetector::Peak,
            makeup_db: Some(0.0),
            ..CompressorParams::default()
        };
        let mut linked = Compressor::new(&params);
        // Loud left, quiet right: a linked compressor turns both down by the same amount.
        let mut left = [0.5f32; 9600];
        let mut right = [0.05f32; 9600];
        assert_no_alloc::assert_no_alloc(|| linked.process_stereo(&mut left, &mut right));
        assert!((left[9599] / 0.5 - right[9599] / 0.05).abs() < 1e-6);
        assert!(lin_to_db(left[9599] / 0.5) < -15.0);
        assert!(linked.reduction_db() < -15.0);
    }

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
