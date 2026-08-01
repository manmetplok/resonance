//! Cost of one 128-frame audio block through the whole delay plugin.
//!
//! Driven through `ResonancePlugin::process` at factory-default parameters.
//! 128 frames @ 48 kHz = 2.667 ms of the host's budget.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const FRAMES: usize = 128;

fn test_signal(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE;
            0.4 * ((220.0 * t * std::f32::consts::TAU).sin()
                + 0.3 * (587.0 * t * std::f32::consts::TAU).sin())
        })
        .collect()
}

fn bench_delay(c: &mut Criterion) {
    let mut plugin = ResonanceDelay::new();
    plugin.initialize(SAMPLE_RATE, FRAMES as u32);

    let signal = test_signal(FRAMES);
    let mut left = signal.clone();
    let mut right = signal.clone();

    // Prime the feedback path so the measured block is steady-state.
    for _ in 0..200 {
        left.copy_from_slice(&signal);
        right.copy_from_slice(&signal);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, FRAMES, &mut EventIterator::empty(), None);
    }

    c.bench_function("delay/block128", |b| {
        b.iter(|| {
            left.copy_from_slice(&signal);
            right.copy_from_slice(&signal);
            let mut outs = [OutputBuffer {
                left: &mut left,
                right: &mut right,
            }];
            plugin.process(
                black_box(&mut outs),
                FRAMES,
                &mut EventIterator::empty(),
                None,
            );
            black_box(left[0])
        })
    });
}

criterion_group!(benches, bench_delay);
criterion_main!(benches);
