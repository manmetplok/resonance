//! Cost of one 128-frame audio block through the IR convolution engine.
//!
//! The dominant variable is IR length: the shared `FftConvolver` is *uniformly*
//! partitioned at the engine's hop size, so per-block cost grows linearly with
//! the number of partitions (`ir_len / hop`). Benchmarking a sweep of lengths
//! is the point — a 20 ms cabinet IR and a 2 s room IR are different worlds.
//!
//! 128 frames @ 48 kHz = 2.667 ms of the host's budget.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use resonance_ir::dsp::{block_size_for_sample_rate, IrEngine, StereoConvolver};
use resonance_plugin::{Smoother, SmoothingStyle};

const SAMPLE_RATE: f32 = 48_000.0;
const FRAMES: usize = 128;

/// A synthetic but spectrally realistic impulse response: an exponentially
/// decaying noise burst. Matches the partition count and memory-traffic
/// profile of a real capture of the same length, which is what we are
/// measuring — the actual sample values do not affect FFT cost.
fn synth_ir(len: usize) -> Vec<f32> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state >> 40) as f32 / 8_388_608.0) - 1.0
    };
    (0..len)
        .map(|i| {
            let decay = (-6.0 * i as f32 / len as f32).exp();
            rng() * decay
        })
        .collect()
}

fn test_signal(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE;
            0.4 * (220.0 * t * std::f32::consts::TAU).sin()
        })
        .collect()
}

fn bench_ir_lengths(c: &mut Criterion) {
    let mut group = c.benchmark_group("ir");
    group.sample_size(100);

    let hop = block_size_for_sample_rate(SAMPLE_RATE);
    let signal = test_signal(FRAMES);

    // Lengths in milliseconds, spanning the realistic range:
    //   20 ms  — guitar cabinet IR (the plugin's headline use case)
    //   50 ms  — long cabinet / small room
    //  200 ms  — ambience
    //  500 ms  — room
    // 2000 ms  — hall / large space
    for ms in [20u32, 50, 200, 500, 2000] {
        let len = (ms as f32 * 0.001 * SAMPLE_RATE) as usize;
        let partitions = len.div_ceil(hop);
        let ir = synth_ir(len);

        let mut engine = IrEngine::new(hop);
        engine.install(StereoConvolver::new(&ir, None, hop));

        let mut dry_wet = Smoother::new(SmoothingStyle::Linear(50.0));
        dry_wet.set_sample_rate(SAMPLE_RATE);
        dry_wet.reset(1.0);
        let mut output_gain = Smoother::new(SmoothingStyle::Linear(50.0));
        output_gain.set_sample_rate(SAMPLE_RATE);
        output_gain.reset(1.0);

        let mut left = signal.clone();
        let mut right = signal.clone();

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{ms}ms_{partitions}part")),
            &len,
            |b, _| {
                b.iter(|| {
                    left.copy_from_slice(&signal);
                    right.copy_from_slice(&signal);
                    black_box(engine.process_block(
                        black_box(&mut left),
                        black_box(&mut right),
                        &mut dry_wet,
                        &mut output_gain,
                    ))
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_ir_lengths);
criterion_main!(benches);
