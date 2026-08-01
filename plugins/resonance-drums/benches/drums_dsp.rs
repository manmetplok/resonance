//! Cost of one 128-frame audio block through the drum sampler.
//!
//! Drums is an instrument, not an effect, so the cost driver is the number
//! of voices sounding at once rather than a fixed per-block chain. This
//! measures a realistic busy moment: several pads struck and still ringing.
//!
//! 128 frames @ 48 kHz = 2.667 ms of the host's budget.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use resonance_common::drum_map::GM_PADS;
use resonance_drums::kit::NUM_OUTPUT_PORTS;
use resonance_drums::ResonanceDrums;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const FRAMES: usize = 128;

/// Run one block against a freshly built set of `NUM_OUTPUT_PORTS` stereo
/// port views. The plugin is multi-output (kick / snare / hats / toms /
/// cymbals / room / master) and returns early if handed fewer ports, so
/// the full set is required for the bench to measure anything at all.
fn run_block(plugin: &mut ResonanceDrums, ports: &mut [(Vec<f32>, Vec<f32>)], events: &[NoteEvent]) {
    let mut views: Vec<OutputBuffer<'_>> = ports
        .iter_mut()
        .map(|(l, r)| OutputBuffer {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        })
        .collect();
    let mut ev = if events.is_empty() {
        EventIterator::empty()
    } else {
        EventIterator::new(events)
    };
    plugin.process(&mut views, FRAMES, &mut ev, None);
}

fn bench_drums(c: &mut Criterion) {
    let mut plugin = ResonanceDrums::new();
    plugin.initialize(SAMPLE_RATE, FRAMES as u32);

    let mut ports: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; FRAMES], vec![0.0f32; FRAMES]))
        .collect();

    // Strike six pads spread across the kit, then let them ring. Cymbals
    // and open hats stay in the voice pool for seconds, so after the decay
    // blocks the sampler is mixing a realistic simultaneous voice count.
    let strikes: Vec<NoteEvent> = [0usize, 1, 2, 3, 9, 12]
        .iter()
        .map(|&pad| NoteEvent::NoteOn {
            note: GM_PADS[pad].note,
            velocity: 0.9,
            timing: 0,
        })
        .collect();

    run_block(&mut plugin, &mut ports, &strikes);
    // A few blocks of decay so the measured state is "voices sounding",
    // not "voices just triggered".
    for _ in 0..10 {
        run_block(&mut plugin, &mut ports, &[]);
    }

    c.bench_function("drums/block128_6voices", |b| {
        b.iter(|| {
            run_block(black_box(&mut plugin), black_box(&mut ports), &[]);
            black_box(ports[0].0[0])
        })
    });
}

criterion_group!(benches, bench_drums);
criterion_main!(benches);
