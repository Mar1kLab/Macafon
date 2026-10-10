//! Calibration recording on the control loop: raw samples are drained from the engine every
//! tick until both steps are complete (CAL-001, CAL-002). Progress counts samples, not time.

use macafon_dsp::SAMPLE_RATE_HZ;
use macafon_dsp::calibration::{SILENCE_SAMPLES, SPEECH_SAMPLES};
use macafon_engine::MicControl;
use macafon_ipc::{CalibrationFailure, CalibrationState, CalibrationStep};
use std::time::{Duration, Instant};

/// No raw audio for this long means the graph is not running the microphone.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);

pub struct Recording {
    samples: Vec<f32>,
    last_sample_at: Instant,
}

pub enum Poll {
    Measuring(CalibrationState),
    Complete(Vec<f32>),
    Failed(CalibrationFailure, &'static str),
}

impl Recording {
    pub fn start(control: &mut MicControl) -> Self {
        control.start_capture();
        Self {
            samples: Vec::with_capacity(SILENCE_SAMPLES + SPEECH_SAMPLES + SAMPLE_RATE_HZ as usize),
            last_sample_at: Instant::now(),
        }
    }

    pub fn initial_state() -> CalibrationState {
        measuring(0)
    }

    pub fn poll(&mut self, control: &mut MicControl) -> Poll {
        let before = self.samples.len();
        if control.read_capture(&mut self.samples).is_err() {
            control.stop_capture();
            return Poll::Failed(
                CalibrationFailure::Internal,
                "the daemon could not keep up with the recording",
            );
        }
        if self.samples.len() > before {
            self.last_sample_at = Instant::now();
        } else if self.last_sample_at.elapsed() >= STALL_TIMEOUT {
            control.stop_capture();
            return Poll::Failed(
                CalibrationFailure::Stalled,
                "no audio is arriving from the microphone",
            );
        }
        if self.samples.len() >= SILENCE_SAMPLES + SPEECH_SAMPLES {
            control.stop_capture();
            return Poll::Complete(std::mem::take(&mut self.samples));
        }
        Poll::Measuring(measuring(self.samples.len()))
    }
}

fn measuring(samples: usize) -> CalibrationState {
    let (step, offset, length) = if samples < SILENCE_SAMPLES {
        (CalibrationStep::Silence, samples, SILENCE_SAMPLES)
    } else {
        (
            CalibrationStep::Speech,
            samples - SILENCE_SAMPLES,
            SPEECH_SAMPLES,
        )
    };
    CalibrationState::Measuring {
        step,
        progress: offset as f32 / length as f32,
        step_duration_ms: (length as u64 * 1000 / u64::from(SAMPLE_RATE_HZ)) as u32,
    }
}
