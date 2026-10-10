//! Butterworth high-pass as a cascade of TPT SVF sections (PRD §6.3.2), zero latency.

use crate::settings::HighPassParams;
use crate::svf::{Coefficients, FilterKind, Svf, coefficients};

const Q_12_DB_OCT: [f32; 1] = [std::f32::consts::FRAC_1_SQRT_2];
const Q_24_DB_OCT: [f32; 2] = [0.541_196_1, 1.306_563];

pub struct HighPass {
    sections: [Svf; 2],
    cascade: bool,
}

impl HighPass {
    pub fn new(params: &HighPassParams) -> Self {
        let cascade = params.slope_db_oct == 24;
        Self {
            sections: [0, 1].map(|index| Svf::new(section(params, cascade, index))),
            cascade,
        }
    }

    pub fn set(&mut self, params: &HighPassParams) {
        if (params.slope_db_oct == 24) != self.cascade {
            // A slope change alters the topology; the 10 ms coefficient ramp keeps it smooth.
            self.cascade = !self.cascade;
            self.sections[1].reset();
        }
        for (index, section_filter) in self.sections.iter_mut().enumerate() {
            section_filter.set(section(params, self.cascade, index));
        }
    }

    pub fn reset(&mut self) {
        self.sections.iter_mut().for_each(Svf::reset);
    }

    pub fn process(&mut self, io: &mut [f32]) {
        let [first, second] = &mut self.sections;
        if self.cascade {
            io.iter_mut()
                .for_each(|sample| *sample = second.tick(first.tick(*sample)));
        } else {
            io.iter_mut()
                .for_each(|sample| *sample = first.tick(*sample));
        }
    }
}

fn section(params: &HighPassParams, cascade: bool, index: usize) -> Coefficients {
    let q = if cascade {
        Q_24_DB_OCT[index]
    } else {
        Q_12_DB_OCT[0]
    };
    coefficients(FilterKind::LowCut, params.freq_hz, q, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svf::tests::measured_gain_db;

    #[test]
    fn butterworth_cutoff_is_minus_three_db_with_selected_slope() {
        for (slope, octave_below_db) in [(12, -12.3), (24, -24.1)] {
            let params = HighPassParams {
                freq_hz: 100.0,
                slope_db_oct: slope,
            };
            let mut filter = HighPass::new(&params);
            let mut tick = |x: f32| {
                let mut block = [x];
                filter.process(&mut block);
                block[0]
            };
            let cutoff = measured_gain_db(100.0, &mut tick);
            assert!(
                (cutoff + 3.01).abs() < 0.1,
                "{slope}: {cutoff} dB at cutoff"
            );
            let below = measured_gain_db(50.0, &mut tick);
            assert!(
                (below - octave_below_db).abs() < 0.3,
                "{slope}: {below} dB at 50 Hz"
            );
            let pass = measured_gain_db(2000.0, &mut tick);
            assert!(pass.abs() < 0.1, "{slope}: {pass} dB at 2 kHz");
        }
    }
}
