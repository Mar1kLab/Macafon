#![cfg(feature = "parakeet")]

use macafon_stt::{ParakeetEngine, SpeechEngine, TranscribeOptions, protocol::*};
use std::{error::Error, path::PathBuf};

/// Real-model regression: sending a whole WAV to Silero in one call used to
/// classify this audible recording as silence. Fixtures stay outside the repo.
/// Windows also needs the matching runtime DLLs beside the Cargo test executable.
#[test]
#[ignore = "requires MACAFON_STT_TEST_MODEL_DIR and MACAFON_STT_TEST_WAV (16 kHz mono speech)"]
fn parakeet_speech_survives_vad_windowing() -> Result<(), Box<dyn Error>> {
    let dir = PathBuf::from(std::env::var("MACAFON_STT_TEST_MODEL_DIR")?);
    let wav = hound::WavReader::open(std::env::var("MACAFON_STT_TEST_WAV")?)?;
    assert_eq!(wav.spec().channels, 1);
    assert_eq!(wav.spec().sample_rate, 16000);
    assert_eq!(wav.spec().bits_per_sample, 16);
    let samples = wav
        .into_samples::<i16>()
        .map(|s| s.map(|s| f32::from(s) / 32768.0))
        .collect::<Result<Vec<_>, _>>()?;
    let mut engine = ParakeetEngine::load(&dir, 2)?;
    let result = engine.transcribe(
        &samples,
        &TranscribeOptions {
            language: LanguageInput {
                policy: LanguagePolicy::Auto,
                fixed: Language::from_code("en").expect("supported language"),
                layout: None,
                last_cyrillic: None,
                layout_languages: Vec::new(),
            },
            vocabulary: Vec::new(),
        },
    )?;
    assert!(!result.text.trim().is_empty());
    assert!(result.speech_ms >= 200);
    assert_eq!(result.language, None);
    assert_eq!(result.lang_probs, None);
    Ok(())
}
