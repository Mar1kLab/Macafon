use crate::protocol::{LanguageInput, Transcript};
#[cfg(any(feature = "whisper", feature = "parakeet"))]
use std::path::Path;

#[derive(Debug, Clone)]
pub struct TranscribeOptions {
    pub language: LanguageInput,
    pub vocabulary: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("model file missing: {0}")]
    ModelMissing(String),
    #[error("model load failed: {0}")]
    ModelLoad(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("no speech")]
    NoSpeech,
}

pub trait SpeechEngine {
    fn transcribe(
        &mut self,
        pcm_16k: &[f32],
        opts: &TranscribeOptions,
    ) -> Result<Transcript, SttError>;
}

#[cfg(any(feature = "whisper", feature = "parakeet"))]
pub(crate) fn model_path(dir: &Path, file_name: &str) -> Result<String, SttError> {
    let path = dir.join(file_name);
    if !path.is_file() {
        return Err(SttError::ModelMissing(file_name.to_owned()));
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| SttError::ModelLoad("model path is not UTF-8".into()))
}

#[cfg(any(feature = "whisper", feature = "parakeet"))]
pub(crate) fn duration_ms(samples: usize) -> u32 {
    (samples as u64 * 1000 / u64::from(crate::SAMPLE_RATE_HZ)).min(u64::from(u32::MAX)) as u32
}

#[cfg(any(feature = "whisper", feature = "parakeet"))]
pub(crate) fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Only complete output matches are suppressed; never remove words from real speech.
pub fn is_hallucination(text: &str) -> bool {
    let normalized = text
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c == '…')
        .trim()
        .to_lowercase();
    matches!(
        normalized.as_str(),
        "thank you for watching"
            | "thanks for watching"
            | "thank you"
            | "thanks for listening"
            | "продолжение следует"
            | "спасибо за просмотр"
            | "дякую за перегляд"
            | "далі буде"
            | "subtitles by the amara.org community"
            | "you"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hallucinations_match_whole_output_only() {
        for text in [
            "Thank you for watching!",
            "Продолжение следует…",
            "Дякую за перегляд.",
        ] {
            assert!(is_hallucination(text));
        }
        assert!(!is_hallucination(
            "Thank you for watching the children today."
        ));
    }
}
