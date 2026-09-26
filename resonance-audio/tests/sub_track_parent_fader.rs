//! The parent fader of a multi-output instrument (ba doc #275 P1.1).
//!
//! A drum kit leaves through its taps, not through the parent's own main
//! output — which is silent for `com.resonance.drums`. So the parent's
//! fader used to move a value that nothing read: `mixer.set_volume_db`
//! stored it, `song.summary` read it back, and the audio was bit-identical.
//! A field agent balanced a mix against a fader that did nothing and the
//! master measured exactly what unattenuated drums predict.
//!
//! The fader is now the kit's GROUP TRIM: volume only, applied to every
//! tap. Pan is deliberately not folded in — see `auto_volume_ramp`.

mod multi_out_harness;

use multi_out_harness::{at_master, peak, EngineState, PARENT, PORT_LEVELS, TAP_A};
use resonance_audio::test_support::StemSource;

/// Both taps reach master, so the kit's level is their sum.
const BOTH_TAPS: f32 = PORT_LEVELS[1] + PORT_LEVELS[2];

fn kit_level(state: &EngineState) -> f32 {
    peak(&state.render(StemSource::Track(PARENT)))
}

#[test]
fn the_parent_fader_trims_the_whole_kit() {
    let state = EngineState::new();
    let unity = kit_level(&state);
    assert!(
        (unity - at_master(BOTH_TAPS)).abs() < 1e-6,
        "unity parent: expected {}, got {unity}",
        at_master(BOTH_TAPS)
    );

    state.tracks.read().get(&PARENT).unwrap().set_volume(0.5);
    let trimmed = kit_level(&state);
    assert!(
        (trimmed - at_master(BOTH_TAPS) * 0.5).abs() < 1e-6,
        "-6 dB on the parent must halve the kit: expected {}, got {trimmed}",
        at_master(BOTH_TAPS) * 0.5
    );
}

/// Unity must stay bit-identical: the trim is a multiply by 1.0 for every
/// project that never touched the parent fader.
#[test]
fn a_parent_at_unity_changes_nothing() {
    let state = EngineState::new();
    let before = state.render(StemSource::Track(PARENT));
    state.tracks.read().get(&PARENT).unwrap().set_volume(1.0);
    let after = state.render(StemSource::Track(PARENT));
    assert_eq!(before, after, "unity trim must not perturb a sample");
}

/// The trim composes with each tap's own fader rather than replacing it —
/// a kit at -6 dB with the snare tap at -6 dB puts the snare at -12 dB.
#[test]
fn the_trim_composes_with_the_taps_own_fader() {
    let state = EngineState::new();
    state.tracks.read().get(&PARENT).unwrap().set_volume(0.5);
    state.tracks.read().get(&TAP_A).unwrap().set_volume(0.5);

    let got = peak(&state.render(StemSource::Track(TAP_A)));
    let expect = at_master(PORT_LEVELS[1]) * 0.25;
    assert!(
        (got - expect).abs() < 1e-6,
        "expected {expect} (0.5 parent x 0.5 tap), got {got}"
    );
}

/// Silencing the parent silences the kit — this already worked (mute
/// propagates through `parent_silenced`) and must keep working now that
/// the gain path changed.
#[test]
fn a_parent_at_zero_silences_the_kit() {
    let state = EngineState::new();
    state.tracks.read().get(&PARENT).unwrap().set_volume(0.0);
    assert!(
        kit_level(&state) < 1e-9,
        "a parent fader at -inf must silence its taps"
    );
}
