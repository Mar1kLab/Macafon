use crate::{SpeechEngine, SttError, TranscribeOptions, Transcript, engine::*};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};
use std::{path::Path, time::Instant};

pub struct ParakeetEngine {
    recognizer: OfflineRecognizer,
    vad: VoiceActivityDetector,
}

impl ParakeetEngine {
    pub fn load(model_dir: &Path, threads: u16) -> Result<Self, SttError> {
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(model_path(model_dir, "encoder.int8.onnx")?),
            decoder: Some(model_path(model_dir, "decoder.int8.onnx")?),
            joiner: Some(model_path(model_dir, "joiner.int8.onnx")?),
        };
        config.model_config.tokens = Some(model_path(model_dir, "tokens.txt")?);
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = i32::from(threads.max(1));
        config.model_config.provider = Some("cpu".into());
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
            SttError::ModelLoad("Parakeet recognizer initialization failed".into())
        })?;
        let vad_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model_path(model_dir, "silero_vad.onnx")?),
                threshold: 0.5,
                min_silence_duration: 0.5,
                min_speech_duration: 0.1,
                window_size: 512,
                max_speech_duration: 30.0,
            },
            sample_rate: 16000,
            num_threads: i32::from(threads.max(1)),
            provider: Some("cpu".into()),
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&vad_config, 310.0)
            .ok_or_else(|| SttError::ModelLoad("Silero VAD initialization failed".into()))?;
        Ok(Self { recognizer, vad })
    }
}

impl SpeechEngine for ParakeetEngine {
    fn transcribe(
        &mut self,
        pcm_16k: &[f32],
        _opts: &TranscribeOptions,
    ) -> Result<Transcript, SttError> {
        let started = Instant::now();
        self.vad.reset();
        // sherpa's Silero wrapper advances one analysis window per call.
        for window in pcm_16k.chunks(512) {
            self.vad.accept_waveform(window);
        }
        self.vad.flush();
        let mut segments = Vec::new();
        let mut speech_samples = 0;
        while !self.vad.is_empty() {
            let segment = self
                .vad
                .front()
                .ok_or_else(|| SttError::Inference("VAD segment unavailable".into()))?;
            speech_samples += segment.samples().len();
            segments.push(segment.samples().to_vec());
            self.vad.pop();
        }
        let speech_ms = duration_ms(speech_samples);
        if speech_ms < 200 {
            return Err(SttError::NoSpeech);
        }
        let mut text = String::new();
        // Bound encoder memory for the five-minute recording limit: VAD emits
        // segments of at most 30 seconds, each decoded independently.
        for segment in segments {
            let stream = self.recognizer.create_stream();
            stream.accept_waveform(16000, &segment);
            self.recognizer.decode(&stream);
            let result = stream
                .get_result()
                .ok_or_else(|| SttError::Inference("Parakeet produced no result".into()))?;
            text.push(' ');
            text.push_str(&result.text);
        }
        let text = clean_text(&text);
        if text.is_empty() {
            return Err(SttError::NoSpeech);
        }
        Ok(Transcript {
            text,
            language: None,
            lang_probs: None,
            audio_ms: duration_ms(pcm_16k.len()),
            speech_ms,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        })
    }
}
