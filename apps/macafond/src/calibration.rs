//! Calibration recording on the control loop: raw samples are drained from the engine every
//! tick until both steps are complete (CAL-001, CAL-002). Progress counts samples, not time.

use macafon_dsp::SAMPLE_RATE_HZ;
use macafon_dsp::calibration::{Clipping, SILENCE_SAMPLES, SPEECH_SAMPLES, clipping};
use macafon_engine::MicControl;
use macafon_ipc::{CalibrationFailure, CalibrationState, CalibrationStep};
use std::time::{Duration, Instant};

/// No raw audio for this long means the graph is not running the microphone.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);
const HOT_INPUT: &str = "Microphone input is clipping. Lower its level in system settings or on the microphone/interface until it no longer clips, then try again.";
const HOT_DEVICE: &str = "The microphone clips inside the device, before the system level, so lowering that level cannot help; it was put back. Lower the gain knob on the microphone or interface (or in its own app), then try again.";
/// Flat tops at least this far under full scale come from a converter clip before a digital
/// level stage, not from that stage.
const FLAT_TOP_BELOW_FULL_SCALE: f32 = 0.95;

pub struct Recording {
    samples: Vec<f32>,
    last_sample_at: Instant,
    /// The UI asks for the phrase again after a level change.
    level_lowered: bool,
    #[cfg(any(windows, target_os = "linux"))]
    pub input_device: Option<String>,
    #[cfg(any(windows, target_os = "linux"))]
    adjustments: u8,
    #[cfg(any(windows, target_os = "linux"))]
    level_change_db: f32,
    #[cfg(any(windows, target_os = "linux"))]
    settle_until: Option<Instant>,
    #[cfg(any(windows, target_os = "linux"))]
    validation_samples: usize,
}

pub enum Poll {
    Measuring(CalibrationState),
    Complete {
        samples: Vec<f32>,
        /// CAL-005: total change of the system capture level (≤ 0).
        input_level_change_db: f32,
    },
    Failed(CalibrationFailure, &'static str),
}

impl Recording {
    pub fn start(control: &mut MicControl) -> Self {
        control.start_capture();
        Self {
            samples: Vec::with_capacity(SILENCE_SAMPLES + SPEECH_SAMPLES + SAMPLE_RATE_HZ as usize),
            last_sample_at: Instant::now(),
            level_lowered: false,
            #[cfg(any(windows, target_os = "linux"))]
            input_device: None,
            #[cfg(any(windows, target_os = "linux"))]
            adjustments: 0,
            #[cfg(any(windows, target_os = "linux"))]
            level_change_db: 0.0,
            #[cfg(any(windows, target_os = "linux"))]
            settle_until: None,
            #[cfg(any(windows, target_os = "linux"))]
            validation_samples: 0,
        }
    }

    pub fn initial_state() -> CalibrationState {
        measuring(0, false)
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
                return Poll::Measuring(self.state());
            }
            self.settle_until = None;
            self.validation_samples = self
                .validation_samples
                .saturating_sub(self.samples.len() - before);
        }
        // CAL-005: only real clipping touches the system level, in the silence step too.
        // Loud but clean speech stays as the user set it; digital Gain alone sets the level.
        if let Some(clip) = clipping(&self.samples) {
            return self.clipped(control, clip);
        }
        #[cfg(any(windows, target_os = "linux"))]
        if self.validation_samples > 0 {
            return Poll::Measuring(self.state());
        }
        if self.samples.len() >= SILENCE_SAMPLES + SPEECH_SAMPLES {
            control.stop_capture();
            return Poll::Complete {
                samples: std::mem::take(&mut self.samples),
                input_level_change_db: self.level_change_db(),
            };
        }
        Poll::Measuring(self.state())
    }

    fn state(&self) -> CalibrationState {
        measuring(self.samples.len(), self.level_lowered)
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn level_change_db(&self) -> f32 {
        self.level_change_db
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn level_change_db(&self) -> f32 {
        0.0
    }

    fn clipped(&mut self, control: &mut MicControl, clip: Clipping) -> Poll {
        // Flat tops well under full scale: the converter clipped before a digital level
        // stage. Lowering that stage only moves the flat tops down, so give the user's level
        // back and point at the microphone's own gain.
        if let Clipping::FlatTop { level } = clip
            && level < FLAT_TOP_BELOW_FULL_SCALE
        {
            #[cfg(any(windows, target_os = "linux"))]
            if self.adjustments > 0
                && let Some(device) = &self.input_device
                && let Err(error) = restore_input_level(device)
            {
                tracing::warn!(%error, "Cannot restore microphone capture level");
            }
            control.stop_capture();
            return Poll::Failed(CalibrationFailure::ClippedInDevice, HOT_DEVICE);
        }
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
                    self.level_change_db += change_db;
                    // A clipped recording cannot be reconstructed: clipped speech is measured
                    // again, a clipped silence step starts over.
                    let speech_clean = self.samples.len() > SILENCE_SAMPLES
                        && clipping(&self.samples[..SILENCE_SAMPLES]).is_none();
                    self.samples
                        .truncate(if speech_clean { SILENCE_SAMPLES } else { 0 });
                    let gain_lin = 10.0f32.powf(change_db / 20.0);
                    for sample in &mut self.samples {
                        *sample *= gain_lin;
                    }
                    self.level_lowered = true;
                    self.settle_until = Some(Instant::now() + Duration::from_millis(300));
                    self.validation_samples = SAMPLE_RATE_HZ as usize / 2;
                    tracing::info!(change_db, "Lowered clipping microphone capture level");
                    return Poll::Measuring(self.state());
                }
                Err(error) => {
                    tracing::warn!(%error, "Cannot lower microphone capture level")
                }
            }
        }
        control.stop_capture();
        Poll::Failed(CalibrationFailure::Clipped, HOT_INPUT)
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

#[cfg(windows)]
fn restore_input_level(device: &str) -> anyhow::Result<()> {
    let state_dir = macafon_config::state_dir()?;
    Ok(macafon_backend_windows::input_level::restore_device(
        &state_dir, device,
    )?)
}

/// CAL-005 on Linux: the selected PipeWire source, through WirePlumber's volume control.
#[cfg(target_os = "linux")]
fn lower_input_level(device: &str) -> anyhow::Result<f32> {
    Ok(macafon_backend_pipewire::lower_input_level(device)?)
}

#[cfg(target_os = "linux")]
fn restore_input_level(device: &str) -> anyhow::Result<()> {
    Ok(macafon_backend_pipewire::restore_capture_level(device)?)
}

fn measuring(samples: usize, level_lowered: bool) -> CalibrationState {
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
        level_lowered,
    }
}
