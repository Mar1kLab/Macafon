//! Lock-free control of one output channel (PRD §7.3, §11.4). Same scheme as the
//! microphone: retunes travel through an SPSC queue, structural changes build a processor off
//! the RT thread that `ChannelRt` crossfades in; replaced processors return for freeing.

use crate::{EngineError, GARBAGE_CAPACITY, QUEUE_CAPACITY, SCRATCH_SAMPLES, SWAP_FADE_SAMPLES};
use macafon_dsp::{ChannelProcessor, ChannelSettings, ChannelTelemetry};
use std::sync::Arc;

#[allow(clippy::large_enum_variant)] // inline settings: popping must not free memory on RT
enum Command {
    Retune(ChannelSettings, f32),
    Replace(Box<ChannelProcessor>),
}

/// Create a connected control/RT pair for one channel. `master_gain` is the linear Master
/// fader including its mute (CH-006).
pub fn channel(
    settings: &ChannelSettings,
    master_gain: f32,
    telemetry: Arc<ChannelTelemetry>,
) -> Result<(ChannelControl, ChannelRt), EngineError> {
    let processor = ChannelProcessor::new(settings, master_gain, telemetry.clone())?;
    let (commands, command_queue) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
    let (garbage, garbage_queue) = rtrb::RingBuffer::new(GARBAGE_CAPACITY);
    Ok((
        ChannelControl {
            settings: *settings,
            master_gain,
            commands,
            garbage: garbage_queue,
            telemetry,
        },
        ChannelRt {
            current: Box::new(processor),
            outgoing: None,
            retired: None,
            fade_position: 0,
            scratch_left: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
            scratch_right: vec![0.0; SCRATCH_SAMPLES].into_boxed_slice(),
            commands: command_queue,
            garbage,
        },
    ))
}

pub struct ChannelControl {
    settings: ChannelSettings,
    master_gain: f32,
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Box<ChannelProcessor>>,
    telemetry: Arc<ChannelTelemetry>,
}

impl ChannelControl {
    pub fn settings(&self) -> &ChannelSettings {
        &self.settings
    }

    pub fn telemetry(&self) -> Arc<ChannelTelemetry> {
        self.telemetry.clone()
    }

    pub fn latency_samples(&self) -> u32 {
        self.settings.latency_samples()
    }

    /// Send new settings: retuned in place, or rebuilt and crossfaded.
    pub fn apply(
        &mut self,
        settings: &ChannelSettings,
        master_gain: f32,
    ) -> Result<(), EngineError> {
        settings.validate()?;
        self.collect_garbage();
        if self.commands.is_full() {
            return Err(EngineError::Busy);
        }
        let command = if self.settings.requires_rebuild(settings) {
            Command::Replace(Box::new(ChannelProcessor::new(
                settings,
                master_gain,
                self.telemetry.clone(),
            )?))
        } else {
            Command::Retune(*settings, master_gain)
        };
        self.commands.push(command).map_err(|_| EngineError::Busy)?;
        self.settings = *settings;
        self.master_gain = master_gain;
        Ok(())
    }

    pub fn set_master(&mut self, master_gain: f32) -> Result<(), EngineError> {
        if master_gain == self.master_gain {
            return Ok(());
        }
        let settings = self.settings;
        self.apply(&settings, master_gain)
    }

    pub fn collect_garbage(&mut self) {
        while let Ok(processor) = self.garbage.pop() {
            drop(processor);
        }
    }
}

pub struct ChannelRt {
    current: Box<ChannelProcessor>,
    outgoing: Option<Box<ChannelProcessor>>,
    retired: Option<Box<ChannelProcessor>>,
    fade_position: u32,
    scratch_left: Box<[f32]>,
    scratch_right: Box<[f32]>,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Box<ChannelProcessor>>,
}

impl ChannelRt {
    pub fn latency_samples(&self) -> u32 {
        self.current.latency_samples()
    }

    /// RT-safe: no allocation, locks or deallocation.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.drain();
        if self.outgoing.is_none() {
            self.current.process(left, right);
            return;
        }
        let len = left.len().min(right.len());
        for (l, r) in left[..len]
            .chunks_mut(SCRATCH_SAMPLES)
            .zip(right[..len].chunks_mut(SCRATCH_SAMPLES))
        {
            let Some(old) = &mut self.outgoing else {
                self.current.process(l, r);
                continue;
            };
            let previous_l = &mut self.scratch_left[..l.len()];
            let previous_r = &mut self.scratch_right[..r.len()];
            previous_l.copy_from_slice(l);
            previous_r.copy_from_slice(r);
            old.process(previous_l, previous_r);
            self.current.process(l, r);
            for ((new_l, new_r), (old_l, old_r)) in l
                .iter_mut()
                .zip(r.iter_mut())
                .zip(previous_l.iter().zip(previous_r.iter()))
            {
                let weight = (self.fade_position as f32 / SWAP_FADE_SAMPLES as f32).min(1.0);
                *new_l = old_l + weight * (*new_l - old_l);
                *new_r = old_r + weight * (*new_r - old_r);
                self.fade_position = self.fade_position.saturating_add(1);
            }
            if self.fade_position >= SWAP_FADE_SAMPLES {
                self.retired = self.outgoing.take();
                self.release_retired();
            }
        }
    }

    fn release_retired(&mut self) {
        if let Some(processor) = self.retired.take() {
            if let Err(rtrb::PushError::Full(processor)) = self.garbage.push(processor) {
                self.retired = Some(processor);
            }
        }
    }

    fn drain(&mut self) {
        self.release_retired();
        loop {
            let busy = self.outgoing.is_some() || self.retired.is_some();
            match self.commands.peek() {
                Ok(Command::Replace(_)) if busy => return,
                Ok(_) => {}
                Err(_) => return,
            }
            match self.commands.pop() {
                Ok(Command::Retune(settings, master_gain)) => {
                    // The control side only sends retunes that keep the topology.
                    let _ = self.current.retune(&settings, master_gain);
                }
                Ok(Command::Replace(processor)) => {
                    self.outgoing = Some(std::mem::replace(&mut self.current, processor));
                    self.fade_position = 0;
                }
                Err(_) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    use macafon_dsp::channel::ChannelTemplate;

    #[test]
    fn retune_and_rebuild_reach_the_audio_thread_without_allocation() {
        let main = ChannelSettings::template(ChannelTemplate::Main);
        let (mut control, mut rt) = channel(&main, 1.0, Arc::default()).expect("valid channel");
        let mut left = [0.5f32; 128];
        let mut right = [0.5f32; 128];
        let mut louder = main;
        louder.volume_pct = 100.0;
        control.apply(&louder, 1.0).expect("retune queued");
        for _ in 0..8 {
            left.fill(0.5);
            right.fill(0.5);
            assert_no_alloc(|| rt.process(&mut left, &mut right));
        }
        assert_eq!(left[127], 0.5);
        // Turning the limiter on rebuilds the processor; the RT side crossfades into it.
        let mut limited = louder;
        limited.limiter.enabled = true;
        control.apply(&limited, 0.5).expect("rebuild queued");
        for _ in 0..8 {
            left.fill(0.5);
            right.fill(0.5);
            assert_no_alloc(|| rt.process(&mut left, &mut right));
        }
        assert!((left[127] - 0.25).abs() < 1e-6, "{}", left[127]);
        assert_eq!(left, right);
        control.collect_garbage();
        control.set_master(0.0).expect("master queued");
        assert_no_alloc(|| rt.process(&mut left, &mut right));
        assert_no_alloc(|| rt.process(&mut left, &mut right));
        assert_no_alloc(|| rt.process(&mut left, &mut right));
        assert_no_alloc(|| rt.process(&mut left, &mut right));
        assert_eq!(left[127], 0.0);
    }
}
