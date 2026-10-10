//! A/B test player (PRD §6.4): loops a raw recording, runs it through its own copy of the
//! microphone chain (B) and plays either the raw signal (A) or the processed one.
//!
//! A is delayed by the chain latency so both sides share one playback position, and switching
//! is a 20 ms crossfade (AB-004). With loudness match on, A follows B's long-term level, so
//! "louder" does not sound "better" (AB-005).

use crate::{MicControl, MicRt, microphone};
use macafon_dsp::{ChainSettings, ParameterError, SAMPLE_RATE_HZ, Telemetry};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// AB-004: A ⇄ B crossfade.
pub const SWITCH_FADE_SAMPLES: u32 = SAMPLE_RATE_HZ / 50;
const BLOCK_SAMPLES: usize = 1024;
/// Long-term level window of the loudness match.
const LOUDNESS_WINDOW_S: f32 = 2.0;
/// Match gain follows level changes this fast.
const MATCH_SMOOTHING_S: f32 = 0.05;
const MAX_MATCH_DB: f32 = 24.0;
/// Below this mean square (−80 dBFS) a side counts as silent; the match gain holds.
const SILENT_POWER: f32 = 1e-8;
const NO_SEEK: u32 = u32::MAX;

#[derive(Default)]
struct Shared {
    /// Playback position in samples, written by the RT thread.
    position: AtomicU32,
    side_b: AtomicBool,
    loudness_match: AtomicBool,
    seek: AtomicU32,
}

/// Build a player for `clip` (raw samples, at least one block long) with the chain `settings`.
pub fn ab_player(
    clip: Arc<[f32]>,
    settings: &ChainSettings,
    side_b: bool,
    loudness_match: bool,
) -> Result<(AbControl, AbRt), ParameterError> {
    // B keeps its own meters: the A/B copy must not move the live microphone's meters.
    let (chain, rt) = microphone(settings, Arc::new(Telemetry::default()))?;
    let shared = Arc::new(Shared {
        position: AtomicU32::new(0),
        side_b: AtomicBool::new(side_b),
        loudness_match: AtomicBool::new(loudness_match),
        seek: AtomicU32::new(NO_SEEK),
    });
    Ok((
        AbControl {
            chain,
            shared: shared.clone(),
            len: clip.len(),
        },
        AbRt {
            clip,
            chain: rt,
            shared,
            position: 0,
            mix: if side_b { 1.0 } else { 0.0 },
            power_a: 0.0,
            power_b: 0.0,
            match_gain: 1.0,
            dry: vec![0.0; BLOCK_SAMPLES].into_boxed_slice(),
        },
    ))
}

/// Control-thread handle of a playing A/B player.
pub struct AbControl {
    chain: MicControl,
    shared: Arc<Shared>,
    len: usize,
}

impl AbControl {
    /// Keep B in sync with the live chain (AB-003). Mute does not apply to the test.
    pub fn apply(&mut self, settings: &ChainSettings) -> Result<(), crate::EngineError> {
        let settings = ChainSettings {
            muted: false,
            ..*settings
        };
        if &settings == self.chain.settings() {
            return Ok(());
        }
        self.chain.apply(&settings)
    }

    pub fn select_b(&self, side_b: bool) {
        self.shared.side_b.store(side_b, Ordering::Relaxed);
    }

    pub fn set_loudness_match(&self, enabled: bool) {
        self.shared.loudness_match.store(enabled, Ordering::Relaxed);
    }

    /// Jump to `sample` (wrapped into the clip).
    pub fn seek(&self, sample: usize) {
        self.shared
            .seek
            .store((sample % self.len) as u32, Ordering::Relaxed);
    }

    pub fn position_samples(&self) -> usize {
        self.shared.position.load(Ordering::Relaxed) as usize
    }

    pub fn collect_garbage(&mut self) {
        self.chain.collect_garbage();
    }
}

/// Audio-thread side: fills playback buffers. RT-safe.
pub struct AbRt {
    clip: Arc<[f32]>,
    chain: MicRt,
    shared: Arc<Shared>,
    /// Next clip sample fed to the B chain.
    position: usize,
    /// 0 = A, 1 = B.
    mix: f32,
    power_a: f32,
    power_b: f32,
    match_gain: f32,
    dry: Box<[f32]>,
}

impl AbRt {
    /// RT-safe: no allocation, locks or deallocation.
    pub fn process(&mut self, out: &mut [f32]) {
        let seek = self.shared.seek.swap(NO_SEEK, Ordering::Relaxed);
        if seek != NO_SEEK {
            self.position = seek as usize % self.clip.len();
        }
        let target = if self.shared.side_b.load(Ordering::Relaxed) {
            1.0
        } else {
            0.0
        };
        let matching = self.shared.loudness_match.load(Ordering::Relaxed);
        let fade_step = 1.0 / SWITCH_FADE_SAMPLES as f32;
        let smoothing = 1.0 - (-1.0 / (MATCH_SMOOTHING_S * SAMPLE_RATE_HZ as f32)).exp();
        for block in out.chunks_mut(BLOCK_SAMPLES) {
            // A lags by the chain latency so both sides play the same moment.
            let latency = self.chain.latency_samples() as usize % self.clip.len();
            let len = self.clip.len();
            let mut a_index = (self.position + len - latency) % len;
            let dry = &mut self.dry[..block.len()];
            for (wet, dry) in block.iter_mut().zip(dry.iter_mut()) {
                *wet = self.clip[self.position];
                *dry = self.clip[a_index];
                self.position = (self.position + 1) % len;
                a_index = (a_index + 1) % len;
            }
            self.chain.process(block);

            let window = block.len() as f32 / (LOUDNESS_WINDOW_S * SAMPLE_RATE_HZ as f32);
            let alpha = 1.0 - (-window).exp();
            let mean_square =
                |samples: &[f32]| samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
            self.power_a += alpha * (mean_square(dry) - self.power_a);
            self.power_b += alpha * (mean_square(block) - self.power_b);
            let match_target = if !matching {
                1.0
            } else if self.power_a > SILENT_POWER && self.power_b > SILENT_POWER {
                let max = db_to_lin(MAX_MATCH_DB);
                (self.power_b / self.power_a).sqrt().clamp(1.0 / max, max)
            } else {
                self.match_gain
            };

            for (sample, dry) in block.iter_mut().zip(dry.iter()) {
                self.match_gain += smoothing * (match_target - self.match_gain);
                if self.mix < target {
                    self.mix = (self.mix + fade_step).min(target);
                } else if self.mix > target {
                    self.mix = (self.mix - fade_step).max(target);
                }
                let a = dry * self.match_gain;
                *sample = macafon_dsp::sanitize(a + self.mix * (*sample - a));
            }
        }
        self.shared
            .position
            .store(self.position as u32, Ordering::Relaxed);
    }
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    use macafon_dsp::Preset;

    fn tone(len: usize, amplitude: f32) -> Arc<[f32]> {
        (0..len)
            .map(|n| amplitude * (n as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect()
    }

    fn play(rt: &mut AbRt, samples: usize) -> Vec<f32> {
        let mut out = vec![0.0; samples];
        assert_no_alloc(|| out.chunks_mut(128).for_each(|block| rt.process(block)));
        out
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |peak, x| peak.max(x.abs()))
    }

    #[test]
    fn switches_sides_with_a_short_crossfade_and_matches_loudness() {
        let mut quieter = ChainSettings::raw();
        quieter.gain.params.gain_db = -12.0;
        let (control, mut rt) =
            ab_player(tone(48_000, 0.5), &quieter, true, false).expect("valid settings");
        let b = play(&mut rt, 9600);
        assert!((peak(&b[4800..]) - 0.5 * 10f32.powf(-0.6)).abs() < 0.01);
        control.select_b(false);
        let switching = play(&mut rt, 9600);
        let max_step = switching
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max);
        // A 440 Hz sine at 0.5 moves at most 0.029 per sample; a hard switch would jump.
        assert!(max_step < 0.035, "switch clicked: {max_step}");
        assert!((peak(&switching[4800..]) - 0.5).abs() < 0.01);
        // Loudness match pulls A down to B's level.
        control.set_loudness_match(true);
        let matched = play(&mut rt, 4 * 48_000);
        assert!(
            (peak(&matched[3 * 48_000..]) - 0.5 * 10f32.powf(-0.6)).abs() < 0.02,
            "{}",
            peak(&matched[3 * 48_000..])
        );
    }

    #[test]
    fn a_is_aligned_with_the_latency_of_b_and_follows_seeks() {
        // Natural delays B by the NS frame; A must be delayed by the same amount.
        let mut clip = vec![0.0; 4800];
        clip[1000] = 1.0;
        let (control, mut rt) = ab_player(clip.into(), &Preset::Natural.settings(), false, false)
            .expect("valid settings");
        let a = play(&mut rt, 4800);
        let latency = macafon_dsp::LITE_LATENCY_SAMPLES as usize;
        assert_eq!(a[1000 + latency], 1.0);
        assert_eq!(control.position_samples(), 0);
        control.seek(4000);
        play(&mut rt, 128);
        assert_eq!(control.position_samples(), 4128);
    }
}
