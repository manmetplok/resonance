//! Cost of one 128-frame audio block through the whole reverb plugin.
//!
//! Driven through `ResonancePlugin::process` — the exact entry point the CLAP
//! host calls — at factory-default parameters, which is what a user hears.
//! 128 frames @ 48 kHz = 2.667 ms of the host's budget.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::ResonanceReverb;

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

fn bench_reverb(c: &mut Criterion) {
    let mut plugin = ResonanceReverb::new();
    plugin.initialize(SAMPLE_RATE, FRAMES as u32);

    let signal = test_signal(FRAMES);
    let mut left = signal.clone();
    let mut right = signal.clone();

    // Prime the tail so we measure steady-state cost with a live feedback
    // network, not the cheap silent-start case.
    for _ in 0..200 {
        left.copy_from_slice(&signal);
        right.copy_from_slice(&signal);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, FRAMES, &mut EventIterator::empty(), None);
    }

    c.bench_function("reverb/block128", |b| {
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

    bench_decayed_tail(c);
}

/// Denormal check.
///
/// A feedback network fed silence decays geometrically toward zero, and
/// once the state drops below ~1e-38 every multiply-add lands in denormal
/// territory. Without flush-to-zero that costs 10-100x per operation and
/// shows up as a reverb that gets *more* expensive the quieter it gets.
///
/// This drives the tank far into decay with pure silence and measures the
/// steady state. If it comes out at or below the active-signal cost, FTZ
/// is doing its job; a large blow-up would mean it is not.
fn bench_decayed_tail(c: &mut Criterion) {
    let mut plugin = ResonanceReverb::new();
    plugin.initialize(SAMPLE_RATE, FRAMES as u32);

    let signal = test_signal(FRAMES);
    let mut left = signal.clone();
    let mut right = signal.clone();

    // Excite the tank, then feed silence long enough (~40 s) that the
    // feedback state has decayed into the denormal range many times over.
    let mut outs_once = |plugin: &mut ResonanceReverb, l: &mut Vec<f32>, r: &mut Vec<f32>| {
        let mut outs = [OutputBuffer {
            left: l,
            right: r,
        }];
        plugin.process(&mut outs, FRAMES, &mut EventIterator::empty(), None);
    };
    for _ in 0..50 {
        left.copy_from_slice(&signal);
        right.copy_from_slice(&signal);
        outs_once(&mut plugin, &mut left, &mut right);
    }
    for _ in 0..15_000 {
        left.fill(0.0);
        right.fill(0.0);
        outs_once(&mut plugin, &mut left, &mut right);
    }

    c.bench_function("reverb/block128_silent_tail", |b| {
        b.iter(|| {
            left.fill(0.0);
            right.fill(0.0);
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

criterion_group!(benches, bench_reverb);
criterion_main!(benches);
