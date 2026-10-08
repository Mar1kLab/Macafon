//! Parametric EQ (PRD §6.3.5): up to ten minimum-phase SVF bands plus output gain.

use crate::params::EQ_MAX_BANDS;
use crate::settings::{EqBand, EqFilter, EqParams};
use crate::svf::{Coefficients, FilterKind, Svf, coefficients};
use crate::util::SmoothedGain;

pub struct Equalizer {
    bands: [Svf; EQ_MAX_BANDS],
    band_count: usize,
    output: SmoothedGain,
}

impl Equalizer {
    pub fn new(params: &EqParams) -> Self {
        let mut bands = [Svf::new(identity()); EQ_MAX_BANDS];
        for (svf, band) in bands.iter_mut().zip(params.active_bands()) {
            *svf = Svf::new(band_coefficients(band));
        }
        Self {
            bands,
            band_count: params.active_bands().len(),
            output: SmoothedGain::new(params.output_gain_db),
        }
    }

    /// Removed bands ramp to unity before they stop being processed.
    pub fn set(&mut self, params: &EqParams) {
        let active = params.active_bands();
        for (index, svf) in self.bands.iter_mut().enumerate() {
            match active.get(index) {
                Some(band) => {
                    if index >= self.band_count {
                        svf.reset();
                    }
                    svf.set(band_coefficients(band));
                }
                None => svf.set(identity()),
            }
        }
        self.band_count = self.band_count.max(active.len());
        self.output.set_db(params.output_gain_db);
    }

    pub fn reset(&mut self) {
        self.bands.iter_mut().for_each(Svf::reset);
    }

    pub fn process(&mut self, io: &mut [f32]) {
        let bands = &mut self.bands[..self.band_count];
        for sample in io.iter_mut() {
            *sample = bands.iter_mut().fold(*sample, |x, band| band.tick(x));
        }
        self.output.process(io);
    }
}

fn identity() -> Coefficients {
    coefficients(FilterKind::Bell, 1000.0, 1.0, 0.0)
}

fn band_coefficients(band: &EqBand) -> Coefficients {
    let kind = match band.filter {
        EqFilter::Bell => FilterKind::Bell,
        EqFilter::LowShelf => FilterKind::LowShelf,
        EqFilter::HighShelf => FilterKind::HighShelf,
        EqFilter::LowCut => FilterKind::LowCut,
        EqFilter::HighCut => FilterKind::HighCut,
        EqFilter::Notch => FilterKind::Notch,
    };
    coefficients(kind, band.freq_hz, band.q, band.gain_db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EqCharacter;
    use crate::svf::tests::measured_gain_db;

    #[test]
    fn flat_eq_is_transparent_and_amount_scales_the_character() {
        let mut flat = Equalizer::new(&EqParams::from_character(EqCharacter::Presence, 0.0));
        let mut block = [0.0f32; 1];
        for freq in [100.0, 1000.0, 10_000.0] {
            let gain = measured_gain_db(freq, |x| {
                block[0] = x;
                flat.process(&mut block);
                block[0]
            });
            assert!(gain.abs() < 0.01, "{freq} Hz: {gain} dB");
        }
        // Presence boosts the 4 kHz bell by 4 dB at 100%; neighbours contribute a little.
        for (amount, expected) in [(100.0, 4.0), (50.0, 2.0)] {
            let mut eq = Equalizer::new(&EqParams::from_character(EqCharacter::Presence, amount));
            let gain = measured_gain_db(4000.0, |x| {
                block[0] = x;
                eq.process(&mut block);
                block[0]
            });
            assert!((gain - expected).abs() < 0.6, "{amount}%: {gain} dB");
        }
    }
}
