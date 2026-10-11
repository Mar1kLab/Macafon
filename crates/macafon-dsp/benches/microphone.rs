use criterion::{Criterion, criterion_group, criterion_main};
use macafon_dsp::{ChainSettings, MicrophoneProcessor, Preset};
use std::hint::black_box;

fn microphone(c: &mut Criterion) {
    let mut without_ns = Preset::Broadcast.settings();
    without_ns.noise_suppression.enabled = false;
    let mut auto_release = without_ns;
    auto_release.compressor.params.auto_release = true;
    for (name, settings) in [
        ("raw/128", ChainSettings::raw()),
        ("chain-without-ns/128", without_ns),
        ("natural/128", Preset::Natural.settings()),
        ("auto-release/128", auto_release),
        ("metered-chain/128", without_ns),
    ] {
        let mut processor =
            MicrophoneProcessor::new(&settings).expect("valid benchmark parameters");
        processor
            .telemetry()
            .set_enabled(name == "metered-chain/128");
        let mut block = [0.0; 128];
        c.bench_function(name, |b| {
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
