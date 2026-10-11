//! Mono BS.1770 K-weighting and an ungated three-second short-term window at 48 kHz.
use crate::{SAMPLE_RATE_HZ, sanitize};

const WINDOW_SAMPLES: usize = 3 * SAMPLE_RATE_HZ as usize;

struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    state: [f64; 2],
}
impl Biquad {
    fn tick(&mut self, input: f64) -> f64 {
        let output = self.b[0] * input + self.state[0];
        self.state[0] = self.b[1] * input - self.a[0] * output + self.state[1];
        self.state[1] = self.b[2] * input - self.a[1] * output;
        for state in &mut self.state {
            if state.abs() < 1e-30 {
                *state = 0.0;
            }
        }
        output
    }
}

pub(crate) struct ShortTermLoudness {
    shelf: Biquad,
    high_pass: Biquad,
    energies: Box<[f32]>,
    position: usize,
    count: usize,
    sum: f64,
}
impl ShortTermLoudness {
    pub fn new() -> Self {
        Self {
            // Reference coefficients of ITU-R BS.1770-5, Tables 1 and 2, at 48 kHz.
            shelf: Biquad {
                b: [1.53512485958697, -2.69169618940638, 1.19839281085285],
                a: [-1.69065929318241, 0.73248077421585],
                state: [0.0; 2],
            },
            high_pass: Biquad {
                b: [1.0, -2.0, 1.0],
                a: [-1.99004745483398, 0.99007225036621],
                state: [0.0; 2],
            },
            energies: vec![0.0; WINDOW_SAMPLES].into_boxed_slice(),
            position: 0,
            count: 0,
            sum: 0.0,
        }
    }
    pub fn measure(&mut self, block: &[f32]) -> f32 {
        for &sample in block {
            let weighted = self.high_pass.tick(self.shelf.tick(f64::from(sample)));
            let energy = sanitize((weighted * weighted) as f32);
            self.sum += f64::from(energy) - f64::from(self.energies[self.position]);
            self.energies[self.position] = energy;
            self.position += 1;
            if self.position == WINDOW_SAMPLES {
                self.position = 0;
            }
            self.count = (self.count + 1).min(WINDOW_SAMPLES);
        }
        if self.count == 0 {
            0.0
        } else {
            (self.sum.max(0.0) / self.count as f64) as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    #[test]
    fn k_weighted_one_khz_reference_and_three_second_silence_window() {
        let mut meter = ShortTermLoudness::new();
        let mut block = [0.0; 480];
        let mut energy = 0.0;
        for frame in 0..400 {
            for (offset, sample) in block.iter_mut().enumerate() {
                *sample = 0.1
                    * (std::f32::consts::TAU * 1000.0 * (frame * 480 + offset) as f32 / 48000.0)
                        .sin();
            }
            assert_no_alloc(|| energy = meter.measure(&block));
        }
        let lufs = -0.691 + 10.0 * energy.log10();
        assert!((lufs + 23.05).abs() < 0.1, "{lufs}");
        // The window must forget all energy, not behave as an exponential approximation.
        for _ in 0..310 {
            assert_no_alloc(|| energy = meter.measure(&[0.0; 480]));
        }
        assert!(energy < 1e-10, "{energy}");
    }
}
