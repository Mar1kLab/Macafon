//! Downward expander / gate (PRD §6.3.4): 5 ms RMS detector on a 100 Hz high-passed sidechain,
//! hysteresis and hold, attack/release smoothing of the gain in dB.

use crate::settings::ExpanderParams;
use crate::svf::{FilterKind, Svf, coefficients};
use crate::util::{
    SNAP_DB, SNAP_POWER, db_to_lin, follow, ms_to_samples, power_to_db, time_coefficient,
};

const DETECTOR_MS: f32 = 5.0;
const SIDECHAIN_HPF_HZ: f32 = 100.0;

pub struct Expander {
    params: ExpanderParams,
    sidechain: Svf,
    detector: f32,
    mean_square: f32,
    attack: f32,
    release: f32,
    hold_samples: u32,
    hold_left: u32,
    open: bool,
    gain_db: f32,
    /// Deepest reduction in the last block, dB ≤ 0.
    block_reduction_db: f32,
}

impl Expander {
    pub fn new(params: &ExpanderParams) -> Self {
        let mut expander = Self {
            params: *params,
            sidechain: Svf::new(coefficients(
                FilterKind::LowCut,
                SIDECHAIN_HPF_HZ,
                std::f32::consts::FRAC_1_SQRT_2,
                0.0,
            )),
            detector: time_coefficient(DETECTOR_MS),
            mean_square: 0.0,
            attack: 0.0,
            release: 0.0,
            hold_samples: 0,
            hold_left: 0,
            open: false,
            gain_db: 0.0,
            block_reduction_db: 0.0,
        };
        expander.set(params);
        expander
    }

    /// Thresholds and times are followed by the gain smoothing, so changes cannot click.
    pub fn set(&mut self, params: &ExpanderParams) {
        self.params = *params;
        self.attack = time_coefficient(params.attack_ms);
        self.release = time_coefficient(params.release_ms);
        self.hold_samples = ms_to_samples(params.hold_ms);
    }

    pub fn reset(&mut self) {
        self.sidechain.reset();
        self.mean_square = 0.0;
        self.hold_left = 0;
        self.open = false;
        self.gain_db = 0.0;
    }

    pub fn reduction_db(&self) -> f32 {
        self.block_reduction_db
    }

    /// Static gain for a detector level, dB ≤ 0; also advances the hysteresis/hold state.
    fn target_gain_db(&mut self, level_db: f32) -> f32 {
        let ExpanderParams {
            threshold_db,
            ratio,
            range_db,
            hysteresis_db,
            ..
        } = self.params;
        if level_db >= threshold_db {
            self.open = true;
            self.hold_left = self.hold_samples;
        } else if self.open && level_db < threshold_db - hysteresis_db {
            if self.hold_left > 0 {
                self.hold_left -= 1;
            } else {
                self.open = false;
            }
        }
        if self.open {
            0.0
        } else if ratio == f32::INFINITY {
            range_db
        } else {
            ((level_db - threshold_db) * (ratio - 1.0)).clamp(range_db, 0.0)
        }
    }

    pub fn process(&mut self, io: &mut [f32]) {
        let mut deepest = 0.0f32;
        for sample in io {
            let side = self.sidechain.tick(*sample);
            self.mean_square = follow(self.mean_square, side * side, self.detector, SNAP_POWER);
            let target = self.target_gain_db(power_to_db(self.mean_square));
            let coefficient = if target > self.gain_db {
                self.attack
            } else {
                self.release
            };
            self.gain_db = follow(self.gain_db, target, coefficient, SNAP_DB);
            deepest = deepest.min(self.gain_db);
            *sample *= db_to_lin(self.gain_db);
        }
        self.block_reduction_db = deepest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled_gain_db(params: &ExpanderParams, level_dbfs: f32) -> f32 {
        let mut expander = Expander::new(params);
        let amplitude = 10f32.powf(level_dbfs / 20.0) * std::f32::consts::SQRT_2;
        let mut block = [0.0; 480];
        let mut phase = 0usize;
        for _ in 0..200 {
            for sample in &mut block {
                *sample = amplitude * (phase as f32 * 2.0 * std::f32::consts::PI / 48.0).sin();
                phase += 1;
            }
            expander.process(&mut block);
        }
        expander.gain_db
    }

    #[test]
    fn static_curve_follows_ratio_and_range_below_threshold() {
        let params = ExpanderParams {
            threshold_db: -40.0,
            ratio: 3.0,
            range_db: -30.0,
            hysteresis_db: 0.0,
            ..ExpanderParams::default()
        };
        // 1 kHz sines: RMS level equals the configured dBFS value after the sidechain HPF.
        for (level, expected) in [(-30.0, 0.0), (-45.0, -10.0), (-50.0, -20.0), (-70.0, -30.0)] {
            let gain = settled_gain_db(&params, level);
            assert!((gain - expected).abs() < 0.2, "{level} dBFS: {gain} dB");
        }
        let gate = ExpanderParams {
            ratio: f32::INFINITY,
            ..params
        };
        assert!((settled_gain_db(&gate, -41.0) + 30.0).abs() < 0.2);
    }

    #[test]
    fn hysteresis_keeps_the_open_expander_open_inside_the_band() {
        let params = ExpanderParams {
            threshold_db: -40.0,
            hysteresis_db: 6.0,
            ..ExpanderParams::default()
        };
        let mut expander = Expander::new(&params);
        assert_eq!(expander.target_gain_db(-35.0), 0.0);
        for _ in 0..48_000 {
            assert_eq!(expander.target_gain_db(-44.0), 0.0);
        }
        let closed = (0..=expander.hold_samples)
            .map(|_| expander.target_gain_db(-47.0))
            .last();
        assert_eq!(closed, Some(-14.0));
    }
}
