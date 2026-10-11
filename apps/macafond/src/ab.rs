//! A/B test on the control loop (PRD §6.4): recording the raw microphone, the loop player
//! and its waveform. The clip lives in memory only (AB-007).

use macafon_backend_pipewire::PlayerRequest;
use macafon_dsp::{ChainSettings, SAMPLE_RATE_HZ};
use macafon_engine::{AbControl, MicControl};
use macafon_ipc::{AbRecording, AbSide, AbState, AbWaveform, ErrorBody, ErrorCode};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// AB-001: 3–30 s, 10 s by default.
const MIN_SECONDS: u32 = 3;
const MAX_SECONDS: u32 = 30;
const DEFAULT_SECONDS: u32 = 10;
const STALL_TIMEOUT: Duration = Duration::from_secs(2);
pub const WAVEFORM_POINTS: std::ops::RangeInclusive<u32> = 16..=4096;

struct Recording {
    samples: Vec<f32>,
    max_samples: usize,
    last_sample_at: Instant,
}

pub struct Ab {
    recording: Option<Recording>,
    clip: Option<Arc<[f32]>>,
    player: Option<AbControl>,
    request: Option<PlayerRequest>,
    side: AbSide,
    loudness_match: bool,
    device: Option<String>,
}

fn samples_to_ms(samples: usize) -> u32 {
    (samples as u64 * 1000 / u64::from(SAMPLE_RATE_HZ)) as u32
}

fn invalid(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::InvalidParams, message)
}

impl Default for Ab {
    fn default() -> Self {
        Self {
            recording: None,
            clip: None,
            player: None,
            request: None,
            side: AbSide::B,
            loudness_match: true,
            device: None,
        }
    }
}

impl Ab {
    pub fn state(&self) -> AbState {
        AbState {
            recording: self.recording.as_ref().map(|recording| AbRecording {
                elapsed_ms: samples_to_ms(recording.samples.len()),
                max_ms: samples_to_ms(recording.max_samples),
            }),
            clip_ms: self.clip.as_ref().map(|clip| samples_to_ms(clip.len())),
            playing: self.player.is_some(),
            side: self.side,
            position_ms: self
                .player
                .as_ref()
                .map_or(0, |player| samples_to_ms(player.position_samples())),
            loudness_match: self.loudness_match,
            device: self.device.clone(),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Recording or playing: worth a progress event every tick.
    pub fn is_active(&self) -> bool {
        self.recording.is_some() || self.player.is_some()
    }

    pub fn take_request(&mut self) -> Option<PlayerRequest> {
        self.request.take()
    }

    pub fn record(
        &mut self,
        control: &mut MicControl,
        max_seconds: Option<u32>,
    ) -> Result<(), ErrorBody> {
        let seconds = max_seconds.unwrap_or(DEFAULT_SECONDS);
        if !(MIN_SECONDS..=MAX_SECONDS).contains(&seconds) {
            return Err(invalid(format!(
                "recording length must be {MIN_SECONDS}–{MAX_SECONDS} s"
            )));
        }
        self.stop();
        let max_samples = seconds as usize * SAMPLE_RATE_HZ as usize;
        control.start_capture();
        self.recording = Some(Recording {
            samples: Vec::with_capacity(max_samples + SAMPLE_RATE_HZ as usize),
            max_samples,
            last_sample_at: Instant::now(),
        });
        Ok(())
    }

    pub fn stop_recording(&mut self, control: &mut MicControl) -> Result<(), ErrorBody> {
        let Some(recording) = &mut self.recording else {
            return Err(invalid("not recording"));
        };
        let _ = control.read_capture(&mut recording.samples);
        if recording.samples.len() < MIN_SECONDS as usize * SAMPLE_RATE_HZ as usize {
            return Err(invalid(format!("record at least {MIN_SECONDS} s")));
        }
        self.finish(control);
        Ok(())
    }

    /// End an interrupted recording; keep it if it is long enough.
    pub fn abort_recording(&mut self, control: Option<&mut MicControl>) {
        if let Some(control) = control {
            control.stop_capture();
        }
        if let Some(recording) = self.recording.take() {
            if recording.samples.len() >= MIN_SECONDS as usize * SAMPLE_RATE_HZ as usize {
                self.recording = Some(recording);
                self.finalize();
            }
        }
    }

    fn finish(&mut self, control: &mut MicControl) {
        control.stop_capture();
        self.finalize();
    }

    fn finalize(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let mut samples = recording.samples;
        samples.truncate(recording.max_samples);
        tracing::info!(
            seconds = samples.len() as f32 / SAMPLE_RATE_HZ as f32,
            "A/B clip recorded"
        );
        self.clip = Some(samples.into());
    }

    /// Drain the capture; true when something changed.
    pub fn poll(&mut self, control: &mut MicControl) -> bool {
        if let Some(player) = &mut self.player {
            player.collect_garbage();
        }
        let Some(recording) = &mut self.recording else {
            return false;
        };
        let before = recording.samples.len();
        let overrun = control.read_capture(&mut recording.samples).is_err();
        if recording.samples.len() > before {
            recording.last_sample_at = Instant::now();
        }
        if overrun || recording.last_sample_at.elapsed() >= STALL_TIMEOUT {
            tracing::warn!(overrun, "A/B recording interrupted");
            self.abort_recording(Some(control));
        } else if recording.samples.len() >= recording.max_samples {
            self.finish(control);
        }
        true
    }

    pub fn play(
        &mut self,
        settings: &ChainSettings,
        device: Option<String>,
    ) -> Result<(), ErrorBody> {
        if self.recording.is_some() {
            return Err(ErrorBody::new(ErrorCode::Busy, "recording in progress"));
        }
        let clip = self
            .clip
            .clone()
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "record a clip first"))?;
        let (control, rt) =
            macafon_engine::ab_player(clip, settings, self.side == AbSide::B, self.loudness_match)
                .map_err(|error| invalid(error.to_string()))?;
        self.player = Some(control);
        self.device = device.clone();
        self.request = Some(PlayerRequest::Start {
            player: rt,
            target: device,
        });
        Ok(())
    }

    pub fn stop(&mut self) {
        if self.player.take().is_some() {
            self.request = Some(PlayerRequest::Stop);
        }
    }

    /// The stream is gone (PipeWire restart or creation failure).
    pub fn player_lost(&mut self) {
        self.player = None;
        self.request = None;
    }

    pub fn select(&mut self, side: AbSide) {
        self.side = side;
        if let Some(player) = &self.player {
            player.select_b(side == AbSide::B);
        }
    }

    pub fn toggle(&mut self) {
        self.select(match self.side {
            AbSide::A => AbSide::B,
            AbSide::B => AbSide::A,
        });
    }

    pub fn seek(&mut self, position_ms: u32) -> Result<(), ErrorBody> {
        let player = self.player.as_ref().ok_or_else(|| invalid("not playing"))?;
        player.seek((u64::from(position_ms) * u64::from(SAMPLE_RATE_HZ) / 1000) as usize);
        Ok(())
    }

    pub fn set_loudness_match(&mut self, enabled: bool) {
        self.loudness_match = enabled;
        if let Some(player) = &self.player {
            player.set_loudness_match(enabled);
        }
    }

    /// AB-003: B follows every change of the live chain.
    pub fn settings_changed(&mut self, settings: &ChainSettings) {
        if let Some(player) = &mut self.player {
            if let Err(error) = player.apply(settings) {
                tracing::warn!(%error, "A/B chain not updated");
            }
        }
    }

    /// Forget all captured audio when the UI connection closes.
    pub fn clear(&mut self, control: Option<&mut MicControl>) {
        if self.recording.is_some() {
            if let Some(control) = control {
                control.stop_capture();
                // The capture ring may still hold microphone samples; drain it before dropping.
                let mut discarded = Vec::new();
                let _ = control.read_capture(&mut discarded);
            }
        }
        self.recording = None;
        self.clip = None;
        self.stop();
        self.device = None;
    }

    /// Explicit export only; the clip otherwise never leaves memory.
    pub fn export(
        &self,
        settings: &ChainSettings,
        side: AbSide,
        path: &Path,
    ) -> Result<(), ErrorBody> {
        let clip = self
            .clip
            .as_ref()
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "record a clip first"))?;
        let processed;
        let samples = match side {
            AbSide::A => clip.as_ref(),
            AbSide::B => {
                processed = render_b(clip, settings)?;
                processed.as_slice()
            }
        };
        // Avoid silently replacing another recording or following an existing symlink.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))?;
        write_wav(&mut file, samples)
            .and_then(|()| file.sync_all())
            .map_err(|error| ErrorBody::new(ErrorCode::Internal, error.to_string()))
    }

    /// AB-006: peaks of A and of B through `settings`, time-aligned.
    pub fn waveform(&self, settings: &ChainSettings, points: u32) -> Result<AbWaveform, ErrorBody> {
        let clip = self
            .clip
            .as_ref()
            .ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, "record a clip first"))?;
        if !WAVEFORM_POINTS.contains(&points) {
            return Err(invalid(format!(
                "points must be {}–{}",
                WAVEFORM_POINTS.start(),
                WAVEFORM_POINTS.end()
            )));
        }
        let processed = render_b(clip, settings)?;
        Ok(AbWaveform {
            a: peaks(clip, points as usize),
            b: peaks(&processed, points as usize),
        })
    }
}

fn peaks(samples: &[f32], points: usize) -> Vec<f32> {
    (0..points)
        .map(|point| {
            let start = point * samples.len() / points;
            let end = ((point + 1) * samples.len() / points)
                .max(start + 1)
                .min(samples.len());
            samples[start.min(end)..end]
                .iter()
                .fold(0.0f32, |peak, x| peak.max(x.abs()))
        })
        .collect()
}

fn render_b(clip: &[f32], settings: &ChainSettings) -> Result<Vec<f32>, ErrorBody> {
    let settings = ChainSettings {
        muted: false,
        ..*settings
    };
    let mut processor = macafon_engine::prepare_offline_processor(&settings, Arc::default())
        .map_err(|error| invalid(error.to_string()))?;
    let latency = processor.latency_samples() as usize;
    let mut processed = clip.to_vec();
    processed.resize(clip.len() + latency, 0.0);
    processed
        .chunks_mut(macafon_dsp::FRAME_SAMPLES)
        .for_each(|block| processor.process(block));
    processed.drain(..latency);
    Ok(processed)
}

/// Mono IEEE float WAV at the graph sample rate, preserving the raw samples exactly.
fn write_wav(writer: &mut impl Write, samples: &[f32]) -> std::io::Result<()> {
    let bytes = u32::try_from(samples.len())
        .ok()
        .and_then(|length| length.checked_mul(4))
        .ok_or_else(|| std::io::Error::other("WAV too large"))?;
    writer.write_all(b"RIFF")?;
    writer.write_all(&(48 + bytes).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16u32.to_le_bytes())?;
    writer.write_all(&3u16.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE_HZ.to_le_bytes())?;
    writer.write_all(&(SAMPLE_RATE_HZ * 4).to_le_bytes())?;
    writer.write_all(&4u16.to_le_bytes())?;
    writer.write_all(&32u16.to_le_bytes())?;
    writer.write_all(b"fact")?;
    writer.write_all(&4u32.to_le_bytes())?;
    writer.write_all(&(bytes / 4).to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&bytes.to_le_bytes())?;
    for sample in samples {
        writer.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn wav_export_preserves_samples_and_clear_forgets_clip() {
        let mut ab = Ab {
            clip: Some(vec![0.25, -0.75].into()),
            ..Ab::default()
        };
        let mut bytes = Vec::new();
        write_wav(&mut bytes, ab.clip.as_ref().unwrap()).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(&bytes[56..60], &0.25f32.to_le_bytes());
        assert_eq!(&bytes[60..64], &(-0.75f32).to_le_bytes());
        ab.clear(None);
        assert!(ab.state().clip_ms.is_none());
    }
}
