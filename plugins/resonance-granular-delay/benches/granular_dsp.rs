//! Cost of one 128-frame audio block through the whole granular delay.
//!
//! Driven through `ResonancePlugin::process` at factory-default parameters.
//! 128 frames @ 48 kHz = 2.667 ms of the host's budget.
//!
//! FU-M2c adds the HQ tier's band-limited read to the picture: HQ grains
//! transposed *up* read through `resonance_dsp::BandlimitedReader`, a
//! Kaiser-windowed sinc whose tap count grows with the rate (~20 taps at
//! unity, ~80 at +24 st) where the other tiers read 2-6 taps. The
//! `granular_read/*` group measures one read per kernel; the
//! `granular_delay/block128_*` variants measure what that costs a whole
//! block at +24 st, Normal vs HQ.
//!
//!     cargo bench -p resonance-granular-delay --no-default-features \
//!         --bench granular_dsp

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use resonance_dsp::{read_bspline6_wrapped, read_hermite_wrapped, BandlimitedReader};
use resonance_granular_delay::ResonanceGranularDelay;
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

/// A plugin primed until a full population of grains is in flight,
/// otherwise we would measure the cheap "nothing scheduled yet" case.
fn primed_plugin(setup: impl Fn(&ResonanceGranularDelay)) -> ResonanceGranularDelay {
    let mut plugin = ResonanceGranularDelay::new();
    setup(&plugin);
    plugin.initialize(SAMPLE_RATE, FRAMES as u32);
    let signal = test_signal(FRAMES);
    let (mut left, mut right) = (signal.clone(), signal.clone());
    for _ in 0..400 {
        left.copy_from_slice(&signal);
        right.copy_from_slice(&signal);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, FRAMES, &mut EventIterator::empty(), None);
    }
    plugin
}

fn bench_block(c: &mut Criterion, name: &str, setup: impl Fn(&ResonanceGranularDelay)) {
    let mut plugin = primed_plugin(setup);
    let signal = test_signal(FRAMES);
    let (mut left, mut right) = (signal.clone(), signal.clone());
    c.bench_function(name, |b| {
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

fn bench_granular(c: &mut Criterion) {
    bench_block(c, "granular_delay/block128", |_| {});
    // Quality: 0 = Lo-fi, 1 = Normal, 2 = HQ.
    bench_block(c, "granular_delay/block128_normal_+24st", |p| {
        p.params.pitch.set_value(24.0);
        p.params.quality.set_value(1);
    });
    bench_block(c, "granular_delay/block128_hq_0st", |p| {
        p.params.quality.set_value(2);
    });
    bench_block(c, "granular_delay/block128_hq_+24st", |p| {
        p.params.pitch.set_value(24.0);
        p.params.quality.set_value(2);
    });
}

/// One fractional read per kernel, over a 64k-sample ring (larger than L1,
/// like the real delay buffer), stepping the read position the way a grain
/// transposed by `rate` does.
fn bench_reads(c: &mut Criterion) {
    const LEN: usize = 1 << 16;
    const READS: usize = 256;
    let buf: Vec<f32> = (0..LEN).map(|i| ((i as f32) * 0.013).sin()).collect();
    let bl = BandlimitedReader::new();
    let mut group = c.benchmark_group("granular_read");
    group.throughput(criterion::Throughput::Elements(READS as u64));
    let mut run = |name: &str, rate: f64, read: &dyn Fn(f64) -> f32| {
        group.bench_function(name, |b| {
            let mut pos = 1000.25_f64;
            b.iter(|| {
                let mut acc = 0.0f32;
                for _ in 0..READS {
                    acc += read(black_box(pos));
                    pos += rate;
                    if pos > (LEN - 1000) as f64 {
                        pos = 1000.25;
                    }
                }
                black_box(acc)
            })
        });
    };
    run("hermite4", 1.0, &|p| read_hermite_wrapped(&buf, p));
    run("bspline6", 1.0, &|p| read_bspline6_wrapped(&buf, p));
    for (st, rate) in [(7, 1.498_307_f64), (12, 2.0), (24, 4.0)] {
        run(&format!("bandlimited_+{st}st"), rate, &|p| bl.read_wrapped(&buf, p, rate));
    }
    group.finish();
}

criterion_group!(benches, bench_granular, bench_reads);
criterion_main!(benches);
