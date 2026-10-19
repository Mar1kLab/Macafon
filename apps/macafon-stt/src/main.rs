mod platform;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use macafon_stt::{
    SpeechEngine, SttError, TranscribeOptions,
    framing::{self, Frame},
    protocol::*,
};
use std::{io, path::PathBuf, sync::mpsc, time::Instant};

#[derive(Parser)]
#[command(about = "Local dictation worker; no arguments selects the framed stdin/stdout protocol")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Transcribe {
        file: PathBuf,
        #[arg(long, value_enum)]
        engine: CliEngine,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, value_enum)]
        lang: Option<CliLanguage>,
        #[arg(long, default_value_t = 2)]
        threads: u16,
        /// Game mode: enable EcoQoS and halve the thread budget.
        #[arg(long)]
        efficiency: bool,
        /// Benchmark comparison only; ordinary dictation stays below normal.
        #[arg(long)]
        normal_priority: bool,
        /// Repeat in the same process to separate first inference from warm runs.
        #[arg(long, default_value_t = 1)]
        repeat: u16,
        #[arg(long)]
        flash_attention: bool,
        #[arg(long)]
        single_segment: bool,
        #[arg(long, default_value_t = 0)]
        audio_ctx: u16,
        #[arg(long, default_value_t = 0)]
        beam_size: u16,
        #[arg(long, default_value_t = 1)]
        vad_threads: u16,
        /// Whisper on the GPU (Vulkan).
        #[arg(long)]
        gpu: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CliEngine {
    Whisper,
    Parakeet,
}
#[derive(Clone, Copy, ValueEnum)]
enum CliLanguage {
    En,
    Uk,
    Ru,
}

fn load(
    engine: Engine,
    dir: PathBuf,
    id: String,
    threads: u16,
    gpu: bool,
) -> Result<Box<dyn SpeechEngine>, SttError> {
    match engine {
        Engine::Whisper => {
            if gpu {
                match macafon_stt::WhisperEngine::load(&dir, &id, threads, true) {
                    Ok(engine) => return Ok(Box::new(engine)),
                    // No Vulkan driver or not enough video memory: the CPU still works.
                    Err(error) => tracing::warn!(%error, "GPU load failed; using the CPU"),
                }
            }
            Ok(Box::new(macafon_stt::WhisperEngine::load(
                &dir, &id, threads, false,
            )?))
        }
        Engine::Parakeet => {
            if gpu {
                return Err(SttError::ModelLoad("Parakeet supports CPU only".into()));
            }
            Ok(Box::new(macafon_stt::ParakeetEngine::load(&dir, threads)?))
        }
    }
}

fn event_error(error: SttError) -> WorkerEvent {
    let code = match error {
        SttError::ModelMissing(_) => ErrorCode::ModelMissing,
        SttError::ModelLoad(_) => ErrorCode::ModelLoad,
        _ => ErrorCode::Inference,
    };
    // Native error details may include user data: the wire error exposes only a stable category.
    let message = match code {
        ErrorCode::ModelMissing => "A required model file is missing",
        ErrorCode::ModelLoad => "The speech model could not be loaded",
        _ => "Speech inference failed",
    }
    .to_owned();
    WorkerEvent::Error { code, message }
}

fn report_backend(engine: Engine) {
    match engine {
        Engine::Whisper => tracing::info!(
            system_info = macafon_stt::WhisperEngine::system_info(),
            "Whisper CPU backend"
        ),
        Engine::Parakeet => {
            let (sherpa, onnxruntime) = macafon_stt::ParakeetEngine::backend_versions();
            tracing::info!(sherpa, onnxruntime, provider = "cpu", "Parakeet backend");
        }
    }
}

fn report_timings(engine: &dyn SpeechEngine) {
    let timings = engine.timings();
    tracing::info!(
        vad_ms = timings.vad.as_millis(),
        detection_ms = timings.language.as_millis(),
        recognition_ms = timings.recognition.as_millis(),
        "inference stages"
    );
}

enum Input {
    Frame(Frame<WorkerRequest>),
    Eof,
    Protocol,
    Loaded {
        generation: u64,
        result: WorkerEvent,
    },
    Transcribed {
        generation: u64,
        utterance: u64,
        result: WorkerEvent,
    },
}

enum Job {
    Load {
        generation: u64,
        engine: Engine,
        dir: PathBuf,
        id: String,
        threads: u16,
        gpu: bool,
    },
    Transcribe {
        generation: u64,
        utterance: u64,
        audio: Vec<f32>,
        options: TranscribeOptions,
    },
}

fn worker() -> Result<()> {
    // A bounded queue applies pipe backpressure; the dispatcher remains responsive
    // during native loading/inference, including Cancel, Exit and stdin EOF.
    let (incoming, receive) = mpsc::sync_channel(32);
    let reader_events = incoming.clone();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        loop {
            let event = match framing::read_frame(&mut stdin) {
                Ok(Some(frame)) => Input::Frame(frame),
                Ok(None) => Input::Eof,
                Err(_) => Input::Protocol,
            };
            let done = matches!(event, Input::Eof | Input::Protocol);
            if reader_events.send(event).is_err() || done {
                break;
            }
        }
    });
    let (jobs, job_receive) = mpsc::sync_channel(2);
    std::thread::spawn(move || {
        let mut engine: Option<Box<dyn SpeechEngine>> = None;
        while let Ok(job) = job_receive.recv() {
            let event = match job {
                Job::Load {
                    generation,
                    engine: kind,
                    dir,
                    id,
                    threads,
                    gpu,
                } => {
                    // Release the previous runtime before allocating its replacement.
                    engine = None;
                    let result = match load(kind, dir, id, threads, gpu) {
                        Ok(loaded) => {
                            report_backend(kind);
                            engine = Some(loaded);
                            WorkerEvent::Ready {
                                ram_bytes: platform::ram_bytes(),
                                threads,
                            }
                        }
                        Err(error) => event_error(error),
                    };
                    Input::Loaded { generation, result }
                }
                Job::Transcribe {
                    generation,
                    utterance,
                    audio,
                    options,
                } => {
                    let audio_ms = (audio.len() as u64 * 1000 / 16000) as u32;
                    let result = match engine.as_mut() {
                        Some(engine) => {
                            let result = engine.transcribe(&audio, &options);
                            report_timings(engine.as_ref());
                            match result {
                                Ok(transcript) => WorkerEvent::Result(transcript),
                                Err(SttError::NoSpeech) => WorkerEvent::NoSpeech { audio_ms },
                                Err(error) => event_error(error),
                            }
                        }
                        None => event_error(SttError::ModelLoad("not loaded".into())),
                    };
                    Input::Transcribed {
                        generation,
                        utterance,
                        result,
                    }
                }
            };
            if incoming.send(event).is_err() {
                break;
            }
        }
    });
    let mut stdout = io::stdout().lock();
    let mut audio = Vec::new();
    let mut recording = false;
    let mut generation = 0u64;
    let mut utterance = 0u64;
    let mut ready = false;
    let mut pending = None;
    let mut busy = false;
    while let Ok(input) = receive.recv() {
        match input {
            Input::Eof | Input::Frame(Frame::Json(WorkerRequest::Exit)) => break,
            Input::Protocol => {
                framing::write_json(
                    &mut stdout,
                    &WorkerEvent::Error {
                        code: ErrorCode::Protocol,
                        message: "Invalid worker frame".into(),
                    },
                )?;
                break;
            }
            Input::Frame(Frame::Json(WorkerRequest::Load {
                engine,
                model_dir,
                model_id,
                threads,
                gpu,
                efficiency,
            })) => {
                if busy {
                    framing::write_json(
                        &mut stdout,
                        &WorkerEvent::Error {
                            code: ErrorCode::Protocol,
                            message: "Worker is busy".into(),
                        },
                    )?;
                    continue;
                }
                generation = generation.wrapping_add(1);
                ready = false;
                busy = true;
                platform::scheduling(efficiency, false)?;
                let threads = platform::thread_budget(
                    threads,
                    std::thread::available_parallelism()
                        .map(|n| n.get().min(u16::MAX as usize) as u16)
                        .unwrap_or(1),
                    efficiency,
                );
                framing::write_json(&mut stdout, &WorkerEvent::Loading)?;
                jobs.send(Job::Load {
                    generation,
                    engine,
                    dir: model_dir,
                    id: model_id,
                    threads,
                    gpu: gpu && !efficiency,
                })?;
            }
            Input::Frame(Frame::Json(WorkerRequest::Begin)) => {
                utterance = utterance.wrapping_add(1);
                recording = true;
                audio.clear();
                pending = None;
            }
            Input::Frame(Frame::Audio(chunk)) => {
                if recording {
                    if audio.len() + chunk.len() > 300 * 16000 {
                        recording = false;
                        audio.clear();
                        framing::write_json(
                            &mut stdout,
                            &WorkerEvent::Error {
                                code: ErrorCode::Protocol,
                                message: "Recording exceeds five minutes".into(),
                            },
                        )?;
                    } else {
                        audio.extend_from_slice(&chunk);
                    }
                }
            }
            Input::Frame(Frame::Json(WorkerRequest::Cancel)) => {
                utterance = utterance.wrapping_add(1);
                recording = false;
                audio.clear();
                pending = None;
            }
            Input::Frame(Frame::Json(WorkerRequest::Finish {
                language,
                vocabulary,
            })) => {
                if recording {
                    recording = false;
                    pending = Some(TranscribeOptions {
                        language,
                        vocabulary,
                    });
                }
            }
            Input::Loaded {
                generation: result_generation,
                result,
            } => {
                if result_generation == generation {
                    busy = false;
                    ready = matches!(result, WorkerEvent::Ready { .. });
                    if !ready {
                        pending = None;
                        audio.clear();
                    }
                    framing::write_json(&mut stdout, &result)?;
                }
            }
            Input::Transcribed {
                generation: result_generation,
                utterance: result_utterance,
                result,
            } => {
                busy = false;
                if result_generation == generation && result_utterance == utterance {
                    framing::write_json(&mut stdout, &result)?;
                }
            }
        }
        if ready && !busy {
            if let Some(options) = pending.take() {
                busy = true;
                jobs.send(Job::Transcribe {
                    generation,
                    utterance,
                    audio: std::mem::take(&mut audio),
                    options,
                })?;
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_target(false)
        .init();
    platform::scheduling(false, false)?;
    let Some(Command::Transcribe {
        file,
        engine,
        model_dir,
        model,
        lang,
        threads,
        efficiency,
        normal_priority,
        repeat,
        flash_attention,
        single_segment,
        audio_ctx,
        beam_size,
        vad_threads,
        gpu,
    }) = Args::parse().command
    else {
        return worker();
    };
    platform::scheduling(efficiency, normal_priority)?;
    let mut wav = hound::WavReader::open(file).context("open WAV")?;
    let spec = wav.spec();
    anyhow::ensure!(
        spec.channels == 1 && spec.sample_rate == 16000,
        "WAV must be mono at 16000 Hz"
    );
    let audio = match spec.sample_format {
        hound::SampleFormat::Float => wav.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1);
            wav.samples::<i32>()
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    anyhow::ensure!(
        audio.len() <= 300 * 16000 && audio.iter().all(|s| s.is_finite()),
        "invalid or oversized WAV"
    );
    let kind = match engine {
        CliEngine::Whisper => Engine::Whisper,
        CliEngine::Parakeet => Engine::Parakeet,
    };
    let id = model
        .or_else(|| {
            model_dir
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .context("--model required for this folder")?;
    let started = Instant::now();
    let threads = platform::thread_budget(
        threads,
        std::thread::available_parallelism()
            .map(|n| n.get().min(u16::MAX as usize) as u16)
            .unwrap_or(1),
        efficiency,
    );
    let mut engine: Box<dyn SpeechEngine> = match kind {
        Engine::Whisper => Box::new(macafon_stt::WhisperEngine::load_with_settings(
            &model_dir,
            &id,
            threads,
            gpu,
            macafon_stt::WhisperSettings {
                flash_attention,
                single_segment,
                audio_ctx,
                beam_size,
                vad_threads,
            },
        )?),
        Engine::Parakeet => Box::new(macafon_stt::ParakeetEngine::load_with_vad_threads(
            &model_dir,
            threads,
            vad_threads,
        )?),
    };
    let load_ms = started.elapsed().as_millis();
    report_backend(kind);
    let ram_bytes = platform::ram_bytes();
    let fixed = lang.map(|l| match l {
        CliLanguage::En => Language::En,
        CliLanguage::Uk => Language::Uk,
        CliLanguage::Ru => Language::Ru,
    });
    let options = TranscribeOptions {
        language: LanguageInput {
            policy: if fixed.is_some() {
                LanguagePolicy::Fixed
            } else {
                LanguagePolicy::Auto
            },
            fixed: fixed.unwrap_or(Language::En),
            layout: None,
            last_cyrillic: None,
        },
        vocabulary: Vec::new(),
    };
    for iteration in 0..repeat.max(1) {
        let result = engine.transcribe(&audio, &options);
        report_timings(engine.as_ref());
        tracing::info!(
            load_ms,
            iteration,
            ram_bytes,
            after_inference_ram_bytes = platform::ram_bytes(),
            "model measurement"
        );
        let event = match result {
            Ok(transcript) => WorkerEvent::Result(transcript),
            Err(SttError::NoSpeech) => WorkerEvent::NoSpeech {
                audio_ms: (audio.len() * 1000 / 16000) as u32,
            },
            Err(error) => event_error(error),
        };
        // Explicit debug command output goes to the caller, never through the logger.
        println!("{}", serde_json::to_string(&event)?);
    }
    Ok(())
}
