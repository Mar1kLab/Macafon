//! De-esser without look-ahead (PRD §6.3.7): a band-pass sidechain detector driving either a
//! dynamic bell at the same band (split-band) or the whole signal (wideband).

use crate::settings::{DeEsserMode, DeEsserParams};
use crate::svf::{FilterKind, Svf, coefficients};
use crate::util::{Ramp, SNAP_AMPLITUDE, db_to_lin, follow, lin_to_db, time_coefficient};

const SIDECHAIN_Q: f32 = 1.0;
const ATTACK_MS: f32 = 0.5;
const RELEASE_MS: f32 = 40.0;
const RATIO: f32 = 4.0;

pub struct DeEsser {
    params: DeEsserParams,
    sidechain: Svf,
    envelope: f32,
    attack: f32,
    release: f32,
    listen: Ramp<1>,
    block_reduction_db: f32,
}

impl DeEsser {
    pub fn new(params: &DeEsserParams) -> Self {
        Self {
            params: *params,
            sidechain: Svf::new(coefficients(
                FilterKind::BandPass,
                params.freq_hz,
                SIDECHAIN_Q,
                0.0,
            )),
            envelope: 0.0,
            attack: time_coefficient(ATTACK_MS),
            release: time_coefficient(RELEASE_MS),
            listen: Ramp::new([if params.listen { 1.0 } else { 0.0 }]),
            block_reduction_db: 0.0,
        }
    }

    pub fn set(&mut self, params: &DeEsserParams) {
        self.params = *params;
        self.sidechain.set(coefficients(
            FilterKind::BandPass,
            params.freq_hz,
            SIDECHAIN_Q,
            0.0,
        ));
        self.listen
            .set_target([if params.listen { 1.0 } else { 0.0 }]);
    }

    pub fn reset(&mut self) {
        self.sidechain.reset();
        self.envelope = 0.0;
    }

    pub fn reduction_db(&self) -> f32 {
        self.block_reduction_db
    }

    pub fn process(&mut self, io: &mut [f32]) {
        let DeEsserParams {
            threshold_db,
            max_reduction_db,
            mode,
            ..
        } = self.params;
        let mut deepest = 0.0f32;
        for sample in io {
            let band = self.sidechain.tick(*sample);
            let rectified = band.abs();
            let coefficient = if rectified > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = follow(self.envelope, rectified, coefficient, SNAP_AMPLITUDE);
            let over_db = lin_to_db(self.envelope) - threshold_db;
            let reduction_db = -(over_db * (1.0 - 1.0 / RATIO)).clamp(0.0, max_reduction_db);
            deepest = deepest.min(reduction_db);
            let gain = db_to_lin(reduction_db);
            let processed = match mode {
                // Unit-peak band-pass: exactly `gain` at the centre, unity far from it.
                DeEsserMode::SplitBand => *sample + (gain - 1.0) * band,
                DeEsserMode::Wideband => *sample * gain,
            };
            let [listen] = self.listen.next();
            *sample = processed + listen * (band - processed);
        }
        self.block_reduction_db = deepest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svf::tests::measured_gain_db;

    #[test]
    fn split_band_reduces_loud_sibilance_but_not_the_voice_band() {
        let params = DeEsserParams {
            threshold_db: -40.0,
            max_reduction_db: 8.0,
            ..DeEsserParams::default()
        };
        let mut de_esser = DeEsser::new(&params);
        let mut block = [0.0f32; 1];
        let mut tick = |x: f32| {
            block[0] = x;
            de_esser.process(&mut block);
            block[0]
        };
        // The −15 dBFS test sine is far above threshold: full reduction at 6.5 kHz.
        let sibilance = measured_gain_db(6500.0, &mut tick);
        assert!((sibilance + 8.0).abs() < 1.0, "{sibilance} dB");
        // At 300 Hz the band-pass detector stays quiet and the voice passes untouched.
        let voice = measured_gain_db(300.0, &mut tick);
        assert!(voice.abs() < 0.3, "{voice} dB");
    }
}
