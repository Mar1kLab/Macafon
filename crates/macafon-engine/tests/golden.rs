//! Sparse waveform references keep audio regression fixtures small and reviewable.
use macafon_dsp::{Preset, SAMPLE_RATE_HZ};
use macafon_engine::prepare_offline_processor;
use std::path::Path;
use std::sync::Arc;

fn pcm16_mono(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("checked-in test recording");
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut offset = 12;
    let mut result = None;
    while offset + 8 <= bytes.len() {
        let length = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .expect("fixed-width fixture field"),
        ) as usize;
        let data = &bytes[offset + 8..offset + 8 + length];
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                assert_eq!(
                    u16::from_le_bytes(data[..2].try_into().expect("fixed-width fixture field")),
                    1
                );
                assert_eq!(
                    u16::from_le_bytes(data[2..4].try_into().expect("fixed-width fixture field")),
                    1
                );
                assert_eq!(
                    u32::from_le_bytes(data[4..8].try_into().expect("fixed-width fixture field")),
                    SAMPLE_RATE_HZ
                );
                assert_eq!(
                    u16::from_le_bytes(data[14..16].try_into().expect("fixed-width fixture field")),
                    16
                );
            }
            b"data" => {
                result = Some(
                    data.chunks_exact(2)
                        .map(|v| {
                            i16::from_le_bytes(v.try_into().expect("fixed-width fixture field"))
                                as f32
                                / 32768.0
                        })
                        .collect(),
                )
            }
            _ => {}
        }
        offset += 8 + length + (length & 1);
    }
    result.expect("recording has a data chunk")
}

#[test]
fn speech_with_fan_keyboard_and_street_noise_matches_reference_waveforms() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (noise, preset) in [
        ("fan", Preset::Natural),
        ("keyboard", Preset::Broadcast),
        ("street", Preset::NoisyRoom),
    ] {
        let mut samples = pcm16_mono(&fixtures.join(format!("voice-{noise}.wav")));
        let mut processor = prepare_offline_processor(&preset.settings(), Arc::default())
            .expect("preset initializes offline processor");
        for block in samples.chunks_mut(128) {
            processor.process(block);
        }
        assert!(samples.iter().all(|v| v.is_finite()));
        assert!(
            samples.iter().any(|v| v.abs() > 0.01),
            "{noise} lost speech"
        );
        let reference = fixtures.join(format!("{noise}-{}.f32", preset.id()));
        let anchors: Vec<f32> = samples.iter().step_by(97).copied().collect();
        if std::env::var_os("MACAFON_UPDATE_GOLDEN").as_deref() == Some(std::ffi::OsStr::new("1")) {
            std::fs::write(
                &reference,
                anchors
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .expect("explicit reference update succeeds");
        }
        let expected = std::fs::read(reference).expect("explicitly generated reference waveform");
        assert_eq!(expected.len(), anchors.len() * 4);
        for (index, (actual, expected)) in anchors.iter().zip(expected.chunks_exact(4)).enumerate()
        {
            let expected =
                f32::from_le_bytes(expected.try_into().expect("fixed-width fixture field"));
            assert!(
                (actual - expected).abs() <= 0.0001,
                "{noise} sample {}: {actual} vs {expected}",
                index * 97
            );
        }
    }
}
