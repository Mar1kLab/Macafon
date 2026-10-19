//! Optional microphone tap. Ownership is transferred and retired outside the audio callback.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const BUFFER_SAMPLES: usize = 96_000;
const PRE_ROLL_SAMPLES: usize = 14_400;

pub struct DictationReader {
    samples: rtrb::Consumer<f32>,
    monitoring: Arc<AtomicBool>,
    recording: Arc<AtomicBool>,
    overrun: Arc<AtomicBool>,
}
pub(crate) struct DictationTap {
    monitoring: Arc<AtomicBool>,
    pub(crate) point: macafon_dsp::TapPoint,
    samples: rtrb::Producer<f32>,
    recording: Arc<AtomicBool>,
    overrun: Arc<AtomicBool>,
    pre_roll: Box<[f32]>,
    position: usize,
    filled: usize,
    was_recording: bool,
}

pub(crate) fn tap(
    pre_roll: bool,
    point: macafon_dsp::TapPoint,
) -> (DictationReader, Box<DictationTap>) {
    let (producer, consumer) = rtrb::RingBuffer::new(BUFFER_SAMPLES);
    let monitoring = Arc::new(AtomicBool::new(false));
    let recording = Arc::new(AtomicBool::new(false));
    let overrun = Arc::new(AtomicBool::new(false));
    (
        DictationReader {
            samples: consumer,
            monitoring: monitoring.clone(),
            recording: recording.clone(),
            overrun: overrun.clone(),
        },
        Box::new(DictationTap {
            point,
            samples: producer,
            monitoring,
            recording,
            overrun,
            pre_roll: vec![0.0; if pre_roll { PRE_ROLL_SAMPLES } else { 0 }].into_boxed_slice(),
            position: 0,
            filled: 0,
            was_recording: false,
        }),
    )
}

impl DictationReader {
    pub fn set_monitoring(&self, monitoring: bool) {
        self.monitoring.store(monitoring, Ordering::Release);
    }
    pub fn set_recording(&self, recording: bool) {
        self.recording.store(recording, Ordering::Release);
    }
    pub fn pop(&mut self) -> Option<f32> {
        self.samples.pop().ok()
    }
    pub fn take_overrun(&self) -> bool {
        self.overrun.swap(false, Ordering::Relaxed)
    }
}
impl DictationTap {
    /// Copies only into prepared storage. Full queues drop samples and report an overrun.
    pub(crate) fn process(&mut self, input: &[f32]) {
        let recording = self.recording.load(Ordering::Acquire);
        if recording || self.monitoring.load(Ordering::Acquire) {
            if recording && !self.was_recording && !self.pre_roll.is_empty() {
                let start =
                    (self.position + self.pre_roll.len() - self.filled) % self.pre_roll.len();
                for n in 0..self.filled {
                    let sample = self.pre_roll[(start + n) % self.pre_roll.len()];
                    if self.samples.push(sample).is_err() {
                        self.overrun.store(true, Ordering::Relaxed);
                    }
                }
            }
            for &sample in input {
                if self
                    .samples
                    .push(if sample.is_finite() { sample } else { 0.0 })
                    .is_err()
                {
                    self.overrun.store(true, Ordering::Relaxed);
                }
            }
        }
        if !self.pre_roll.is_empty() {
            for &sample in input {
                self.pre_roll[self.position] = if sample.is_finite() { sample } else { 0.0 };
                self.position = (self.position + 1) % self.pre_roll.len();
            }
            self.filled = (self.filled + input.len()).min(self.pre_roll.len());
        }
        self.was_recording = recording;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::assert_no_alloc;
    #[test]
    fn tap_pre_roll_record_overflow_and_stop_allocate_nothing() {
        let (mut reader, mut tap) = tap(true, macafon_dsp::TapPoint::Clean);
        assert_no_alloc(|| tap.process(&[0.25; PRE_ROLL_SAMPLES + 128]));
        assert!(reader.pop().is_none());
        reader.set_recording(true);
        assert_no_alloc(|| tap.process(&[0.5; 128]));
        for _ in 0..PRE_ROLL_SAMPLES {
            assert_eq!(reader.pop(), Some(0.25));
        }
        for _ in 0..128 {
            assert_eq!(reader.pop(), Some(0.5));
        }
        assert_no_alloc(|| tap.process(&[0.5; BUFFER_SAMPLES + 1]));
        assert!(reader.take_overrun());
        while reader.pop().is_some() {}
        reader.set_recording(false);
        assert_no_alloc(|| tap.process(&[0.75; 128]));
        assert!(reader.pop().is_none());

        // Exercise queued ownership transfer and the independent virtual-microphone gate.
        let mut settings = macafon_dsp::ChainSettings::raw();
        settings.muted = true;
        let (mut control, mut rt) = crate::microphone(&settings, Arc::default()).expect("engine");
        let mut reader = control.enable_dictation(false, macafon_dsp::TapPoint::Clean).expect("tap");
        reader.set_recording(true);
        control.mute_for_dictation(true);
        let mut block = [0.25;128];
        assert_no_alloc(|| rt.process(&mut block));
        assert!(block.iter().all(|x| *x == 0.0));
        assert_eq!(reader.pop(), Some(0.25));
        control.disable_dictation().expect("detach");
        assert_no_alloc(|| rt.process(&mut block));
        assert_no_alloc(|| rt.process(&mut block));
        control.collect_garbage();
        assert!(control.settings().muted, "existing mute is preserved");
    }
}
