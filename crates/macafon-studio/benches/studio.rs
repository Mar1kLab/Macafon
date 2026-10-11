use criterion::{Criterion, criterion_group, criterion_main};
use macafon_dsp::StudioProcessor;
use macafon_dsp::settings::{NoiseEngine, NoiseSuppressionParams};
use macafon_studio::StudioNoiseSuppression;
use std::hint::black_box;

fn studio(c: &mut Criterion) {
    let mut model = StudioNoiseSuppression::offline(&NoiseSuppressionParams {
        engine: NoiseEngine::Studio,
        ..NoiseSuppressionParams::default()
    })
    .expect("embedded model initializes");
    let mut frame = [0.0; 480];
    c.bench_function("studio/offline-hop-480", |b| {
        b.iter(|| {
            for (index, sample) in frame.iter_mut().enumerate() {
                *sample = (index as f32 * 0.05).sin() * 0.1;
            }
            model.process(black_box(&mut frame));
            black_box(&frame);
        })
    });
    // Queue synchronization is part of the offline benchmark, but not worker inference.
    eprintln!(
        "Last worker inference: {} us",
        model.last_inference_ns() / 1000
    );
}
criterion_group!(benches, studio);
criterion_main!(benches);
