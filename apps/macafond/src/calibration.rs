//! Calibration recording on the control loop: raw samples are drained from the engine every
//! tick until both steps are complete (CAL-001, CAL-002). Progress counts samples, not time.

use macafon_dsp::SAMPLE_RATE_HZ;
use macafon_dsp::calibration::{RAW_SPEECH_PEAK_DBFS, SILENCE_SAMPLES, SPEECH_SAMPLES};
use macafon_engine::MicControl;
use macafon_ipc::{CalibrationFailure, CalibrationState, CalibrationStep};
use std::time::{Duration, Instant};

/// No raw audio for this long means the graph is not running the microphone.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);
const HOT_INPUT: &str = "Microphone input is too hot. Lower its level in system settings or on the interface until speech peaks stay below -15 dBFS, then try again.";

pub struct Recording {
    samples: Vec<f32>,
    last_sample_at: Instant,
    #[cfg(windows)]
    pub input_device: Option<String>,
    #[cfg(windows)]
    adjustments: u8,
    #[cfg(windows)]
    settle_until: Option<Instant>,
    #[cfg(windows)]
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
            #[cfg(windows)]
            input_device: None,
            #[cfg(windows)]
            adjustments: 0,
            #[cfg(windows)]
            settle_until: None,
            #[cfg(windows)]
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
        #[cfg(windows)]
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
        // Use speech peaks, not RMS, to reserve room before the raw tap. Digital Gain
        // cannot repair a saturated capture endpoint. Wait for half a second of speech.
        if self.samples.len() >= SILENCE_SAMPLES + SAMPLE_RATE_HZ as usize / 2 {
            let peak = self.samples[SILENCE_SAMPLES..]
                .iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            let peak_dbfs = 20.0 * peak.max(1e-6).log10();
            if peak_dbfs > RAW_SPEECH_PEAK_DBFS {
                #[cfg(windows)]
                if self.adjustments < 12 {
                    let adjustment = (|| {
                        let device = self
                            .input_device
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("no selected capture endpoint"))?;
                        let state_dir = macafon_config::state_dir()?;
                        Ok::<_, anyhow::Error>(macafon_backend_windows::input_level::lower(
                            &state_dir, device, peak_dbfs,
                        )?)
                    })();
                    match adjustment {
                        Ok(change_db) => {
                            self.adjustments += 1;
                            // A clipped speech recording cannot be reconstructed. Remeasure
                            // speech; otherwise compute the new levels from clean samples.
                            if peak >= 0.989 {
                                self.samples.truncate(SILENCE_SAMPLES);
                            }
                            let gain_lin = 10.0f32.powf(change_db / 20.0);
                            for sample in &mut self.samples {
                                *sample *= gain_lin;
                            }
                            self.settle_until = Some(Instant::now() + Duration::from_millis(300));
                            self.validation_samples = SAMPLE_RATE_HZ as usize / 2;
                            tracing::info!(
                                peak_dbfs,
                                change_db,
                                "Lowered microphone capture level"
                            );
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
        }
        #[cfg(windows)]
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
