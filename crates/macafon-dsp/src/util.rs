//! Small RT-safe building blocks shared by the modules.

use crate::SAMPLE_RATE_HZ;

/// Parameter changes are ramped over 10 ms (DSP-003).
pub const SMOOTHING_SAMPLES: u32 = SAMPLE_RATE_HZ / 100;

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

/// Level in dB; silence maps to a finite floor so dB arithmetic stays finite.
#[inline]
pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-6).log10()
}

#[inline]
pub fn power_to_db(power: f32) -> f32 {
    10.0 * power.max(1e-12).log10()
}

/// One-pole smoothing coefficient for a time constant in milliseconds.
pub fn time_coefficient(ms: f32) -> f32 {
    let samples = ms * 0.001 * SAMPLE_RATE_HZ as f32;
    if samples <= 1.0 {
        1.0
    } else {
        1.0 - (-1.0 / samples).exp()
    }
}

/// Snap distances for `follow`: far below audibility in each domain, far above denormals.
pub const SNAP_DB: f32 = 1e-6;
pub const SNAP_POWER: f32 = 1e-20;
pub const SNAP_AMPLITUDE: f32 = 1e-10;

/// One-pole step that lands exactly on the target instead of decaying into denormals.
#[inline]
pub fn follow(current: f32, target: f32, coefficient: f32, snap: f32) -> f32 {
    let next = current + coefficient * (target - current);
    if (next - target).abs() < snap {
        target
    } else {
        next
    }
}

pub fn ms_to_samples(ms: f32) -> u32 {
    (ms * 0.001 * SAMPLE_RATE_HZ as f32).round() as u32
}

/// Piecewise-linear interpolation through `(x, y)` points sorted by x.
pub fn interpolate(x: f32, points: &[(f32, f32)]) -> f32 {
    let mut previous = points[0];
    if x <= previous.0 {
        return previous.1;
    }
    for &point in &points[1..] {
        if x <= point.0 {
            let t = (x - previous.0) / (point.0 - previous.0);
            return previous.1 + (point.1 - previous.1) * t;
        }
        previous = point;
    }
    previous.1
}

/// Linear ramp of N values towards a target over a fixed number of samples.
#[derive(Debug, Clone, Copy)]
pub struct Ramp<const N: usize> {
    value: [f32; N],
    target: [f32; N],
    step: [f32; N],
    remaining: u32,
}

impl<const N: usize> Ramp<N> {
    pub fn new(value: [f32; N]) -> Self {
        Self {
            value,
            target: value,
            step: [0.0; N],
            remaining: 0,
        }
    }

    pub fn set_target(&mut self, target: [f32; N]) {
        if target == self.target {
            return;
        }
        self.target = target;
        for ((step, target), value) in self.step.iter_mut().zip(target).zip(self.value) {
            *step = (target - value) / SMOOTHING_SAMPLES as f32;
        }
        self.remaining = SMOOTHING_SAMPLES;
    }

    #[inline]
    pub fn is_ramping(&self) -> bool {
        self.remaining > 0
    }

    #[inline]
    pub fn value(&self) -> [f32; N] {
        self.value
    }

    #[inline]
    // A ramp always yields a value, including after it reaches its target; it never ends.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> [f32; N] {
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.value = self.target;
            } else {
                for (value, step) in self.value.iter_mut().zip(self.step) {
                    *value += step;
                }
            }
        }
        self.value
    }
}

/// Gain stage with a click-free 10 ms ramp; exact pass-through at unity.
#[derive(Debug, Clone, Copy)]
pub struct SmoothedGain {
    ramp: Ramp<1>,
}

impl SmoothedGain {
    pub fn new(gain_db: f32) -> Self {
        Self {
            ramp: Ramp::new([db_to_lin(gain_db)]),
        }
    }

    pub fn set_db(&mut self, gain_db: f32) {
        self.ramp.set_target([db_to_lin(gain_db)]);
    }

    pub fn process(&mut self, io: &mut [f32]) {
        if !self.ramp.is_ramping() {
            let [gain] = self.ramp.value();
            if gain != 1.0 {
                io.iter_mut().for_each(|sample| *sample *= gain);
            }
            return;
        }
        for sample in io {
            *sample *= self.ramp.next()[0];
        }
    }
}
