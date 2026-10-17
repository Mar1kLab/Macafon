//! Lock-free, independently clocked playback taps.
//!
//! All command and garbage rings are allocated on the control thread. A control-side
//! lease is retained for every sent sound until its return; retired endpoints retain
//! their leases until their tap has been destroyed. Call `tick` regularly to reclaim.
use crate::decode::{Sound, sanitize};
use macafon_dsp::SAMPLE_RATE_HZ;
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
const COMMANDS: usize = 32;
const RETURNS: usize = 64;
const FADE_FRAMES: usize = SAMPLE_RATE_HZ as usize / 200;
const GAIN_FRAMES: f32 = SAMPLE_RATE_HZ as f32 / 50.0;
const ALIVE: Duration = Duration::from_millis(300);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Microphone,
    Monitor,
}
impl Output {
    fn index(self) -> usize {
        match self {
            Self::Microphone => 0,
            Self::Monitor => 1,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackStatus {
    pub token: Option<u64>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub paused: bool,
    pub looping: bool,
}
struct Settings {
    gain: AtomicUsize,
    enabled: AtomicBool,
    duck: AtomicBool,
}
impl Settings {
    fn new() -> Self {
        Self {
            gain: AtomicUsize::new(1.0_f32.to_bits() as usize),
            enabled: AtomicBool::new(true),
            duck: AtomicBool::new(false),
        }
    }
}
struct Shared {
    active: AtomicBool,
    heartbeat: AtomicU64,
    token: AtomicU64,
    position: AtomicUsize,
    revision: AtomicU64,
    finished: AtomicBool,
}
impl Shared {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(true),
            heartbeat: AtomicU64::new(0),
            token: AtomicU64::new(0),
            position: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }
}
struct Returned {
    lease: usize,
    sound: Arc<Sound>,
}
struct Command {
    lease: usize,
    sound: Option<Arc<Sound>>,
    token: u64,
    position: usize,
    revision: u64,
    gain_lin: f32,
    paused: bool,
    looping: bool,
}
struct Endpoint {
    commands: Producer<Command>,
    returned: Consumer<Returned>,
    shared: Arc<Shared>,
    leases: [Option<Arc<Sound>>; RETURNS],
    heartbeat: u64,
    last_seen: Option<Instant>,
    dirty: bool,
    sent_revision: Option<u64>,
}
impl Endpoint {
    fn reclaim(&mut self) {
        while let Ok(returned) = self.returned.pop() {
            self.leases[returned.lease] = None;
            drop(returned.sound);
            // Both references are freed here, never in a render call.
        }
    }
    fn alive(&self, now: Instant) -> bool {
        self.shared.active.load(Ordering::Acquire)
            && self
                .last_seen
                .is_some_and(|t| now.saturating_duration_since(t) <= ALIVE)
    }
}
pub struct SoundpadControl {
    outputs: [Arc<Settings>; 2],
    endpoints: [Option<Endpoint>; 2],
    retired: Vec<Endpoint>,
    sound: Option<Arc<Sound>>,
    token: u64,
    next_token: u64,
    revision: u64,
    anchor: Instant,
    anchor_frame: usize,
    gain_lin: f32,
    paused: bool,
    looping: bool,
}
impl Default for SoundpadControl {
    fn default() -> Self {
        Self::new()
    }
}
impl SoundpadControl {
    pub fn new() -> Self {
        Self {
            outputs: std::array::from_fn(|_| Arc::new(Settings::new())),
            endpoints: [None, None],
            retired: Vec::new(),
            sound: None,
            token: 0,
            next_token: 0,
            revision: 0,
            anchor: Instant::now(),
            anchor_frame: 0,
            gain_lin: 1.0,
            paused: false,
            looping: false,
        }
    }
    /// Replaces the previous tap for this output. The old tap becomes silent on its next call.
    pub fn tap(&mut self, output: Output) -> SoundpadTap {
        let index = output.index();
        if let Some(old) = self.endpoints[index].take() {
            old.shared.active.store(false, Ordering::Release);
            self.retired.push(old);
        }
        let (commands, receiver) = RingBuffer::new(COMMANDS);
        let (garbage, returned) = RingBuffer::new(RETURNS);
        let shared = Arc::new(Shared::new());
        self.endpoints[index] = Some(Endpoint {
            commands,
            returned,
            shared: Arc::clone(&shared),
            leases: std::array::from_fn(|_| None),
            heartbeat: 0,
            last_seen: None,
            dirty: true,
            sent_revision: None,
        });
        self.flush();
        SoundpadTap {
            commands: receiver,
            garbage,
            shared,
            settings: Arc::clone(&self.outputs[index]),
            output,
            current: None,
            tail: None,
            deferred: std::array::from_fn(|_| None),
            revision: 0,
            gain_lin: 1.0,
            duck_gain: 1.0,
            residual: [0.0; 2],
            residual_left: 0,
            token: 0,
            position: 0,
            finished: true,
        }
    }
    pub fn play(&mut self, sound: Arc<Sound>, gain_lin: f32, start_ms: u32, looping: bool) -> u64 {
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.token = self.next_token;
        self.anchor_frame = ms_to_frames(start_ms).min(sound.frames());
        self.sound = Some(sound);
        self.anchor = Instant::now();
        self.gain_lin = clean_gain(gain_lin);
        self.paused = false;
        self.looping = looping;
        self.revision = self.revision.wrapping_add(1);
        self.changed();
        self.token
    }
    pub fn stop(&mut self) {
        self.sound = None;
        self.token = 0;
        self.anchor_frame = 0;
        self.paused = false;
        self.changed();
    }
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.anchor_frame = self.position(Instant::now());
            self.anchor = Instant::now();
            self.paused = paused;
            self.changed();
        }
    }
    pub fn seek(&mut self, position_ms: u32) {
        self.anchor_frame = self
            .sound
            .as_ref()
            .map_or(0, |s| ms_to_frames(position_ms).min(s.frames()));
        self.anchor = Instant::now();
        self.revision = self.revision.wrapping_add(1);
        self.changed();
    }
    pub fn set_looping(&mut self, looping: bool) {
        self.anchor_frame = self.position(Instant::now());
        self.anchor = Instant::now();
        self.looping = looping;
        self.changed();
    }
    pub fn set_sound_gain(&mut self, gain_lin: f32) {
        self.gain_lin = clean_gain(gain_lin);
        self.changed();
    }
    pub fn set_output_gain(&self, output: Output, gain_lin: f32) {
        self.outputs[output.index()]
            .gain
            .store(clean_gain(gain_lin).to_bits() as usize, Ordering::Release);
    }
    pub fn set_output_enabled(&self, output: Output, enabled: bool) {
        self.outputs[output.index()]
            .enabled
            .store(enabled, Ordering::Release);
    }
    pub fn set_duck_voice(&self, enabled: bool) {
        self.outputs[0].duck.store(enabled, Ordering::Release);
    }
    fn changed(&mut self) {
        for ep in self.endpoints.iter_mut().flatten() {
            ep.dirty = true;
        }
        self.flush();
    }
    fn wall_position(&self, now: Instant) -> usize {
        let Some(sound) = &self.sound else {
            return 0;
        };
        let elapsed = if self.paused {
            0
        } else {
            (now.saturating_duration_since(self.anchor).as_secs_f64() * f64::from(SAMPLE_RATE_HZ))
                as usize
        };
        let frame = self.anchor_frame.saturating_add(elapsed);
        if self.looping && sound.frames() > 0 {
            frame % sound.frames()
        } else {
            frame.min(sound.frames())
        }
    }
    fn position(&self, now: Instant) -> usize {
        self.endpoints
            .iter()
            .flatten()
            .filter(|ep| {
                ep.alive(now)
                    && ep.shared.token.load(Ordering::Acquire) == self.token
                    && ep.shared.revision.load(Ordering::Acquire) == self.revision
            })
            .map(|ep| ep.shared.position.load(Ordering::Acquire))
            .max()
            .unwrap_or_else(|| self.wall_position(now))
    }
    fn flush(&mut self) {
        let position = self.position(Instant::now());
        for ep in self.endpoints.iter_mut().flatten() {
            ep.reclaim();
            if !ep.dirty {
                continue;
            }
            let lease_index = ep.leases.iter().position(Option::is_none);
            if self.sound.is_some() && lease_index.is_none() {
                continue;
            }
            let lease_index = lease_index.unwrap_or(0);
            let command = Command {
                lease: lease_index,
                sound: self.sound.as_ref().map(Arc::clone),
                token: self.token,
                position: if ep
                    .sent_revision
                    .is_some_and(|revision| revision != self.revision)
                {
                    self.anchor_frame
                } else {
                    position
                },
                revision: self.revision,
                gain_lin: self.gain_lin,
                paused: self.paused,
                looping: self.looping,
            };
            // The extra lease guarantees even tap destruction cannot free PCM on an audio thread.
            let lease = command.sound.as_ref().map(Arc::clone);
            if ep.commands.push(command).is_ok() {
                if let Some(lease) = lease {
                    ep.leases[lease_index] = Some(lease);
                }
                ep.dirty = false;
                ep.sent_revision = Some(self.revision);
            }
        }
    }
    pub fn tick(&mut self, now: Instant) {
        for ep in self.endpoints.iter_mut().flatten() {
            ep.reclaim();
            let heartbeat = ep.shared.heartbeat.load(Ordering::Acquire);
            if heartbeat != ep.heartbeat {
                ep.heartbeat = heartbeat;
                ep.last_seen = Some(now);
            }
        }
        for ep in &mut self.retired {
            ep.reclaim();
        }
        self.retired.retain(|ep| Arc::strong_count(&ep.shared) > 1);
        if let Some(sound) = &self.sound {
            let mut alive = self
                .endpoints
                .iter()
                .flatten()
                .filter(|ep| ep.alive(now))
                .peekable();
            let ended = if alive.peek().is_some() {
                alive.all(|ep| {
                    ep.shared.token.load(Ordering::Acquire) == self.token
                        && ep.shared.revision.load(Ordering::Acquire) == self.revision
                        && ep.shared.finished.load(Ordering::Acquire)
                })
            } else {
                !self.paused && !self.looping && self.wall_position(now) >= sound.frames()
            };
            if ended {
                self.stop();
            }
        }
        self.flush();
    }
    pub fn status(&self) -> PlaybackStatus {
        PlaybackStatus {
            token: self.sound.as_ref().map(|_| self.token),
            position_ms: frames_to_ms(self.position(Instant::now())),
            duration_ms: self.sound.as_ref().map_or(0, |s| s.duration_ms()),
            paused: self.paused,
            looping: self.looping,
        }
    }
}
// If teardown happens before the audio streams stop, retain leases on a non-RT
// reaper until all taps are gone. Normal teardown (taps first) needs no thread.
impl Drop for SoundpadControl {
    fn drop(&mut self) {
        let mut endpoints = std::mem::take(&mut self.retired);
        endpoints.extend(self.endpoints.iter_mut().filter_map(Option::take));
        for ep in &endpoints {
            ep.shared.active.store(false, Ordering::Release);
        }
        if endpoints
            .iter()
            .all(|ep| Arc::strong_count(&ep.shared) == 1)
        {
            return;
        }
        let backup: Vec<_> = endpoints
            .iter()
            .flat_map(|ep| ep.leases.iter().flatten().map(Arc::clone))
            .collect();
        if std::thread::Builder::new()
            .name("macafon-soundpad-reaper".into())
            .spawn(move || {
                while endpoints.iter().any(|ep| Arc::strong_count(&ep.shared) > 1) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .is_err()
        {
            // OS thread exhaustion: leaking the leases is safer than freeing PCM on RT.
            std::mem::forget(backup);
        }
    }
}
struct Voice {
    lease: usize,
    sound: Arc<Sound>,
    last: [f32; 2],
    position: usize,
    gain_lin: f32,
    target_gain_lin: f32,
    envelope: f32,
    paused: bool,
    looping: bool,
}
impl Voice {
    fn new(
        sound: Arc<Sound>,
        lease: usize,
        position: usize,
        gain_lin: f32,
        paused: bool,
        looping: bool,
    ) -> Self {
        Self {
            lease,
            sound,
            last: [0.0; 2],
            position,
            gain_lin,
            target_gain_lin: gain_lin,
            envelope: 0.0,
            paused,
            looping,
        }
    }

    fn sample(&mut self, retiring: bool) -> [f32; 2] {
        let target = if retiring || self.paused { 0.0 } else { 1.0 };
        self.envelope = approach(self.envelope, target, 1.0 / FADE_FRAMES as f32);
        self.gain_lin = smooth(self.gain_lin, self.target_gain_lin);
        if self.envelope <= 0.0 && (self.paused || retiring) {
            return [0.0; 2];
        }
        if self.position >= self.sound.frames() {
            if self.looping && self.sound.frames() != 0 {
                self.position = 0;
            } else {
                return [0.0; 2];
            }
        }
        let offset = self.position * 2;
        let samples = self.sound.samples();
        let value = [
            samples.get(offset).copied().unwrap_or(0.0),
            samples.get(offset + 1).copied().unwrap_or(0.0),
        ];
        self.position += 1;
        self.last = value.map(|s| sanitize(s * self.gain_lin * self.envelope));
        self.last
    }
}
pub struct SoundpadTap {
    commands: Consumer<Command>,
    garbage: Producer<Returned>,
    shared: Arc<Shared>,
    settings: Arc<Settings>,
    output: Output,
    current: Option<Voice>,
    tail: Option<Voice>,
    deferred: [Option<Arc<Sound>>; RETURNS],
    revision: u64,
    gain_lin: f32,
    duck_gain: f32,
    residual: [f32; 2],
    residual_left: usize,
    token: u64,
    position: usize,
    finished: bool,
}
impl SoundpadTap {
    fn give_back(&mut self, sound: Arc<Sound>, lease: usize) {
        if let Err(rtrb::PushError::Full(returned)) = self.garbage.push(Returned { sound, lease }) {
            // This slot belongs exclusively to this reference until control receives
            // its return. It cannot be reused while the Arc is deferred here.
            self.deferred[lease] = Some(returned.sound);
        }
    }
    fn retire(&mut self) {
        if let Some(old) = self.tail.take() {
            let value = old.last;
            self.residual = std::array::from_fn(|i| self.residual[i] + value[i]);
            self.residual_left = FADE_FRAMES;
            self.give_back(old.sound, old.lease);
        }
        self.tail = self.current.take();
    }
    fn begin(&mut self) -> bool {
        if !self.shared.active.load(Ordering::Acquire) {
            return false;
        }
        for index in 0..RETURNS {
            if let Some(sound) = self.deferred[index].take() {
                if let Err(rtrb::PushError::Full(returned)) = self.garbage.push(Returned {
                    sound,
                    lease: index,
                }) {
                    self.deferred[index] = Some(returned.sound);
                    break;
                }
            }
        }
        for _ in 0..COMMANDS {
            let Ok(command) = self.commands.pop() else {
                break;
            };
            if command.token != self.token {
                self.retire();
                self.token = command.token;
                self.position = command.position;
                self.finished = false;
                self.current = command.sound.map(|sound| {
                    Voice::new(
                        sound,
                        command.lease,
                        command.position,
                        command.gain_lin,
                        command.paused,
                        command.looping,
                    )
                });
            } else {
                if let Some(sound) = command.sound {
                    if self.current.is_none() && command.revision != self.revision {
                        self.current = Some(Voice::new(
                            sound,
                            command.lease,
                            command.position,
                            command.gain_lin,
                            command.paused,
                            command.looping,
                        ));
                    } else {
                        self.give_back(sound, command.lease);
                    }
                }
                if command.revision != self.revision {
                    // Decay the previous sample while fading in the new seek position.
                    if let Some(current) = &mut self.current {
                        self.residual = current.last;
                        self.residual_left = FADE_FRAMES;
                        current.position = command.position;
                        current.envelope = 0.0;
                    }
                    self.position = command.position;
                    self.finished = false;
                }
            }
            if let Some(current) = &mut self.current {
                current.paused = command.paused;
                current.looping = command.looping;
                current.target_gain_lin = command.gain_lin;
            }
            self.revision = command.revision;
        }
        true
    }
    fn next(&mut self) -> ([f32; 2], f32) {
        let mut sound = [0.0; 2];
        let mut playing = false;
        if let Some(current) = &mut self.current {
            sound = current.sample(false);
            self.position = current.position;
            playing = !current.paused && current.position < current.sound.frames();
            if (current.sound.frames() == 0 || !current.looping)
                && current.position >= current.sound.frames()
            {
                if let Some(voice) = self.current.take() {
                    self.finished = true;
                    self.position = voice.sound.frames();
                    self.give_back(voice.sound, voice.lease);
                }
            }
        }
        if let Some(tail) = &mut self.tail {
            let value = tail.sample(true);
            sound[0] += value[0];
            sound[1] += value[1];
            if tail.envelope <= 0.0 || (!tail.looping && tail.position >= tail.sound.frames()) {
                if let Some(tail) = self.tail.take() {
                    self.give_back(tail.sound, tail.lease);
                }
            }
        }
        if self.residual_left > 0 {
            let factor = self.residual_left as f32 / FADE_FRAMES as f32;
            sound[0] += self.residual[0] * factor;
            sound[1] += self.residual[1] * factor;
            self.residual_left -= 1;
        }
        let enabled = self.settings.enabled.load(Ordering::Relaxed);
        let gain = if enabled {
            f32::from_bits(self.settings.gain.load(Ordering::Relaxed) as u32)
        } else {
            0.0
        };
        self.gain_lin = smooth(self.gain_lin, gain);
        let duck = self.output == Output::Microphone
            && self.settings.duck.load(Ordering::Relaxed)
            && playing;
        self.duck_gain = approach(
            self.duck_gain,
            if duck { 0.0 } else { 1.0 },
            1.0 / FADE_FRAMES as f32,
        );
        (
            sound.map(|s| {
                if enabled {
                    sanitize(s * self.gain_lin)
                } else {
                    0.0
                }
            }),
            self.duck_gain,
        )
    }
    fn publish(&self) {
        self.shared.token.store(0, Ordering::Release);
        self.shared.position.store(self.position, Ordering::Release);
        self.shared.finished.store(self.finished, Ordering::Release);
        self.shared.revision.store(self.revision, Ordering::Release);
        self.shared.token.store(self.token, Ordering::Release);
        self.shared.heartbeat.fetch_add(1, Ordering::Release);
    }
    pub fn mix(&mut self, out: &mut [f32], channels: usize) {
        if !self.begin() {
            return;
        }
        if channels != 0 {
            for frame in out.chunks_exact_mut(channels) {
                let (sound, _) = self.next();
                if channels == 1 {
                    frame[0] = sanitize(frame[0] + (sound[0] + sound[1]) * 0.5);
                } else {
                    frame[0] = sanitize(frame[0] + sound[0]);
                    frame[1] = sanitize(frame[1] + sound[1]);
                }
            }
        }
        self.publish();
    }
    pub fn mix_with_voice(&mut self, voice: &[f32], out: &mut [f32], channels: usize) {
        let active = self.begin();
        if channels != 0 {
            for (index, frame) in out.chunks_exact_mut(channels).enumerate() {
                let (sound, duck) = if active { self.next() } else { ([0.0; 2], 1.0) };
                let input = sanitize(voice.get(index).copied().unwrap_or(0.0)) * duck;
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let added = if channels == 1 {
                        (sound[0] + sound[1]) * 0.5
                    } else {
                        sound[channel % 2]
                    };
                    *sample = sanitize(input + added).clamp(-1.0, 1.0);
                }
            }
        }
        if active {
            self.publish();
        }
    }
}
fn clean_gain(gain_lin: f32) -> f32 {
    sanitize(gain_lin).max(0.0)
}
fn smooth(value: f32, target: f32) -> f32 {
    sanitize(value + (target - value) / GAIN_FRAMES)
}
fn approach(value: f32, target: f32, step: f32) -> f32 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}
fn ms_to_frames(ms: u32) -> usize {
    (u64::from(ms) * u64::from(SAMPLE_RATE_HZ) / 1000) as usize
}
fn frames_to_ms(frames: usize) -> u32 {
    (frames as u64 * 1000 / u64::from(SAMPLE_RATE_HZ)).min(u64::from(u32::MAX)) as u32
}

impl Drop for SoundpadTap {
    fn drop(&mut self) {
        // Return every PCM reference before Shared's final tap reference disappears.
        // Control-side leases also protect references still queued in either ring.
        if let Some(voice) = self.current.take() {
            self.give_back(voice.sound, voice.lease);
        }
        if let Some(voice) = self.tail.take() {
            self.give_back(voice.sound, voice.lease);
        }
        while let Ok(command) = self.commands.pop() {
            if let Some(sound) = command.sound {
                self.give_back(sound, command.lease);
            }
        }
        for index in 0..RETURNS {
            if let Some(sound) = self.deferred[index].take() {
                // The control-side lease remains until the endpoint is reclaimed.
                drop(sound);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    fn constant(value: f32, frames: usize) -> Arc<Sound> {
        Arc::new(Sound::from_interleaved_stereo(vec![value; frames * 2]))
    }
    fn render(tap: &mut SoundpadTap, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        assert_no_alloc(|| tap.mix(&mut out, 2));
        out
    }
    #[test]
    fn replace_stop_and_garbage_return_are_allocation_free() {
        let mut control = SoundpadControl::new();
        let mut tap = control.tap(Output::Monitor);
        let old = constant(0.5, 48_000);
        let weak = Arc::downgrade(&old);
        control.play(old, 1.0, 0, false);
        let first = render(&mut tap, 512);
        assert!((first[1022] - 0.5).abs() < 0.001);
        control.play(constant(-0.5, 48_000), 1.0, 0, false);
        let replaced = render(&mut tap, 512);
        let mut previous = first[1022];
        for frame in replaced.chunks_exact(2) {
            assert!((frame[0] - previous).abs() < 0.02);
            previous = frame[0];
        }
        assert!(weak.upgrade().is_some());
        control.tick(Instant::now());
        assert!(weak.upgrade().is_none());
        control.stop();
        let stopped = render(&mut tap, 512);
        for frame in stopped.chunks_exact(2) {
            assert!((frame[0] - previous).abs() < 0.02);
            previous = frame[0];
        }
        assert_eq!(stopped[1022], 0.0);
        control.tick(Instant::now());
    }
    #[test]
    fn independent_clocks_wait_for_both_outputs() {
        let mut control = SoundpadControl::new();
        let mut mic = control.tap(Output::Microphone);
        let mut monitor = control.tap(Output::Monitor);
        let sound = constant(0.25, 4800);
        let weak = Arc::downgrade(&sound);
        let token = control.play(sound, 1.0, 0, false);
        let mut mic_out = [0.0; 254];
        let mut monitor_out = [0.0; 882];
        assert_no_alloc(|| {
            mic.mix(&mut mic_out, 2);
            monitor.mix(&mut monitor_out, 2);
        });
        control.tick(Instant::now());
        for _ in 0..40 {
            assert_no_alloc(|| mic.mix(&mut mic_out, 2));
        }
        control.tick(Instant::now());
        assert_eq!(control.status().token, Some(token));
        assert_eq!(control.status().position_ms, 100);
        for _ in 0..11 {
            assert_no_alloc(|| monitor.mix(&mut monitor_out, 2));
        }
        control.tick(Instant::now());
        assert_eq!(control.status().token, None);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn loop_pause_resume_seek_and_disabled_output_keep_position() {
        let mut control = SoundpadControl::new();
        let mut tap = control.tap(Output::Monitor);
        control.play(constant(0.25, 480), 1.0, 0, true);
        let out = render(&mut tap, 2000);
        assert!(out[960..].iter().all(|s| (*s - 0.25).abs() < 0.001));
        control.set_paused(true);
        let out = render(&mut tap, 512);
        assert_eq!(out[1022], 0.0);
        let held = tap.position;
        render(&mut tap, 512);
        assert_eq!(tap.position, held);
        control.set_paused(false);
        let out = render(&mut tap, 512);
        assert!(out[1022] > 0.24);
        control.set_output_enabled(Output::Monitor, false);
        let before = tap.position;
        let out = render(&mut tap, 127);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_ne!(tap.position, before);
        control.set_output_enabled(Output::Monitor, true);
        control.seek(5);
        render(&mut tap, 1);
        assert!((240..250).contains(&tap.position));
        control.set_looping(false);
        render(&mut tap, 600);
        assert!(tap.finished);
        // Seeking must revive an already-finished tap, and a stale finish cannot stop it.
        control.tick(Instant::now());
        control.play(constant(0.25, 480), 1.0, 0, false);
        render(&mut tap, 600);
        control.seek(0);
        control.tick(Instant::now());
        assert!(control.status().token.is_some());
        let out = render(&mut tap, 300);
        assert!(out[598] > 0.24);
    }
    #[test]
    fn voice_duck_clamp_sanitize_and_channel_mapping() {
        let mut control = SoundpadControl::new();
        let mut tap = control.tap(Output::Microphone);
        control.play(constant(0.25, 48_000), 1.0, 0, false);
        control.set_duck_voice(true);
        let voice = [0.5; 512];
        let mut out = [0.0; 512 * 4];
        assert_no_alloc(|| tap.mix_with_voice(&voice, &mut out, 4));
        assert!(out.chunks_exact(4).all(|f| f.iter().all(|s| *s == f[0])));
        assert!((out[out.len() - 1] - 0.25).abs() < 0.001);
        control.set_duck_voice(false);
        control.set_sound_gain(10.0);
        assert_no_alloc(|| tap.mix_with_voice(&voice, &mut out, 4));
        assert!(out.iter().all(|s| s.is_finite() && *s <= 1.0));
        let invalid = [f32::NAN, f32::INFINITY];
        let mut sanitized = [0.0; 2];
        control.stop();
        render(&mut tap, 512);
        assert_no_alloc(|| tap.mix_with_voice(&invalid, &mut sanitized, 1));
        assert_eq!(sanitized, [0.0; 2]);
        // Stereo monitor adds only into the first two channels; mono averages L/R.
        let mut monitor = control.tap(Output::Monitor);
        control.play(
            Arc::new(Sound::from_interleaved_stereo([0.2, 0.6].repeat(1000))),
            1.0,
            0,
            true,
        );
        render(&mut monitor, 300);
        let mut multichannel = [0.1; 8];
        assert_no_alloc(|| monitor.mix(&mut multichannel, 4));
        assert!((multichannel[0] - 0.3).abs() < 0.001);
        assert!((multichannel[1] - 0.7).abs() < 0.001);
        assert_eq!(multichannel[2], 0.1);
        let mut mono = [0.1];
        assert_no_alloc(|| monitor.mix(&mut mono, 1));
        assert!((mono[0] - 0.5).abs() < 0.001);
        assert_no_alloc(|| {
            monitor.mix(&mut mono, 0);
            monitor.mix_with_voice(&[], &mut mono, 0);
        });
    }
    #[test]
    fn reconnect_and_command_flood_preserve_rt_ownership() {
        let mut control = SoundpadControl::new();
        let mut old = control.tap(Output::Monitor);
        control.play(constant(0.2, 4800), 1.0, 0, true);
        render(&mut old, 500);
        let mut tap = control.tap(Output::Monitor);
        let mut silence = [0.0; 64];
        assert_no_alloc(|| old.mix(&mut silence, 2));
        assert_eq!(silence, [0.0; 64]);
        assert!(render(&mut tap, 500)[998] > 0.19);
        let mut weak = Vec::new();
        for round in 0..20 {
            for _ in 0..100 {
                let sound = constant(0.2, 480);
                weak.push(Arc::downgrade(&sound));
                control.play(sound, 1.0, 0, false);
            }
            render(&mut tap, 512);
            control.tick(Instant::now());
            if round % 3 == 0 {
                let new = control.tap(Output::Monitor);
                drop(tap);
                tap = new;
                control.tick(Instant::now());
            }
        }
        drop(old);
        control.stop();
        render(&mut tap, 1000);
        control.tick(Instant::now());
        assert!(weak.iter().all(|s| s.upgrade().is_none()));
    }
    #[test]
    fn wall_clock_fallback_honors_pause_seek_and_dead_taps() {
        let mut control = SoundpadControl::new();
        control.play(constant(0.2, 48_000), 1.0, 0, false);
        control.set_paused(true);
        let now = control.anchor;
        control.tick(now + Duration::from_secs(10));
        assert!(control.status().token.is_some());
        control.seek(900);
        control.set_paused(false);
        control.tick(control.anchor + Duration::from_millis(101));
        assert!(control.status().token.is_none());
        let mut tap = control.tap(Output::Monitor);
        control.play(constant(0.2, 4800), 1.0, 0, false);
        render(&mut tap, 10);
        let now = control.anchor;
        control.tick(now);
        control.tick(now + Duration::from_millis(301));
        assert!(control.status().token.is_none());
    }
}
