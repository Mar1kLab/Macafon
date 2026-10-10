//! A/B test on the control loop (PRD §6.4): recording the raw microphone, the loop player
//! and its waveform. The clip lives in memory only (AB-007).

use macafon_backend_pipewire::PlayerRequest;
use macafon_dsp::{ChainSettings, MicrophoneProcessor, SAMPLE_RATE_HZ};
use macafon_engine::{AbControl, MicControl};
use macafon_ipc::{AbRecording, AbSide, AbState, AbWaveform, ErrorBody, ErrorCode};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// AB-001: 3–30 s, 10 s by default.
const MIN_SECONDS: u32 = 3;
const MAX_SECONDS: u32 = 30;
const DEFAULT_SECONDS: u32 = 10;
/// The loop seam is faded so it does not click.
const EDGE_FADE_SAMPLES: usize = SAMPLE_RATE_HZ as usize / 100;
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
        let fade = EDGE_FADE_SAMPLES.min(samples.len() / 2);
        for index in 0..fade {
            let gain = index as f32 / fade as f32;
            samples[index] *= gain;
            let last = samples.len() - 1 - index;
            samples[last] *= gain;
        }
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
        let settings = ChainSettings {
            muted: false,
            ..*settings
        };
        let mut processor =
            MicrophoneProcessor::new(&settings).map_err(|error| invalid(error.to_string()))?;
        let latency = processor.latency_samples() as usize;
        let mut processed = clip.to_vec();
        processed.resize(clip.len() + latency, 0.0);
        processed
            .chunks_mut(macafon_dsp::FRAME_SAMPLES)
            .for_each(|block| processor.process(block));
        Ok(AbWaveform {
            a: peaks(clip, points as usize),
            b: peaks(&processed[latency..], points as usize),
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
