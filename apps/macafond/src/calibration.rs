//! Calibration recording on the control loop: raw samples are drained from the engine every
//! tick until both steps are complete (CAL-001, CAL-002). Progress counts samples, not time.

use macafon_dsp::SAMPLE_RATE_HZ;
use macafon_dsp::calibration::{SILENCE_SAMPLES, SPEECH_SAMPLES, is_clipped};
use macafon_engine::MicControl;
use macafon_ipc::{CalibrationFailure, CalibrationState, CalibrationStep};
use std::time::{Duration, Instant};

/// No raw audio for this long means the graph is not running the microphone.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);
const HOT_INPUT: &str = "Microphone input is clipping. Lower its level in system settings or on the microphone/interface until it no longer clips, then try again.";

pub struct Recording {
    samples: Vec<f32>,
    last_sample_at: Instant,
    #[cfg(any(windows, target_os = "linux"))]
    pub input_device: Option<String>,
    #[cfg(any(windows, target_os = "linux"))]
    adjustments: u8,
    #[cfg(any(windows, target_os = "linux"))]
    settle_until: Option<Instant>,
    #[cfg(any(windows, target_os = "linux"))]
    validation_samples: usize,
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
            #[cfg(any(windows, target_os = "linux"))]
            input_device: None,
            #[cfg(any(windows, target_os = "linux"))]
            adjustments: 0,
            #[cfg(any(windows, target_os = "linux"))]
            settle_until: None,
            #[cfg(any(windows, target_os = "linux"))]
            validation_samples: 0,
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
        #[cfg(any(windows, target_os = "linux"))]
        {
            if self
                .settle_until
                .is_some_and(|until| Instant::now() < until)
            {
                // Drain queued/pre-change packets without using them as fresh measurements.
                self.samples.truncate(before);
                return Poll::Measuring(measuring(self.samples.len()));
            }
            self.settle_until = None;
            self.validation_samples = self
                .validation_samples
                .saturating_sub(self.samples.len() - before);
        }
        // CAL-005: only real clipping touches the system level. Loud but clean speech stays
        // as the user set it; digital Gain alone brings it to the target.
        if self.samples.len() > SILENCE_SAMPLES && is_clipped(&self.samples[SILENCE_SAMPLES..]) {
            #[cfg(any(windows, target_os = "linux"))]
            if self.adjustments < 12 {
                let adjustment = self
                    .input_device
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("no selected capture endpoint"))
                    .and_then(lower_input_level);
                match adjustment {
                    Ok(change_db) => {
                        self.adjustments += 1;
                        // A clipped speech recording cannot be reconstructed: remeasure
                        // speech, and rescale the silence to the new level.
                        self.samples.truncate(SILENCE_SAMPLES);
                        let gain_lin = 10.0f32.powf(change_db / 20.0);
                        for sample in &mut self.samples {
                            *sample *= gain_lin;
                        }
                        self.settle_until = Some(Instant::now() + Duration::from_millis(300));
                        self.validation_samples = SAMPLE_RATE_HZ as usize / 2;
                        tracing::info!(change_db, "Lowered clipping microphone capture level");
                        return Poll::Measuring(measuring(self.samples.len()));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Cannot lower microphone capture level")
                    }
                }
            }
            control.stop_capture();
            return Poll::Failed(CalibrationFailure::Clipped, HOT_INPUT);
        }
        #[cfg(any(windows, target_os = "linux"))]
        if self.validation_samples > 0 {
            return Poll::Measuring(measuring(self.samples.len()));
        }
        if self.samples.len() >= SILENCE_SAMPLES + SPEECH_SAMPLES {
            control.stop_capture();
            return Poll::Complete(std::mem::take(&mut self.samples));
        }
        Poll::Measuring(measuring(self.samples.len()))
    }
}

/// CAL-005: one downward step of the clipping capture endpoint; returns the dB change.
#[cfg(windows)]
fn lower_input_level(device: &str) -> anyhow::Result<f32> {
    let state_dir = macafon_config::state_dir()?;
    Ok(macafon_backend_windows::input_level::lower(
        &state_dir, device,
    )?)
}

/// CAL-005 on Linux: the selected PipeWire source, through WirePlumber's volume control.
#[cfg(target_os = "linux")]
fn lower_input_level(device: &str) -> anyhow::Result<f32> {
    Ok(macafon_backend_pipewire::lower_input_level(device)?)
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
        progress: (offset as f32 / length as f32).min(1.0),
        step_duration_ms: (length as u64 * 1000 / u64::from(SAMPLE_RATE_HZ)) as u32,
    }
}
