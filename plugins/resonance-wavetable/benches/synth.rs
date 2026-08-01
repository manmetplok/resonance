//! Criterion benchmarks for the wavetable synth's realtime path.
//!
//! Everything here measures `SynthEngine::render_block` — the routine the
//! CLAP `process()` callback runs — at a 128-frame quantum and 48 kHz, the
//! configuration the DAW actually uses.
//!
//! The `realtime_budget` group is the one that matters: it reports how much
//! of one 128-frame quantum (2.667 ms of wall clock at 48 kHz) a single
//! plugin instance consumes. Divide the reported time by 2.667 ms to get the
//! fraction of one core's realtime budget.
//!
//! Run with:
//!   cargo bench -p resonance-wavetable --no-default-features

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
/// The quantum the DAW runs at.
const QUANTUM: usize = 128;

/// Build an engine primed with `notes` held down under `params`, then render
/// a few blocks so envelopes have left the attack stage and the filter/FX
/// state is warm. Benchmarks then measure steady-state sustain, which is what
/// dominates a real project.
fn prime(params: &WavetableParams, notes: &[u8]) -> SynthEngine {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let events: Vec<NoteEvent> = notes
        .iter()
        .map(|&note| NoteEvent::NoteOn {
            note,
            velocity: 0.8,
            timing: 0,
        })
        .collect();

    let mut left = vec![0.0f32; QUANTUM];
    let mut right = vec![0.0f32; QUANTUM];
    let mut iter = EventIterator::new(&events);
    engine.render_block(&mut left, &mut right, QUANTUM, params, &mut iter);

    // Warm up past the attack stage (~20 ms).
    for _ in 0..8 {
        let mut iter = EventIterator::new(&[]);
        engine.render_block(&mut left, &mut right, QUANTUM, params, &mut iter);
    }
    engine
}

fn render_once(engine: &mut SynthEngine, params: &WavetableParams, l: &mut [f32], r: &mut [f32]) {
    let mut iter = EventIterator::new(&[]);
    engine.render_block(l, r, QUANTUM, params, &mut iter);
}

/// A chord spanning a realistic register.
fn chord(n: usize) -> Vec<u8> {
    // Stack thirds upward from C2 so mip-level selection sees a real spread
    // of frequencies rather than one cached octave.
    (0..n).map(|i| 36 + (i as u8 * 4) % 48).collect()
}

/// Params matching the "Pad — Warm Analog" preset shape: both oscs on,
/// filter on with drive, chorus + delay on, two mod slots live. This is
/// representative of what the reported 272-bar project runs.
fn heavy_params(unison: i32, max_voices: i32) -> WavetableParams {
    let p = WavetableParams::new();
    p.max_voices.set_value(max_voices);
    p.osc1.enabled.set_value(true);
    p.osc2.enabled.set_value(true);
    p.osc1.wavetable.set_value(1);
    p.osc2.wavetable.set_value(0);
    p.osc1.position.set_value(0.2);
    p.osc2.position.set_value(0.95);
    p.osc2.coarse.set_value(-12);
    p.osc1.pan.set_value(-0.3);
    p.osc2.pan.set_value(0.3);
    p.unison.voices.set_value(unison);
    p.unison.detune.set_value(18.0);
    p.unison.spread.set_value(0.8);

    // Long envelopes so the whole benchmark sits in sustain.
    p.amp_env.attack.set_value(0.01);
    p.amp_env.sustain.set_value(0.9);
    p.amp_env.release.set_value(2.5);

    p.filter.enabled.set_value(true);
    p.filter.cutoff.set_value(2500.0);
    p.filter.resonance.set_value(0.15);
    p.filter.env_depth.set_value(0.25);
    p.filter.keytrack.set_value(0.3);
    p.filter.drive.set_value(0.15);

    // LFO1 -> osc1 position, LFO2 -> filter cutoff (as in the pad preset).
    p.mod_slots[0].source.set_value(1);
    p.mod_slots[0].destination.set_value(1);
    p.mod_slots[0].amount.set_value(0.15);
    p.mod_slots[1].source.set_value(2);
    p.mod_slots[1].destination.set_value(5);
    p.mod_slots[1].amount.set_value(0.25);

    p.chorus.enabled.set_value(true);
    p.delay.enabled.set_value(true);
    p
}

/// Minimal params: one osc, no filter, no FX, no modulation. Isolates the
/// raw oscillator + envelope cost.
fn bare_params(unison: i32) -> WavetableParams {
    let p = WavetableParams::new();
    p.osc1.enabled.set_value(true);
    p.osc2.enabled.set_value(false);
    p.unison.voices.set_value(unison);
    p.unison.detune.set_value(18.0);
    p.filter.enabled.set_value(false);
    p.chorus.enabled.set_value(false);
    p.delay.enabled.set_value(false);
    p.distortion.enabled.set_value(false);
    p.amp_env.attack.set_value(0.01);
    p.amp_env.sustain.set_value(0.9);
    p
}

/// One voice, one unison, one oscillator: the floor of the per-oscillator
/// cost. Throughput is set to frames so criterion reports ns/frame too.
fn bench_single_voice(c: &mut Criterion) {
    let mut g = c.benchmark_group("single_voice");
    g.throughput(Throughput::Elements(QUANTUM as u64));

    for &unison in &[1i32, 3, 5, 7] {
        let params = bare_params(unison);
        let mut engine = prime(&params, &[60]);
        let mut l = vec![0.0f32; QUANTUM];
        let mut r = vec![0.0f32; QUANTUM];

        g.bench_with_input(BenchmarkId::new("bare_unison", unison), &unison, |b, _| {
            b.iter(|| {
                render_once(&mut engine, &params, &mut l, &mut r);
                black_box(l[0] + r[0])
            })
        });
    }
    g.finish();
}

/// Scaling across polyphony at each unison width, with the full signal path
/// (filter + FX + mod matrix) enabled.
fn bench_polyphony(c: &mut Criterion) {
    let mut g = c.benchmark_group("polyphony");
    g.throughput(Throughput::Elements(QUANTUM as u64));

    for &unison in &[1i32, 3, 5] {
        for &voices in &[1usize, 4, 8, 16] {
            let params = heavy_params(unison, 32);
            let mut engine = prime(&params, &chord(voices));
            let mut l = vec![0.0f32; QUANTUM];
            let mut r = vec![0.0f32; QUANTUM];

            g.bench_with_input(
                BenchmarkId::new(format!("u{unison}"), voices),
                &voices,
                |b, _| {
                    b.iter(|| {
                        render_once(&mut engine, &params, &mut l, &mut r);
                        black_box(l[0] + r[0])
                    })
                },
            );
        }
    }
    g.finish();
}

/// The headline number: one plugin instance at the project's worst case
/// (16 voices, unison 5, both oscs, filter, chorus + delay, mod matrix) for
/// one 128-frame quantum. 2.667 ms is 100% of one core's realtime budget at
/// 48 kHz; the DAW runs five of these instances.
fn bench_realtime_budget(c: &mut Criterion) {
    let mut g = c.benchmark_group("realtime_budget");
    g.throughput(Throughput::Elements(QUANTUM as u64));

    // Project worst case per the bug report.
    {
        let params = heavy_params(5, 16);
        let mut engine = prime(&params, &chord(16));
        let mut l = vec![0.0f32; QUANTUM];
        let mut r = vec![0.0f32; QUANTUM];
        g.bench_function("instance_16voice_unison5", |b| {
            b.iter(|| {
                render_once(&mut engine, &params, &mut l, &mut r);
                black_box(l[0] + r[0])
            })
        });
    }

    // A more typical sustained-chord load (4 notes held).
    {
        let params = heavy_params(3, 16);
        let mut engine = prime(&params, &chord(4));
        let mut l = vec![0.0f32; QUANTUM];
        let mut r = vec![0.0f32; QUANTUM];
        g.bench_function("instance_4voice_unison3", |b| {
            b.iter(|| {
                render_once(&mut engine, &params, &mut l, &mut r);
                black_box(l[0] + r[0])
            })
        });
    }

    // Idle instance: no notes at all. Should be near-free; if it isn't,
    // the per-block fixed overhead is the problem.
    {
        let params = heavy_params(5, 16);
        let mut engine = prime(&params, &[]);
        let mut l = vec![0.0f32; QUANTUM];
        let mut r = vec![0.0f32; QUANTUM];
        g.bench_function("instance_idle", |b| {
            b.iter(|| {
                render_once(&mut engine, &params, &mut l, &mut r);
                black_box(l[0] + r[0])
            })
        });
    }
    g.finish();
}

/// Isolate individual stages by toggling one feature at a time against a
/// fixed 8-voice / unison-3 load. The deltas between these tell us where the
/// time actually goes without needing perf.
fn bench_stages(c: &mut Criterion) {
    let mut g = c.benchmark_group("stages");
    let notes = chord(8);

    let variants: Vec<(&str, Box<dyn Fn() -> WavetableParams>)> = vec![
        (
            "osc_only",
            Box::new(|| {
                let p = bare_params(3);
                p.osc2.enabled.set_value(false);
                p
            }),
        ),
        (
            "osc_x2",
            Box::new(|| {
                let p = bare_params(3);
                p.osc2.enabled.set_value(true);
                p
            }),
        ),
        (
            "osc_x2_filter",
            Box::new(|| {
                let p = bare_params(3);
                p.osc2.enabled.set_value(true);
                p.filter.enabled.set_value(true);
                p.filter.drive.set_value(0.15);
                p
            }),
        ),
        (
            "osc_x2_filter_nodrive",
            Box::new(|| {
                let p = bare_params(3);
                p.osc2.enabled.set_value(true);
                p.filter.enabled.set_value(true);
                p.filter.drive.set_value(0.0);
                p
            }),
        ),
        (
            "osc_x2_filter_fx",
            Box::new(|| {
                let p = bare_params(3);
                p.osc2.enabled.set_value(true);
                p.filter.enabled.set_value(true);
                p.filter.drive.set_value(0.15);
                p.chorus.enabled.set_value(true);
                p.delay.enabled.set_value(true);
                p
            }),
        ),
        (
            "oscs_disabled",
            Box::new(|| {
                let p = bare_params(3);
                p.osc1.enabled.set_value(false);
                p.osc2.enabled.set_value(false);
                p
            }),
        ),
    ];

    for (name, build) in variants {
        let params = build();
        let mut engine = prime(&params, &notes);
        let mut l = vec![0.0f32; QUANTUM];
        let mut r = vec![0.0f32; QUANTUM];
        g.bench_function(name, |b| {
            b.iter(|| {
                render_once(&mut engine, &params, &mut l, &mut r);
                black_box(l[0] + r[0])
            })
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_single_voice,
    bench_polyphony,
    bench_realtime_budget,
    bench_stages
);
criterion_main!(benches);
