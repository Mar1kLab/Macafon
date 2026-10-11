//! Peak limiter (PRD §6.3.8): fast attack plus a soft clipper over the last 0.5 dB, so the
//! default needs no look-ahead. Optional 1/2/5 ms look-ahead delays the signal instead.

use crate::SAMPLE_RATE_HZ;
use crate::settings::LimiterParams;
use crate::util::{db_to_lin, lin_to_db, time_coefficient};

const SOFT_CLIP_DB: f32 = 0.5;
const ZERO_LOOKAHEAD_ATTACK_MS: f32 = 0.2;
const MAX_LOOKAHEAD_SAMPLES: usize = 5 * SAMPLE_RATE_HZ as usize / 1000;

pub struct Limiter {
    ceiling: f32,
    knee: f32,
    attack: f32,
    release: f32,
    gain: f32,
    delay: [f32; MAX_LOOKAHEAD_SAMPLES],
    delay_right: [f32; MAX_LOOKAHEAD_SAMPLES],
    delay_len: usize,
    position: usize,
    block_reduction_db: f32,
}

impl Limiter {
    pub fn new(params: &LimiterParams) -> Self {
        let delay_len = params.lookahead_ms as usize * SAMPLE_RATE_HZ as usize / 1000;
        let attack_ms = if delay_len == 0 {
            ZERO_LOOKAHEAD_ATTACK_MS
        } else {
            // Within the look-ahead window the envelope covers >99% of the step.
            params.lookahead_ms as f32 / 5.0
        };
        let mut limiter = Self {
            ceiling: 1.0,
            knee: 1.0,
            attack: time_coefficient(attack_ms),
            release: 0.0,
            gain: 1.0,
            delay: [0.0; MAX_LOOKAHEAD_SAMPLES],
            delay_right: [0.0; MAX_LOOKAHEAD_SAMPLES],
            delay_len: delay_len.min(MAX_LOOKAHEAD_SAMPLES),
            position: 0,
            block_reduction_db: 0.0,
        };
        limiter.set(params);
        limiter
    }

    /// Look-ahead changes the reported latency and therefore requires a new chain.
    pub fn set(&mut self, params: &LimiterParams) {
        self.ceiling = db_to_lin(params.ceiling_db);
        self.knee = db_to_lin(params.ceiling_db - SOFT_CLIP_DB);
        self.release = time_coefficient(params.release_ms);
    }

    pub fn reset(&mut self) {
        self.gain = 1.0;
        self.delay.fill(0.0);
        self.delay_right.fill(0.0);
    }

    pub fn latency_samples(&self) -> u32 {
        self.delay_len as u32
    }

    pub fn reduction_db(&self) -> f32 {
        self.block_reduction_db
    }

    pub fn process(&mut self, io: &mut [f32]) {
        let mut lowest = 1.0f32;
        for sample in io {
            let gain = self.gain_for(sample.abs());
            lowest = lowest.min(gain);
            let delayed = Self::delay(&mut self.delay, self.position, self.delay_len, *sample);
            self.advance();
            *sample = self.clip(delayed * gain);
        }
        self.block_reduction_db = lin_to_db(lowest);
    }

    /// Stereo-linked (CFX-004): the louder channel sets one gain for both.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut lowest = 1.0f32;
        for (l, r) in left.iter_mut().zip(right) {
            let gain = self.gain_for(l.abs().max(r.abs()));
            lowest = lowest.min(gain);
            let delayed_l = Self::delay(&mut self.delay, self.position, self.delay_len, *l);
            let delayed_r = Self::delay(&mut self.delay_right, self.position, self.delay_len, *r);
            self.advance();
            *l = self.clip(delayed_l * gain);
            *r = self.clip(delayed_r * gain);
        }
        self.block_reduction_db = lin_to_db(lowest);
    }

    #[inline]
    fn gain_for(&mut self, peak: f32) -> f32 {
        let target = if peak > self.knee {
            self.knee / peak
        } else {
            1.0
        };
        let coefficient = if target < self.gain {
            self.attack
        } else {
            self.release
        };
        self.gain += coefficient * (target - self.gain);
        self.gain
    }

    #[inline]
    fn delay(line: &mut [f32; MAX_LOOKAHEAD_SAMPLES], position: usize, len: usize, x: f32) -> f32 {
        if len == 0 {
            x
        } else {
            std::mem::replace(&mut line[position], x)
        }
    }

    #[inline]
    fn advance(&mut self) {
        if self.delay_len != 0 {
            self.position = (self.position + 1) % self.delay_len;
        }
    }

    /// Soft clipper over the last 0.5 dB below the ceiling.
    #[inline]
    fn clip(&self, y: f32) -> f32 {
        let magnitude = y.abs();
        if magnitude > self.knee {
            let span = self.ceiling - self.knee;
            (self.knee + span * ((magnitude - self.knee) / span).tanh()).copysign(y)
        } else {
            y
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_never_exceeds_ceiling_and_quiet_audio_is_untouched() {
        for lookahead_ms in [0, 1, 2, 5] {
            let mut limiter = Limiter::new(&LimiterParams {
                ceiling_db: -1.0,
                lookahead_ms,
                ..LimiterParams::default()
            });
            let ceiling = db_to_lin(-1.0);
            let mut block: Vec<f32> = (0..4800)
                .map(|n| match n {
                    _ if n % 700 == 0 => 4.0,
                    0..2400 => (n as f32 * 0.05).sin() * 0.9,
                    _ => (n as f32 * 0.05).sin() * 4.0,
                })
                .collect();
            limiter.process(&mut block);
            assert!(
                block.iter().all(|y| y.abs() <= ceiling),
                "{lookahead_ms} ms"
            );
            assert!(limiter.reduction_db() < -10.0);
            assert_eq!(limiter.latency_samples(), lookahead_ms as u32 * 48);
        }
        let mut stereo = Limiter::new(&LimiterParams::ceiling(-3.0));
        let mut left: Vec<f32> = (0..4800).map(|n| (n as f32 * 0.05).sin() * 2.0).collect();
        let mut right = vec![0.1f32; 4800];
        stereo.process_stereo(&mut left, &mut right);
        let ceiling = db_to_lin(-3.0);
        assert!(left.iter().chain(&right).all(|y| y.abs() <= ceiling));
        // The quiet right channel follows the left's gain reduction.
        assert!(right[4799] < 0.1 * 0.9);
        let mut limiter = Limiter::new(&LimiterParams::default());
        let quiet: Vec<f32> = (0..4800).map(|n| (n as f32 * 0.05).sin() * 0.5).collect();
        let mut block = quiet.clone();
        limiter.process(&mut block);
        assert_eq!(block, quiet);
    }
}
