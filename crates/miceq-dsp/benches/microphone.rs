use criterion::{Criterion, criterion_group, criterion_main};
use miceq_dsp::MicrophoneProcessor;
use std::hint::black_box;

fn microphone(c: &mut Criterion) {
    for lite in [false, true] {
        let mut processor = if lite {
            MicrophoneProcessor::lite(70.0, 50.0, false).expect("valid benchmark parameters")
        } else {
            MicrophoneProcessor::raw(false)
        };
        let mut block = [0.0; 128];
        c.bench_function(if lite { "lite/128" } else { "raw/128" }, |b| {
            b.iter(|| {
                for (i, sample) in block.iter_mut().enumerate() {
                    *sample = (i as f32 * 0.1).sin() * 0.2;
                }
                processor.process(black_box(&mut block));
                black_box(&block);
            })
        });
    }
}

criterion_group!(benches, microphone);
criterion_main!(benches);
