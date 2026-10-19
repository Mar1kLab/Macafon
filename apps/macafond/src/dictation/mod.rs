//! Daemon-owned dictation state. Inference is isolated in the worker process.
pub mod models;
mod target;
mod worker;

use macafon_ipc::*;
use macafon_stt::protocol::{Language, LanguageInput, LanguagePolicy, WorkerEvent, WorkerRequest};
use std::time::Instant;

pub struct Dictation {
    pub models: models::Manager,
    pub phase: DictationPhase,
    pub error: Option<String>,
    pub history: Vec<DictationResult>,
    pub last: Option<DictationResult>,
    pub worker: Option<worker::Worker>,
    pub loaded_model: Option<String>,
    /// The running worker was loaded in efficiency mode (EcoQoS, half the threads, CPU).
    pub worker_efficiency: bool,
    pub ram: Option<u64>,
    pub threads: Option<u16>,
    pub default_threads: u16,
    pub logical_cores: u16,
    pub started: Option<Instant>,
    pub audio_elapsed_s: u32,
    pub last_used: Instant,
    pub last_cyrillic: Option<Language>,
    pub language: LanguageInput,
    pub test: bool,
    pub copy_only: bool,
    pub target: target::Target,
    pub game: bool,
    pub hands_free: bool,
    /// The last short tap of a hold shortcut (released before the hold delay): a second tap
    /// soon after starts hands-free dictation.
    pub last_press: Option<(Instant, bool)>,
    /// A hold shortcut pressed, not yet held long enough to start (STT-002); `bool` = copy only.
    pub pending_hold: Option<(Instant, bool)>,
    pub audio: Option<Audio>,
    pub popup_state: crate::osd::dictation::State,
    next_id: u64,
}

pub struct Audio {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub commands: std::sync::mpsc::Sender<AudioCommand>,
    pub recording: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub levels: std::sync::Arc<std::sync::Mutex<(DictationLevel, [f32; 15])>>,
}

impl Drop for Dictation {
    fn drop(&mut self) {
        target::finish_restores();
    }
}
pub enum AudioCommand {
    Begin(std::sync::mpsc::SyncSender<worker::Input>),
    Finish(WorkerRequest),
    Cancel,
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Dictation {
    pub fn new(config: &macafon_config::dictation::DictationConfig) -> Self {
        let logical_cores = num_cpus::get().clamp(1, u16::MAX as usize) as u16;
        let default_threads = (num_cpus::get_physical() / 2)
            .max(2)
            .min(logical_cores as usize) as u16;
        Self {
            models: models::Manager::new(&config.models_dir),
            phase: if config.enabled && cfg!(windows) {
                DictationPhase::Unloaded
            } else {
                DictationPhase::Off
            },
            error: None,
            history: Vec::new(),
            last: None,
            worker: None,
            loaded_model: None,
            worker_efficiency: false,
            ram: None,
            threads: None,
            logical_cores,
            default_threads,
            started: None,
            audio_elapsed_s: 0,
            last_used: Instant::now(),
            last_cyrillic: None,
            language: LanguageInput {
                policy: LanguagePolicy::Layout,
                fixed: Language::Uk,
                layout: None,
                last_cyrillic: None,
            },
            test: false,
            copy_only: false,
            target: target::Target::default(),
            game: false,
            hands_free: false,
            last_press: None,
            pending_hold: None,
            audio: None,
            next_id: 1,
            popup_state: crate::osd::dictation::State::Listening,
        }
    }
    pub fn state(&self, prefs: DictationPrefs) -> DictationState {
        DictationState {
            prefs,
            phase: self.phase,
            error: self.error.clone(),
            loaded_model: self.loaded_model.clone(),
            worker_ram_bytes: self.ram,
            worker_threads: self.threads,
            default_threads: self.default_threads,
            logical_cores: self.logical_cores,
            models: self.models.models.clone(),
            storage: self.models.storage.clone(),
            history: self.history.clone(),
        }
    }
    pub fn unload(&mut self, enabled: bool) {
        self.worker = None;
        self.loaded_model = None;
        self.ram = None;
        self.threads = None;
        self.started = None;
        self.phase = if enabled {
            DictationPhase::Unloaded
        } else {
            DictationPhase::Off
        };
    }
    pub fn begin(
        &mut self,
        prefs: &DictationPrefs,
        test: bool,
        copy_only: bool,
        hands_free: bool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(prefs.enabled, "dictation is disabled");
        anyhow::ensure!(!self.models.busy(), "model operation is in progress");
        anyhow::ensure!(
            !matches!(
                self.phase,
                DictationPhase::Listening | DictationPhase::Transcribing
            ),
            "dictation is busy"
        );
        let id = prefs
            .model
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("download and select a model first"))?;
        anyhow::ensure!(
            self.models.ready(id),
            "download and verify the selected model first"
        );
        let spec = models::spec(id)?;
        let audio = self
            .audio
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("microphone is not connected"))?;
        self.target = target::Target::capture();
        self.game = target::game_active();
        self.language = LanguageInput {
            policy: match prefs.language {
                DictationLanguagePolicy::Layout => LanguagePolicy::Layout,
                DictationLanguagePolicy::Fixed => LanguagePolicy::Fixed,
                DictationLanguagePolicy::Auto => LanguagePolicy::Auto,
            },
            fixed: language(prefs.fixed_language),
            layout: self.target.layout,
            last_cyrillic: self.last_cyrillic,
        };
        // EcoQoS parks the worker on efficiency cores at low clocks (13 s instead of 1 s for
        // Parakeet), so it is only for a full-screen game with game mode on; switching modes
        // reloads the worker.
        let efficiency = self.game && prefs.game_mode;
        if self.worker.is_some() && self.worker_efficiency != efficiency {
            self.worker = None;
            self.loaded_model = None;
        }
        if self.worker.is_none() {
            let worker = worker::Worker::spawn()?;
            worker.send(WorkerRequest::Load {
                engine: spec.engine,
                model_dir: macafon_stt::manifest::model_dir(&self.models.folder, spec),
                model_id: id.into(),
                // The worker halves this budget itself in efficiency mode.
                threads: if prefs.threads == 0 {
                    self.default_threads
                } else {
                    prefs.threads
                },
                gpu: false,
                efficiency,
            })?;
            self.worker = Some(worker);
            self.worker_efficiency = efficiency;
            self.loaded_model = Some(id.into());
        }
        let worker = self
            .worker
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("worker unavailable"))?;
        worker.send(WorkerRequest::Begin)?;
        audio
            .commands
            .send(AudioCommand::Begin(worker.input.clone()))?;

        self.audio_elapsed_s = 0;
        self.started = Some(Instant::now());
        self.test = test;
        self.copy_only = copy_only;
        self.hands_free = hands_free;
        self.error = None;
        self.popup_state = crate::osd::dictation::State::Listening;
        self.phase = DictationPhase::Listening;
        Ok(())
    }
    pub fn finish(&mut self, prefs: &DictationPrefs) -> anyhow::Result<()> {
        let duration = self.started.take().map(|started| started.elapsed());
        self.audio_elapsed_s = duration.map_or(0, |d| d.as_secs() as u32);
        if duration.is_none_or(|d| d.as_millis() < 300) {
            self.cancel(prefs.enabled);
            return Ok(());
        }
        if let Some(audio) = &self.audio {
            audio
                .recording
                .store(false, std::sync::atomic::Ordering::Release);
            audio
                .commands
                .send(AudioCommand::Finish(WorkerRequest::Finish {
                    language: self.language.clone(),
                    vocabulary: prefs.vocabulary.clone(),
                }))?;
        }
        self.popup_state = crate::osd::dictation::State::Transcribing;
        self.phase = DictationPhase::Transcribing;
        self.last_used = Instant::now();
        Ok(())
    }
    pub fn cancel(&mut self, enabled: bool) {
        self.popup_state = crate::osd::dictation::State::Cancelled;
        if let Some(audio) = &self.audio {
            audio
                .recording
                .store(false, std::sync::atomic::Ordering::Release);
            let _ = audio.commands.send(AudioCommand::Cancel);
        }
        // Retire the process too: a cancelled inference must never insert a late result.
        self.unload(enabled);
    }
    pub fn poll(&mut self, prefs: &DictationPrefs) -> (bool, Option<DictationResult>) {
        let mut changed = false;
        let mut result = None;
        let events: Vec<_> = self
            .worker
            .as_ref()
            .map(|w| w.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            changed = true;
            match event {
                Ok(WorkerEvent::Loading) => {}
                Ok(WorkerEvent::Ready { ram_bytes, threads }) => {
                    self.ram = Some(ram_bytes);
                    self.threads = Some(threads);
                    if matches!(
                        self.phase,
                        DictationPhase::Unloaded | DictationPhase::Loading
                    ) {
                        self.phase = DictationPhase::Ready;
                    }
                }
                Ok(WorkerEvent::Result(transcript))
                    if self.phase == DictationPhase::Transcribing =>
                {
                    let text = transcript
                        .text
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.popup_state = crate::osd::dictation::State::NoSpeech;
                    if self.game
                        && prefs.fullscreen_games == DictationGamePolicy::Ignore
                        && !self.test
                    {
                        self.popup_state = crate::osd::dictation::State::Cancelled;
                        self.phase = DictationPhase::Ready;
                        self.last_used = Instant::now();
                        continue;
                    }
                    if !text.is_empty() {
                        let outcome = if self.test {
                            DictationOutcome::Test
                        } else {
                            match target::insert(
                                &self.target,
                                &text,
                                prefs,
                                self.copy_only,
                                self.game,
                            ) {
                                Ok(outcome) => outcome,
                                Err(_) => {
                                    self.popup_state = crate::osd::dictation::State::Error;
                                    self.error = Some("text insertion failed".into());
                                    self.phase = DictationPhase::Ready;
                                    self.last_used = Instant::now();
                                    continue;
                                }
                            }
                        };
                        self.popup_state = if outcome == DictationOutcome::Copied {
                            crate::osd::dictation::State::Copied
                        } else {
                            crate::osd::dictation::State::Done
                        };
                        let language = transcript.language.map(ipc_language);
                        if transcript.language.is_some_and(Language::is_cyrillic) {
                            self.last_cyrillic = transcript.language;
                        }
                        let entry = DictationResult {
                            id: self.next_id,
                            text,
                            language,
                            engine: prefs.engine,
                            model: self.loaded_model.clone().unwrap_or_default(),
                            audio_ms: transcript.audio_ms,
                            elapsed_ms: transcript.elapsed_ms,
                            at_unix_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64,
                            outcome,
                        };
                        self.next_id += 1;
                        self.last = Some(entry.clone());
                        if prefs.keep_history {
                            self.history.insert(0, entry.clone());
                            self.history.truncate(20);
                        }
                        result = Some(entry);
                    }
                    self.phase = DictationPhase::Ready;
                    self.last_used = Instant::now();
                }
                Ok(WorkerEvent::NoSpeech { .. }) => {
                    self.popup_state = crate::osd::dictation::State::NoSpeech;
                    self.phase = DictationPhase::Ready;
                    self.last_used = Instant::now();
                }
                Ok(WorkerEvent::Error { .. }) | Err(_) => {
                    // Never surface inference messages: external engines may include text.
                    self.cancel(prefs.enabled);
                    self.popup_state = crate::osd::dictation::State::Error;
                    self.phase = DictationPhase::Error;
                    self.error = Some("dictation worker failed".into());
                }
                _ => {}
            }
        }
        if self.phase == DictationPhase::Ready
            && ((self.game && prefs.game_mode)
                || (prefs.unload_after_s >= 0
                    && self.last_used.elapsed().as_secs() >= prefs.unload_after_s as u64))
        {
            self.unload(prefs.enabled);
            changed = true;
        }
        (changed, result)
    }
    pub fn reinsert(&self, id: u64, prefs: &DictationPrefs) -> anyhow::Result<()> {
        let entry = self
            .history
            .iter()
            .chain(self.last.iter())
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow::anyhow!("history entry not found"))?;
        target::insert(&self.target, &entry.text, prefs, false, self.game)?;
        Ok(())
    }
    pub fn last_action(&self, prefs: &DictationPrefs, copy: bool) -> anyhow::Result<()> {
        let entry = self
            .last
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no previous dictation"))?;
        target::insert(
            &target::Target::capture(),
            &entry.text,
            prefs,
            copy,
            target::game_active(),
        )?;
        Ok(())
    }
}
fn language(value: DictationLanguage) -> Language {
    match value {
        DictationLanguage::En => Language::En,
        DictationLanguage::Uk => Language::Uk,
        DictationLanguage::Ru => Language::Ru,
    }
}
fn ipc_language(value: Language) -> DictationLanguage {
    match value {
        Language::En => DictationLanguage::En,
        Language::Uk => DictationLanguage::Uk,
        Language::Ru => DictationLanguage::Ru,
    }
}

pub fn audio(
    reader: macafon_engine::DictationReader,
    hub: std::sync::Arc<crate::server::Hub>,
) -> anyhow::Result<Audio> {
    use rubato::Resampler;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let recording = Arc::new(AtomicBool::new(false));
    let active = recording.clone();
    let levels = Arc::new(std::sync::Mutex::new((
        DictationLevel {
            rms_dbfs: -120.0,
            peak_dbfs: -120.0,
        },
        [0.0; 15],
    )));
    let latest = levels.clone();
    let failed = Arc::new(AtomicBool::new(false));
    let failure = failed.clone();
    let (commands, receiver) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("dictation-audio".into())
        .spawn(move || {
            let mut reader = reader;
            let mut resampler = match rubato::FftFixedIn::<f32>::new(48_000, 16_000, 960, 2, 1) {
                Ok(r) => r,
                Err(_) => return,
            };
            let mut input = vec![Vec::with_capacity(960)];
            let mut delay = resampler.output_delay();
            let mut smoothed_bands = [0.0; 15];
            let mut received = 0usize;
            let mut sent = 0usize;
            let mut worker: Option<mpsc::SyncSender<worker::Input>> = None;
            let mut pending_finish = None;
            while !stopping.load(Ordering::Acquire) {
                reader
                    .set_monitoring(hub.metering() && worker.is_none() && pending_finish.is_none());
                if reader.take_overrun() {
                    failure.store(true, Ordering::Release);
                    reader.set_recording(false);
                }
                while let Ok(command) = receiver.try_recv() {
                    match command {
                        AudioCommand::Begin(sender) => {
                            while reader.pop().is_some() {}
                            input[0].clear();
                            resampler.reset();
                            delay = resampler.output_delay();
                            received = 0;
                            sent = 0;
                            worker = Some(sender);
                            pending_finish = None;
                            reader.set_recording(true);
                            active.store(true, Ordering::Release);
                        }
                        AudioCommand::Finish(request) => {
                            reader.set_recording(false);
                            pending_finish = Some((Instant::now(), request));
                        }
                        AudioCommand::Cancel => {
                            reader.set_recording(false);
                            worker = None;
                            pending_finish = None;
                            input[0].clear();
                        }
                    }
                }
                while let Some(sample) = reader.pop() {
                    input[0].push(sample);
                    received += 1;
                    if input[0].len() == 960 {
                        let mut bands = [0.0; 15];
                        if let Ok(output) = resampled(&mut resampler, &input, &mut delay) {
                            bands = band_energies(&output);
                            sent += output.len();
                            if let Some(sender) = &worker {
                                if sender.try_send(worker::Input::Audio(output)).is_err() {
                                    failure.store(true, Ordering::Release);
                                }
                            }
                        }
                        for (smoothed, target) in smoothed_bands.iter_mut().zip(bands) {
                            let time_s = if target > *smoothed { 0.04f32 } else { 0.16f32 };
                            *smoothed += (target - *smoothed) * (1.0 - (-0.02 / time_s).exp());
                        }
                        let rms = (input[0].iter().map(|x| x * x).sum::<f32>() / 960.0).sqrt();
                        let peak = input[0].iter().fold(0.0f32, |peak, x| peak.max(x.abs()));
                        if let Ok(mut latest) = latest.lock() {
                            *latest = (
                                DictationLevel {
                                    rms_dbfs: 20.0 * rms.max(1e-6).log10(),
                                    peak_dbfs: 20.0 * peak.max(1e-6).log10(),
                                },
                                smoothed_bands,
                            );
                        }
                        if active.load(Ordering::Acquire) || hub.metering() {
                            hub.broadcast(&Event::DictationLevel(DictationLevel {
                                rms_dbfs: 20.0 * rms.max(1e-6).log10(),
                                peak_dbfs: 20.0 * peak.max(1e-6).log10(),
                            }));
                        }
                        input[0].clear();
                    }
                }
                if pending_finish
                    .as_ref()
                    .is_some_and(|(at, _)| at.elapsed().as_millis() >= 30)
                {
                    let Some((_, request)) = pending_finish.take() else {
                        continue;
                    };
                    let wanted = received / 3;
                    // Flush FFT delay as well as the partial last block, preserving the final syllable.
                    while sent < wanted {
                        input[0].resize(960, 0.0);
                        match resampled(&mut resampler, &input, &mut delay) {
                            Ok(mut output) => {
                                output.truncate(wanted - sent);
                                sent += output.len();
                                if let Some(sender) = &worker {
                                    if sender.try_send(worker::Input::Audio(output)).is_err() {
                                        failure.store(true, Ordering::Release);
                                    }
                                }
                            }
                            Err(_) => {
                                failure.store(true, Ordering::Release);
                                break;
                            }
                        }
                        input[0].clear();
                    }
                    input[0].clear();
                    if let Some(sender) = &worker {
                        if sender.try_send(worker::Input::Request(request)).is_err() {
                            failure.store(true, Ordering::Release);
                        }
                    }
                    worker = None;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            reader.set_recording(false);
        })?;
    Ok(Audio {
        stop,
        commands,
        recording,
        thread: Some(thread),
        levels,
        failed,
    })
}

/// Removes the FFT resampler startup delay; the caller flushes the tail at Finish.
fn resampled(
    resampler: &mut rubato::FftFixedIn<f32>,
    input: &[Vec<f32>],
    delay: &mut usize,
) -> anyhow::Result<Vec<f32>> {
    use rubato::Resampler;
    let mut output = resampler.process(input, None)?.remove(0);
    let skip = (*delay).min(output.len());
    output.drain(..skip);
    *delay -= skip;
    Ok(output)
}

/// Fifteen logarithmic Goertzel bands for the native Flow display, measured off the RT path.
fn band_energies(samples: &[f32]) -> [f32; 15] {
    std::array::from_fn(|n| {
        let freq_hz = 100.0 * 80.0f32.powf(n as f32 / 14.0);
        let coefficient = 2.0 * (std::f32::consts::TAU * freq_hz / 16_000.0).cos();
        let (mut a, mut b) = (0.0, 0.0);
        for (i, &sample) in samples.iter().enumerate() {
            let window =
                0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / samples.len() as f32).cos();
            let next = sample * window + coefficient * a - b;
            b = a;
            a = next;
        }
        let amplitude = (a * a + b * b - coefficient * a * b).max(0.0).sqrt() * 4.0
            / samples.len().max(1) as f32;
        ((20.0 * amplitude.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_stream_flushes_final_samples_before_finish() {
        use std::sync::{Arc, atomic::Ordering, mpsc};
        let (mut control, mut rt) =
            macafon_engine::microphone(&macafon_dsp::ChainSettings::raw(), Arc::default())
                .expect("microphone");
        let reader = control
            .enable_dictation(false, macafon_dsp::TapPoint::Clean)
            .expect("tap");
        let audio = audio(reader, crate::server::Hub::new()).expect("audio thread");
        let (sender, receiver) = mpsc::sync_channel(256);
        audio
            .commands
            .send(AudioCommand::Begin(sender))
            .expect("begin");
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while !audio.recording.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "audio thread did not start");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut signal = vec![0.25; 48_000];
        assert_no_alloc::assert_no_alloc(|| {
            for block in signal.chunks_mut(128) {
                rt.process(block);
            }
        });
        audio
            .commands
            .send(AudioCommand::Finish(WorkerRequest::Finish {
                language: LanguageInput {
                    policy: LanguagePolicy::Fixed,
                    fixed: Language::En,
                    layout: None,
                    last_cyrillic: None,
                },
                vocabulary: vec![],
            }))
            .expect("finish");
        let mut output = Vec::new();
        loop {
            match receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("streamed frame")
            {
                worker::Input::Audio(samples) => output.extend(samples),
                worker::Input::Request(WorkerRequest::Finish { .. }) => break,
                _ => panic!("unexpected frame"),
            }
        }
        assert_eq!(
            output.len(),
            16_000,
            "FFT delay is removed and the tail is flushed"
        );
        let rms = (output[15_936..].iter().map(|s| s * s).sum::<f32>() / 64.0).sqrt();
        assert!(rms > 0.2, "the final syllable must survive: {rms}");
    }
}
