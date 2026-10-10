//! Trapezoidal (TPT) state-variable filter after A. Simper, "Linear Trapezoidal Integrated SVF".
//!
//! Responses equal the RBJ Audio EQ Cookbook prototypes under the bilinear transform with
//! prewarping at the centre frequency. Coefficients are ramped, which is stable for this form.

use crate::{SAMPLE_RATE_HZ, util::Ramp};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    Bell,
    LowShelf,
    HighShelf,
    LowCut,
    HighCut,
    Notch,
    BandPass,
}

/// `[g, k, m0, m1, m2]`: output = m0·input + m1·band + m2·low.
pub type Coefficients = [f32; 5];

pub fn coefficients(kind: FilterKind, freq_hz: f32, q: f32, gain_db: f32) -> Coefficients {
    let freq_hz = freq_hz.clamp(1.0, SAMPLE_RATE_HZ as f32 * 0.49);
    let g = (PI * freq_hz / SAMPLE_RATE_HZ as f32).tan();
    let k = 1.0 / q;
    let a = 10f32.powf(gain_db / 40.0);
    match kind {
        FilterKind::Bell => {
            let k = 1.0 / (q * a);
            [g, k, 1.0, k * (a * a - 1.0), 0.0]
        }
        FilterKind::LowShelf => [g / a.sqrt(), k, 1.0, k * (a - 1.0), a * a - 1.0],
        FilterKind::HighShelf => [g * a.sqrt(), k, a * a, k * (1.0 - a) * a, 1.0 - a * a],
        FilterKind::LowCut => [g, k, 1.0, -k, -1.0],
        FilterKind::HighCut => [g, k, 0.0, 0.0, 1.0],
        FilterKind::Notch => [g, k, 1.0, -k, 0.0],
        // Constant 0 dB peak gain.
        FilterKind::BandPass => [g, k, 0.0, k, 0.0],
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Svf {
    coefficients: Ramp<5>,
    a: [f32; 3],
    ic1eq: f32,
    ic2eq: f32,
}

impl Svf {
    pub fn new(coefficients: Coefficients) -> Self {
        Self {
            coefficients: Ramp::new(coefficients),
            a: derived(coefficients),
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    pub fn set(&mut self, coefficients: Coefficients) {
        self.coefficients.set_target(coefficients);
    }

    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    #[inline]
    fn tick_taps(&mut self, v0: f32) -> (f32, f32, f32) {
        if self.coefficients.is_ramping() {
            self.coefficients.next();
            self.a = derived(self.coefficients.value());
        }
        let [a1, a2, a3] = self.a;
        let v3 = v0 - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        (v0, v1, v2)
    }

    #[inline]
    pub fn tick(&mut self, input: f32) -> f32 {
        let (v0, v1, v2) = self.tick_taps(input);
        let [_, _, m0, m1, m2] = self.coefficients.value();
        m0 * v0 + m1 * v1 + m2 * v2
    }
}

fn derived([g, k, ..]: Coefficients) -> [f32; 3] {
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    [a1, a2, g * a2]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Steady-state gain of `process` at `freq_hz` in dB, measured on a sine.
    pub fn measured_gain_db(freq_hz: f32, mut process: impl FnMut(f32) -> f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * freq_hz as f64 / SAMPLE_RATE_HZ as f64;
        let (mut input_power, mut output_power) = (0.0f64, 0.0f64);
        for n in 0..96_000 {
            let x = (w * n as f64).sin() as f32 * 0.25;
            let y = process(x);
            if n >= 48_000 {
                input_power += (x as f64).powi(2);
                output_power += (y as f64).powi(2);
            }
        }
        (10.0 * (output_power / input_power).log10()) as f32
    }

    /// RBJ analog prototype magnitude in dB at the prewarped frequency.
    fn analog_gain_db(kind: FilterKind, freq_hz: f32, centre_hz: f32, q: f32, gain_db: f32) -> f32 {
        let fs = SAMPLE_RATE_HZ as f64;
        let warp = |f: f32| (std::f64::consts::PI * f as f64 / fs).tan();
        let w = warp(freq_hz) / warp(centre_hz);
        let (q, a) = (q as f64, 10f64.powf(gain_db as f64 / 40.0));
        let s = |re: f64, im: f64| (re, im);
        let div = |(ar, ai): (f64, f64), (br, bi): (f64, f64)| {
            ((ar * ar + ai * ai) / (br * br + bi * bi)).sqrt()
        };
        // H(s) with s = jw; numerator and denominator as (real, imaginary).
        let (num, den) = match kind {
            FilterKind::Bell => (s(1.0 - w * w, w * a / q), s(1.0 - w * w, w / (a * q))),
            FilterKind::LowShelf => (
                s(a * (a - w * w), a * a.sqrt() * w / q),
                s(1.0 - a * w * w, a.sqrt() * w / q),
            ),
            FilterKind::HighShelf => (
                s(a * (1.0 - a * w * w), a * a.sqrt() * w / q),
                s(a - w * w, a.sqrt() * w / q),
            ),
            FilterKind::LowCut => (s(-w * w, 0.0), s(1.0 - w * w, w / q)),
            FilterKind::HighCut => (s(1.0, 0.0), s(1.0 - w * w, w / q)),
            FilterKind::Notch => (s(1.0 - w * w, 0.0), s(1.0 - w * w, w / q)),
            FilterKind::BandPass => (s(0.0, w / q), s(1.0 - w * w, w / q)),
        };
        (20.0 * div(num, den).log10()) as f32
    }

    #[test]
    fn responses_match_rbj_prototypes_within_a_tenth_of_a_db() {
        let kinds = [
            (FilterKind::Bell, 1000.0, 1.4, 9.0),
            (FilterKind::Bell, 4000.0, 0.7, -12.0),
            (FilterKind::LowShelf, 120.0, 0.707, 6.0),
            (FilterKind::HighShelf, 10_000.0, 0.707, -6.0),
            (FilterKind::LowCut, 80.0, 0.707, 0.0),
            (FilterKind::HighCut, 8000.0, 0.707, 0.0),
            (FilterKind::Notch, 1000.0, 2.0, 0.0),
            (FilterKind::BandPass, 6500.0, 1.0, 0.0),
        ];
        for (kind, centre, q, gain) in kinds {
            for freq in [50.0f32, 150.0, 700.0, 1100.0, 3000.0, 9000.0] {
                // Notch depth is too sensitive to measure near its centre.
                if kind == FilterKind::Notch && (freq - centre).abs() < 300.0 {
                    continue;
                }
                let mut svf = Svf::new(coefficients(kind, centre, q, gain));
                let measured = measured_gain_db(freq, |x| svf.tick(x));
                let expected = analog_gain_db(kind, freq, centre, q, gain);
                assert!(
                    (measured - expected).abs() < 0.1,
                    "{kind:?} {centre} Hz at {freq} Hz: {measured} dB vs {expected} dB"
                );
            }
        }
    }
}
