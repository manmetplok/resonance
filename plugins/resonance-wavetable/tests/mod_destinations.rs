//! The two mod destinations that used to be computed and discarded
//! (ba todo #1323): Osc Balance and Unison Detune.
//!
//! Both are asserted the same way — modulating the destination must produce
//! exactly what setting the underlying parameter to the same place produces,
//! and must differ from leaving it alone. That pins "the modulated value
//! reaches the oscillator" without depending on an arbitrary threshold.
//!
//! `Velocity` is the source throughout: at velocity 1.0 it evaluates to a
//! constant +1.0, so the modulated value is static and the comparison is
//! exact.

use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 1024;

/// A patch with both oscillators up, no filter and no FX, so the only thing
/// that can move the output is the destination under test.
fn bare_params() -> WavetableParams {
    let p = WavetableParams::new();
    p.filter.enabled.set_value(false);
    p.chorus.enabled.set_value(false);
    p.delay.enabled.set_value(false);
    p.distortion.enabled.set_value(false);
    p.osc1.enabled.set_value(true);
    p.osc2.enabled.set_value(true);
    p.osc1.level.set_value(1.0);
    p.osc2.level.set_value(1.0);
    // Sustain flat so the whole block is steady state.
    p.amp_env.attack.set_value(0.001);
    p.amp_env.decay.set_value(0.001);
    p.amp_env.sustain.set_value(1.0);
    p
}

/// `source` → `dest` at `amount`. Slot indices are the picker's integers.
fn route(p: &WavetableParams, slot: usize, source: i32, dest: i32, amount: f32) {
    p.mod_slots[slot].source.set_plain(source as f64);
    p.mod_slots[slot].destination.set_plain(dest as f64);
    p.mod_slots[slot].amount.set_value(amount);
}

const SRC_VELOCITY: i32 = 5;
const SRC_LFO3: i32 = 3;
const DEST_OSC_BALANCE: i32 = 7;
const DEST_UNISON_DETUNE: i32 = 9;

/// Render one block of a held note, stereo interleaved into one vec.
fn render_note(params: &WavetableParams) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let events = [NoteEvent::NoteOn {
        note: 57, // A3, 220 Hz
        velocity: 1.0,
        timing: 0,
    }];
    let mut iter = EventIterator::new(&events);
    engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter);
    left.extend_from_slice(&right);
    left
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

// ---------------------------------------------------------------------------
// Osc Balance
// ---------------------------------------------------------------------------

#[test]
fn osc_balance_modulation_equals_moving_the_parameter() {
    // Modulated: balance param centred, velocity pushes it fully to +1.
    let modulated = bare_params();
    modulated.osc_balance.set_value(0.0);
    route(&modulated, 0, SRC_VELOCITY, DEST_OSC_BALANCE, 1.0);

    // Reference: balance param already at +1, nothing modulating it.
    let reference = bare_params();
    reference.osc_balance.set_value(1.0);

    let a = render_note(&modulated);
    let b = render_note(&reference);
    assert_eq!(
        a, b,
        "modulating osc balance to +1 must equal setting the param to +1"
    );
}

#[test]
fn osc_balance_modulation_changes_the_mix() {
    let unmodulated = bare_params();
    unmodulated.osc_balance.set_value(0.0);

    let modulated = bare_params();
    modulated.osc_balance.set_value(0.0);
    route(&modulated, 0, SRC_VELOCITY, DEST_OSC_BALANCE, 1.0);

    let a = render_note(&unmodulated);
    let b = render_note(&modulated);
    assert!(peak(&a) > 1e-3 && peak(&b) > 1e-3, "one render was silent");
    assert!(
        a.iter().zip(&b).any(|(x, y)| x != y),
        "osc balance modulation produced identical audio — it is not reaching the mix"
    );
}

#[test]
fn osc_balance_modulation_can_mute_an_oscillator() {
    // Balance +1 takes osc1 to zero, so a patch with only osc1 audible goes
    // silent — the clearest possible proof the value reaches the level.
    let params = bare_params();
    params.osc2.enabled.set_value(false);
    params.osc_balance.set_value(0.0);
    assert!(peak(&render_note(&params)) > 1e-3);

    route(&params, 0, SRC_VELOCITY, DEST_OSC_BALANCE, 1.0);
    assert_eq!(
        peak(&render_note(&params)),
        0.0,
        "osc1 should be fully crossfaded out"
    );
}

// ---------------------------------------------------------------------------
// Unison Detune
// ---------------------------------------------------------------------------

#[test]
fn unison_detune_modulation_equals_moving_the_parameter() {
    // Full-scale modulation adds the parameter's whole 0..100 ct range.
    let modulated = bare_params();
    modulated.unison.voices.set_value(5);
    modulated.unison.spread.set_value(0.5);
    modulated.unison.detune.set_value(0.0);
    route(&modulated, 0, SRC_VELOCITY, DEST_UNISON_DETUNE, 1.0);

    let reference = bare_params();
    reference.unison.voices.set_value(5);
    reference.unison.spread.set_value(0.5);
    reference.unison.detune.set_value(100.0);

    let a = render_note(&modulated);
    let b = render_note(&reference);
    assert_eq!(
        a, b,
        "modulating unison detune to full scale must equal setting the param to 100 ct"
    );
}

#[test]
fn unison_detune_modulation_changes_the_sound() {
    let unmodulated = bare_params();
    unmodulated.unison.voices.set_value(5);
    unmodulated.unison.detune.set_value(0.0);

    let modulated = bare_params();
    modulated.unison.voices.set_value(5);
    modulated.unison.detune.set_value(0.0);
    route(&modulated, 0, SRC_VELOCITY, DEST_UNISON_DETUNE, 0.5);

    let a = render_note(&unmodulated);
    let b = render_note(&modulated);
    assert!(peak(&a) > 1e-3 && peak(&b) > 1e-3, "one render was silent");
    assert!(
        a.iter().zip(&b).any(|(x, y)| x != y),
        "unison detune modulation produced identical audio — it is not reaching the oscillator"
    );
}

#[test]
fn unison_detune_modulation_does_not_restart_or_click_the_voice() {
    // Sine wavetable (index 0, position 0) at 220 Hz: the largest legitimate
    // sample-to-sample step is ~2*pi*220/48000 ≈ 2.9 % of the peak. A voice
    // restart or a phase reset would show up as a step comparable to the
    // peak itself, so 20 % is a wide but decisive bound.
    let params = bare_params();
    params.osc1.wavetable.set_plain(0.0);
    params.osc1.position.set_value(0.0);
    params.osc2.enabled.set_value(false);
    params.unison.voices.set_value(5);
    params.unison.spread.set_value(0.5);
    params.unison.detune.set_value(20.0);
    // A moving source, so the detune width really is re-resolved at every
    // control tick across the block.
    params.lfo3.rate.set_value(8.0);
    params.lfo3.depth.set_value(1.0);
    route(&params, 0, SRC_LFO3, DEST_UNISON_DETUNE, 1.0);

    let out = render_note(&params);
    let p = peak(&out);
    assert!(p > 1e-3, "render was silent");

    // Skip the note-on attack; look at the steady-state tail of the left
    // channel only (the vec is L block then R block).
    let left = &out[..BLOCK];
    let max_step = left[BLOCK / 2..]
        .windows(2)
        .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
    assert!(
        max_step < 0.2 * p,
        "detune modulation introduced a discontinuity: max step {max_step} vs peak {p}"
    );
}
