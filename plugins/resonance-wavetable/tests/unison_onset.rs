//! DSP2-13: unison sub-voices must not start phase-locked.
//!
//! With `osc_phase_random` at its default 0, every sub-voice used to start
//! at phase 0. The stack is summed with a 1/√N gain, so the onset was a
//! coherent peak about √N above the steady, phase-spread level (+8.5 dB at
//! 7 voices) followed by a slow phasey sweep. A fixed per-index phase
//! spread keeps the onset at the steady level, and sub-voice 0 still
//! starts at phase 0 so a single-voice patch renders bit-identically.

use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn render(params: &WavetableParams, seconds: f32) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let blocks = (seconds * SR) as usize / BLOCK;
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for b in 0..blocks {
        let events = if b == 0 {
            vec![NoteEvent::NoteOn {
                note: 45,
                velocity: 1.0,
                timing: 0,
            }]
        } else {
            Vec::new()
        };
        let mut left = vec![0.0f32; BLOCK];
        let mut right = vec![0.0f32; BLOCK];
        let mut iter = EventIterator::new(&events);
        engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
        out.extend_from_slice(&left);
    }
    out
}

fn unison_patch(voices: i32) -> WavetableParams {
    let p = WavetableParams::new();
    p.osc1.enabled.set_value(true);
    p.osc2.enabled.set_value(false);
    p.unison.voices.set_value(voices);
    p.unison.detune.set_value(20.0);
    p.unison.spread.set_value(0.0);
    p.analog.phase_random.set_value(0.0);
    p.filter.enabled.set_value(false);
    p.amp_env.attack.set_value(0.001);
    p.amp_env.sustain.set_value(1.0);
    p
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn a_unison_onset_is_not_a_coherent_peak() {
    let out = render(&unison_patch(7), 1.5);
    let win = (0.02 * SR) as usize;
    let onset = peak(&out[..win]);
    // Mean of the 20 ms window peaks over the phase-spread steady state.
    let steady: Vec<f32> = out[(0.3 * SR) as usize..]
        .chunks(win)
        .map(peak)
        .collect();
    let steady = steady.iter().sum::<f32>() / steady.len() as f32;
    assert!(steady > 1e-3, "rendered silence");
    let ratio_db = 20.0 * (onset / steady).log10();
    assert!(
        ratio_db < 2.5,
        "unison-7 onset peaks {ratio_db:.1} dB above the steady level"
    );
}
