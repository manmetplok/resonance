//! DSP2-03: retriggering or stealing a sounding voice must not click.
//!
//! `Voice::trigger` used to zero the filter state, jump every oscillator
//! to phase 0 and reset the sub, while the amp envelope kept its level —
//! so a voice still sounding restarted mid-waveform at non-zero gain. The
//! measure is the largest sample-to-sample step: through a resonant
//! low-pass the steady tone's steps are small, and any reset under a
//! non-zero envelope shows up as a step far above them.

use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;

fn params(max_voices: i32) -> WavetableParams {
    let p = WavetableParams::new();
    p.max_voices.set_value(max_voices);
    p.filter.enabled.set_value(true);
    p.filter.filter_type.set_value(0);
    p.filter.cutoff.set_value(900.0);
    p.filter.resonance.set_value(0.6);
    p.filter.env_depth.set_value(0.0);
    p.filter.keytrack.set_value(0.0);
    p.amp_env.attack.set_value(0.005);
    p.amp_env.sustain.set_value(0.8);
    p.amp_env.release.set_value(0.3);
    p
}

/// A note event `at` seconds into the render.
#[derive(Clone, Copy)]
enum Ev {
    On(u8, f32),
    Off(u8, f32),
}

/// Render `seconds` of mono (left) output with the given events.
fn render(params: &WavetableParams, events: &[Ev], seconds: f32) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let frames = (seconds * SR) as usize;
    let mut out = Vec::with_capacity(frames);
    let mut start = 0usize;
    while start < frames {
        let n = BLOCK.min(frames - start);
        let mut block_events: Vec<NoteEvent> = events
            .iter()
            .filter_map(|e| {
                let (at, ev) = match *e {
                    Ev::On(note, t) => (t, (note, true)),
                    Ev::Off(note, t) => (t, (note, false)),
                };
                let s = (at * SR) as usize;
                (s >= start && s < start + n).then(|| {
                    let timing = (s - start) as u32;
                    if ev.1 {
                        NoteEvent::NoteOn { note: ev.0, velocity: 0.8, timing }
                    } else {
                        NoteEvent::NoteOff { note: ev.0, timing }
                    }
                })
            })
            .collect();
        block_events.sort_by_key(|e| e.timing());
        let mut left = vec![0.0f32; n];
        let mut right = vec![0.0f32; n];
        let mut iter = EventIterator::new(&block_events);
        engine.render_block(&mut left, &mut right, n, params, &mut iter, None);
        out.extend_from_slice(&left);
        start += n;
    }
    out
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

fn slice(x: &[f32], from: f32, to: f32) -> &[f32] {
    &x[(from * SR) as usize..(to * SR) as usize]
}

/// The steps around a retrigger at `at` stay within 1.5x the larger of
/// the steady tone's steps before and after it.
fn assert_continuous(name: &str, out: &[f32], steady_before: (f32, f32), at: f32, steady_after: (f32, f32)) {
    let before = max_step(slice(out, steady_before.0, steady_before.1));
    let after = max_step(slice(out, steady_after.0, steady_after.1));
    assert!(before > 1e-4 && after > 1e-4, "{name}: rendered silence");
    let around = max_step(slice(out, at - 0.002, at + 0.02));
    let bound = 1.5 * before.max(after);
    assert!(
        around <= bound,
        "{name}: step {around:.4} at the retrigger, steady steps {before:.4} / {after:.4}"
    );
}

#[test]
fn mono_retrigger_in_the_release_tail_is_continuous() {
    let p = params(1);
    // Note-off at 0.3 s, the same key again 50 ms into the 300 ms release.
    let out = render(&p, &[Ev::On(36, 0.0), Ev::Off(36, 0.3), Ev::On(36, 0.35)], 0.7);
    assert_continuous("mono same note", &out, (0.1, 0.3), 0.35, (0.5, 0.7));
    let out = render(&p, &[Ev::On(36, 0.0), Ev::Off(36, 0.3), Ev::On(31, 0.35)], 0.7);
    assert_continuous("mono new note", &out, (0.1, 0.3), 0.35, (0.5, 0.7));
}

#[test]
fn poly_steal_at_the_voice_ceiling_is_continuous() {
    let p = params(2);
    // Two held notes fill the ceiling; the third steals the oldest.
    let out = render(&p, &[Ev::On(36, 0.0), Ev::On(31, 0.01), Ev::On(33, 0.3)], 0.6);
    assert_continuous("poly steal", &out, (0.1, 0.3), 0.3, (0.4, 0.6));
}
