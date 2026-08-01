//! Cost of one 128-frame audio block through the amp's NAM inference path.
//!
//! The amp is driven from `AmpProcessor::process_block` — the exact code the
//! CLAP `process()` calls — with a real NAM model loaded from the test
//! fixtures, so the numbers are directly comparable to the host's per-block
//! budget (128 frames @ 48 kHz = 2.667 ms of wall clock).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use resonance_amp::dsp::AmpProcessor;
use resonance_amp::nam::parse::load_model_from_file;

const SAMPLE_RATE: f32 = 48_000.0;
const FRAMES: usize = 128;

/// Realistic guitar-level input: a decaying 220 Hz tone with some harmonics,
/// so the model's nonlinearity is actually exercised (a silent buffer would
/// still cost the same in a WaveNet, but a real signal keeps the activation
/// ranges honest).
fn test_signal(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE;
            0.4 * ((220.0 * t * std::f32::consts::TAU).sin()
                + 0.3 * (440.0 * t * std::f32::consts::TAU).sin())
        })
        .collect()
}

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn bench_models(c: &mut Criterion) {
    let mut group = c.benchmark_group("amp");
    group.sample_size(50);

    // Each fixture is a real NAM capture; `a1/wavenet_a1_standard.nam` is the
    // stock "standard" WaveNet architecture that the overwhelming majority of
    // ToneHunt / Tone3000 captures use, so it is the number that matters.
    for (label, path) in [
        ("wavenet_a1_standard", fixture("a1/wavenet_a1_standard.nam")),
        ("wavenet_a1_feather", fixture("a1/wavenet.nam")),
        ("lstm", fixture("lstm/lstm.nam")),
    ] {
        let Ok(loaded) = load_model_from_file(&path) else {
            eprintln!("skipping {label}: fixture {path} did not load");
            continue;
        };

        let mut processor = AmpProcessor::new();
        processor.initialize(SAMPLE_RATE, 1.0, 1.0);
        processor.install_initial_model(loaded.model);
        processor.set_gain_targets(1.0, 1.0);

        let signal = test_signal(FRAMES);
        let mut left = signal.clone();
        let mut right = signal.clone();

        group.bench_function(label, |b| {
            b.iter(|| {
                left.copy_from_slice(&signal);
                right.copy_from_slice(&signal);
                black_box(processor.process_block(
                    black_box(&mut left),
                    black_box(&mut right),
                    FRAMES,
                ))
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_models);
criterion_main!(benches);
