//! Capture control on the daemon control loop.

use super::{DaemonHost, connected, reply};
use crate::calibration::{Poll, Recording};
use macafon_dsp::calibration::CalibrationError;
use macafon_ipc::{CalibrationFailure, CalibrationState, ErrorBody, ErrorCode, Event, Request};

impl DaemonHost {
    pub(super) fn set_calibration(&mut self, state: CalibrationState) {
        self.calibration = state;
        self.hub
            .broadcast(&Event::CalibrationChanged(self.calibration.clone()));
    }

    pub(super) fn ab_changed(&self) {
        self.hub.broadcast(&Event::AbChanged(self.ab.state()));
    }

    /// Calibration and the A/B recorder share the raw capture.
    pub(super) fn capture_free(&self) -> Result<(), ErrorBody> {
        if self.recording.is_some() {
            Err(ErrorBody::new(ErrorCode::Busy, "calibration is running"))
        } else if self.ab.is_recording() {
            Err(ErrorBody::new(
                ErrorCode::Busy,
                "an A/B recording is running",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn respond_ab(&mut self, request: Request) -> Result<serde_json::Value, ErrorBody> {
        match request {
            Request::AbRecord { max_seconds } => {
                self.capture_free()?;
                let control = connected(&mut self.control, self.input_connected)?;
                self.ab.record(control, max_seconds)?;
            }
            Request::AbStopRecording => {
                let control = connected(&mut self.control, self.input_connected)?;
                self.ab.stop_recording(control)?;
            }
            Request::AbPlay { device } => {
                // With the mixer on the default output is a channel; play to the real device.
                let device = device.or_else(|| {
                    self.config
                        .mixer
                        .enabled
                        .then(|| self.mixer.output_device(&self.config.mixer))
                        .flatten()
                });
                self.ab.play(&self.config.mic.chain, device)?
            }
            Request::AbStop => self.ab.stop(),
            Request::AbSelect { side } => self.ab.select(side),
            Request::AbToggle => self.ab.toggle(),
            Request::AbSeek { position_ms } => self.ab.seek(position_ms)?,
            Request::AbSetLoudnessMatch { enabled } => self.ab.set_loudness_match(enabled),
            Request::AbWaveform { points } => {
                let waveform = self.ab.waveform(&self.config.mic.chain, points)?;
                return reply(waveform);
            }
            _ => {}
        }
        let state = self.ab.state();
        self.hub.broadcast(&Event::AbChanged(state.clone()));
        reply(state)
    }

    pub(super) fn start_calibration(&mut self) -> Result<(), ErrorBody> {
        if self.ab.is_recording() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "an A/B recording is running",
            ));
        }
        if self.recording.is_some() {
            return Err(ErrorBody::new(
                ErrorCode::Busy,
                "calibration is already running",
            ));
        }
        let control = connected(&mut self.control, self.input_connected)?;
        self.recording = Some(Recording::start(control));
        self.set_calibration(Recording::initial_state());
        Ok(())
    }

    pub(super) fn calibration_value(&self) -> Result<serde_json::Value, ErrorBody> {
        reply(&self.calibration)
    }

    pub(super) fn fail_calibration(
        &mut self,
        reason: CalibrationFailure,
        message: impl Into<String>,
    ) {
        if let Some(control) = &mut self.control {
            control.stop_capture();
        }
        self.recording = None;
        let message = message.into();
        tracing::warn!(?reason, %message, "Calibration failed");
        self.set_calibration(CalibrationState::Failed { reason, message });
    }

    pub(super) fn poll_calibration(&mut self) {
        let (Some(recording), Some(control)) = (&mut self.recording, &mut self.control) else {
            return;
        };
        match recording.poll(control) {
            Poll::Measuring(state) => self.set_calibration(state),
            Poll::Failed(reason, message) => self.fail_calibration(reason, message),
            Poll::Complete(samples) => {
                self.recording = None;
                self.finish_calibration(&samples);
            }
        }
    }

    /// CAL-003: analyse, then apply the calibrated chain like any other change.
    pub(super) fn finish_calibration(&mut self, samples: &[f32]) {
        let (settings, report) =
            match macafon_dsp::calibration::calibrate(samples, &self.config.mic.chain) {
                Ok(result) => result,
                Err(error) => {
                    let reason = match error {
                        CalibrationError::Clipped { .. } => CalibrationFailure::Clipped,
                        CalibrationError::NoSpeech => CalibrationFailure::NoSpeech,
                        CalibrationError::TooShort => CalibrationFailure::Internal,
                    };
                    return self.fail_calibration(reason, error.to_string());
                }
            };
        let measured_device = self.active_device.clone();
        let detected_channel = self.detected_channel;
        let applied = self.update(|config| {
            config.mic.chain = settings;
            config.mic.noise_floor_dbfs = Some(report.noise_floor_dbfs);
            config.mic.noise_floor_device = measured_device;
            if config.mic.input_channel == macafon_dsp::InputChannel::Auto {
                config.mic.input_channel = detected_channel;
            }
        });
        match applied {
            Ok(_) => {
                tracing::info!(
                    noise_floor_dbfs = report.noise_floor_dbfs,
                    speech_rms_dbfs = report.speech_rms_dbfs,
                    snr_db = report.snr_db,
                    snr_after_db = report.snr_after_db,
                    gain_db = report.gain_db,
                    "Calibration applied"
                );
                self.set_calibration(CalibrationState::Done { report });
            }
            Err(error) => self.fail_calibration(CalibrationFailure::Internal, error.message),
        }
    }
}
