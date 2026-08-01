//! Cost of one 128-frame audio block through the whole compressor.
//!
//! Two cases: a signal loud enough to drive gain reduction, and one that
//! never crosses the threshold. A detector-path plugin should be much
//! cheaper in the second case; if it isn't, that is a finding.
//!
//! 128 frames @ 48 kHz = 2.667 ms of the host's budget.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use resonance_compressor::ResonanceCompressor;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const FRAMES: usize = 128;

fn test_signal(frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE;
            amp * ((220.0 * t * std::f32::consts::TAU).sin()
                + 0.3 * (587.0 * t * std::f32::consts::TAU).sin())
        })
        .collect()
}

fn bench_compressor(c: &mut Criterion) {
    let mut group = c.benchmark_group("compressor");

    // `loud` drives the gain computer above the knee; `quiet` sits well
    // below the threshold so gain reduction is identically zero.
    for (label, amp) in [("loud", 0.7f32), ("quiet", 0.001f32)] {
        let mut plugin = ResonanceCompressor::new();
        plugin.initialize(SAMPLE_RATE, FRAMES as u32);

        let signal = test_signal(FRAMES, amp);
        let mut left = signal.clone();
        let mut right = signal.clone();

        for _ in 0..200 {
            left.copy_from_slice(&signal);
            right.copy_from_slice(&signal);
            let mut outs = [OutputBuffer {
                left: &mut left,
                right: &mut right,
            }];
            plugin.process(&mut outs, FRAMES, &mut EventIterator::empty(), None);
        }

        group.bench_function(label, |b| {
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

    group.finish();
}

criterion_group!(benches, bench_compressor);
criterion_main!(benches);
