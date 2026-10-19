use crate::{
    SpeechEngine, SttError, TranscribeOptions, Transcript,
    engine::*,
    language,
    protocol::{Language, LanguagePolicy},
};
use std::{path::Path, time::Instant};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

pub struct WhisperEngine {
    context: WhisperContext,
    vad: WhisperVadContext,
    threads: u16,
    settings: WhisperSettings,
    timings: InferenceTimings,
}

/// Explicit tuning for local engine measurements. Defaults preserve full-context
/// recognition quality; shorter audio contexts require accuracy validation.
#[derive(Debug, Clone, Copy)]
pub struct WhisperSettings {
    pub flash_attention: bool,
    pub single_segment: bool,
    pub audio_ctx: u16,
    /// Zero selects greedy best-of-one; a positive value selects beam search.
    pub beam_size: u16,
    /// Zero uses the recognition budget; positive values isolate the VAD pool.
    pub vad_threads: u16,
}

impl Default for WhisperSettings {
    fn default() -> Self {
        Self {
            flash_attention: false,
            single_segment: false,
            audio_ctx: 0,
            beam_size: 0,
            vad_threads: 1,
        }
    }
}

impl WhisperEngine {
    pub fn system_info() -> &'static str {
        whisper_rs::print_system_info()
    }

    pub fn load(
        model_dir: &Path,
        model_id: &str,
        threads: u16,
        gpu: bool,
    ) -> Result<Self, SttError> {
        Self::load_with_settings(
            model_dir,
            model_id,
            threads,
            gpu,
            WhisperSettings::default(),
        )
    }

    pub fn load_with_settings(
        model_dir: &Path,
        model_id: &str,
        threads: u16,
        gpu: bool,
        settings: WhisperSettings,
    ) -> Result<Self, SttError> {
        if gpu && !cfg!(feature = "vulkan") {
            return Err(SttError::ModelLoad("this build has no GPU support".into()));
        }
        let spec = crate::manifest::models()
            .iter()
            .find(|spec| spec.id == model_id && spec.engine == crate::protocol::Engine::Whisper)
            .ok_or_else(|| SttError::ModelLoad("unknown Whisper model".into()))?;
        let mut params = WhisperContextParameters::default();
        params.use_gpu(gpu);
        // Flash attention is the fast path on GPUs; the CPU keeps its measured setting.
        params.flash_attn(settings.flash_attention || gpu);
        let context = WhisperContext::new_with_params(
            &model_path(model_dir, spec.files[0].file_name)?,
            params,
        )
        .map_err(|e| SttError::ModelLoad(e.to_string()))?;
        let threads = threads.max(1);
        let mut vad_params = WhisperVadContextParams::new();
        vad_params.set_n_threads(i32::from(if settings.vad_threads == 0 {
            threads
        } else {
            settings.vad_threads
        }));
        vad_params.set_use_gpu(false);
        let vad = WhisperVadContext::new(
            &model_path(model_dir, "ggml-silero-v5.1.2.bin")?,
            vad_params,
        )
        .map_err(|e| SttError::ModelLoad(e.to_string()))?;
        Ok(Self {
            context,
            vad,
            threads,
            settings,
            timings: InferenceTimings::default(),
        })
    }
}

impl SpeechEngine for WhisperEngine {
    fn timings(&self) -> InferenceTimings {
        self.timings
    }

    fn transcribe(
        &mut self,
        pcm_16k: &[f32],
        opts: &TranscribeOptions,
    ) -> Result<Transcript, SttError> {
        let started = Instant::now();
        self.timings = InferenceTimings::default();
        if pcm_16k.len() < 3200 {
            return Err(SttError::NoSpeech);
        }
        let mut vad_params = WhisperVadParams::new();
        vad_params.set_min_speech_duration(100);
        vad_params.set_min_silence_duration(500);
        vad_params.set_speech_pad(30);
        let segments = self
            .vad
            .segments_from_samples(vad_params, pcm_16k)
            .map_err(|e| SttError::Inference(e.to_string()))?;
        let mut speech = Vec::new();
        let mut previous_end = 0;
        for segment in segments {
            // whisper.cpp timestamps are centiseconds, or 160 samples at 16 kHz.
            let start = ((segment.start * 160.0) as usize)
                .max(previous_end)
                .min(pcm_16k.len());
            let end = ((segment.end * 160.0) as usize)
                .max(start)
                .min(pcm_16k.len());
            speech.extend_from_slice(&pcm_16k[start..end]);
            previous_end = end;
        }
        let speech_ms = duration_ms(speech.len());
        self.timings = InferenceTimings {
            vad: started.elapsed(),
            ..Default::default()
        };
        if speech_ms < 200 {
            return Err(SttError::NoSpeech);
        }
        let mut state = self
            .context
            .create_state()
            .map_err(|e| SttError::Inference(e.to_string()))?;
        let lang_probs = if opts.language.policy == LanguagePolicy::Fixed {
            None
        } else {
            let window = &speech[..speech.len().min(30 * 16000)];
            state
                .pcm_to_mel(window, usize::from(self.threads))
                .map_err(|e| SttError::Inference(e.to_string()))?;
            let (_, probs) = state
                .lang_detect(0, usize::from(self.threads))
                .map_err(|e| SttError::Inference(e.to_string()))?;
            let mut restricted = [0.0; 3];
            for (index, language) in Language::ALL.into_iter().enumerate() {
                let id = whisper_rs::get_lang_id(language.code())
                    .ok_or_else(|| SttError::Inference("language unavailable".into()))?;
                restricted[index] = probs.get(id as usize).copied().unwrap_or(0.0);
            }
            let sum: f32 = restricted.iter().sum();
            if sum > 0.0 {
                restricted = restricted.map(|p| p / sum);
            }
            Some(restricted)
        };
        let language = language::choose(&opts.language, lang_probs);
        self.timings.language = started.elapsed() - self.timings.vad;
        let decoding = Instant::now();
        let strategy = if self.settings.beam_size == 0 {
            SamplingStrategy::Greedy { best_of: 1 }
        } else {
            SamplingStrategy::BeamSearch {
                beam_size: i32::from(self.settings.beam_size),
                patience: -1.0,
            }
        };
        let mut params = FullParams::new(strategy);
        params.set_single_segment(self.settings.single_segment);
        params.set_audio_ctx(i32::from(self.settings.audio_ctx));
        params.set_n_threads(i32::from(self.threads));
        params.set_language(language.map(Language::code));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_no_speech_thold(0.6);
        // NUL cannot be represented by the native prompt API.
        let prompt = opts
            .vocabulary
            .iter()
            .take(50)
            .map(|word| word.replace('\0', ""))
            .collect::<Vec<_>>()
            .join(", ");
        params.set_initial_prompt(&prompt);
        state
            .full(params, &speech)
            .map_err(|e| SttError::Inference(e.to_string()))?;
        self.timings.recognition = decoding.elapsed();
        let mut text = String::new();
        for segment in state.as_iter() {
            if segment.no_speech_probability() < 0.6 {
                text.push_str(
                    segment
                        .to_str()
                        .map_err(|_| SttError::Inference("invalid UTF-8 output".into()))?,
                );
            }
        }
        let text = clean_text(&text);
        if text.is_empty() || is_hallucination(&text) {
            return Err(SttError::NoSpeech);
        }
        Ok(Transcript {
            text,
            language,
            lang_probs,
            audio_ms: duration_ms(pcm_16k.len()),
            speech_ms,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        })
    }
}
