//! Native popup data and calibrated VU mechanics (0 VU = -18 dBFS RMS).
use macafon_ipc::DictationPopupLook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Cancelled,
    Listening,
    Transcribing,
    Done,
    Copied,
    NoSpeech,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub look: DictationPopupLook,
    pub state: State,
    pub rms_dbfs: f32,
    pub bands: [f32; 15],
    pub elapsed_s: u32,
    pub engine: String,
    pub words: usize,
    pub language: String,
    pub hands_free: bool,
}

/// Linear amplitude drives the needle, with an underdamped movement calibrated to 300 ms.
#[derive(Default)]
pub struct Vu {
    pub value: f32,
    velocity: f32,
}
impl Vu {
    pub fn step(&mut self, rms_dbfs: f32, dt: f32) {
        let target = 10.0f32.powf((rms_dbfs + 18.0) / 20.0).clamp(0.0, 1.6);
        let omega = 26.0;
        let damping = 0.8;
        let steps = (dt.min(0.1) * 1000.0).ceil().max(1.0) as usize;
        let h = dt.min(0.1) / steps as f32;
        for _ in 0..steps {
            self.velocity += (-omega * omega * (self.value - target)
                - 2.0 * damping * omega * self.velocity)
                * h;
            self.value += self.velocity * h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vu_reaches_reference_in_300_ms_with_small_overshoot() {
        let mut vu = Vu::default();
        let mut peak = 0.0f32;
        for n in 0..1000 {
            vu.step(-18.0, 0.001);
            peak = peak.max(vu.value);
            if n == 299 {
                assert!((vu.value - 1.0).abs() <= 0.01, "300 ms: {}", vu.value);
            }
        }
        assert!((1.01..1.02).contains(&peak), "overshoot {peak}");
        assert!((vu.value - 1.0).abs() < 0.001);
    }
}
