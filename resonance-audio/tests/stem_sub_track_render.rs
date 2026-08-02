//! Sub-track stems (ba todo #1242) — the `StemSource::Track` sibling of
//! ba todo #1239's bus fix.
//!
//! Every assertion here is on RENDERED AUDIO, not on `stem_filter`'s set.
//! That is the lesson #1239 paid for: the filter-only tests went green
//! while the stem itself was still digital silence, because a sub-track
//! produces no audio of its own — its signal is one output port of the
//! PARENT's instrument, fanned out while the parent renders, and
//! `in_filter` drops a track before its instrument runs.

mod multi_out_harness;

use multi_out_harness::{at_master, peak, EngineState, FRAMES, PARENT, PORT_LEVELS, TAP_A, TAP_B};
use resonance_audio::__test_support::StemSource;
use resonance_audio::types::TrackOutput;

/// Both taps land on master, so the parent's own stem is their sum.
const BOTH_TAPS: f32 = PORT_LEVELS[1] + PORT_LEVELS[2];

/// The bug: `stem_filter(Track(sub))` resolved to `{sub}` alone, the
/// parent instrument never ran, and the stem was digital silence — which
/// `render.stems`, a single-sub-track stem export and `meter.measure`
/// (ba todo #1219) all reported as if it were a real measurement.
#[test]
fn sub_track_stem_is_not_silent() {
    let state = EngineState::new();
    let stem = state.render(StemSource::Track(TAP_A));
    assert_eq!(stem.len(), FRAMES as usize * 2);
    assert!(
        peak(&stem) > 0.05,
        "a sub-track stem must carry its tap's audio, got peak {}",
        peak(&stem)
    );
}

/// "Drums -> Hats" must yield hats, not the whole kit. The parent is
/// pulled into the filter only to DRIVE the fan-out; the sibling taps are
/// held out by `sub_track_disposition`'s own `in_filter` gate
/// (`render_core.rs`), and the parent's port-0 chain is suppressed
/// because it is in the filter for the fan-out alone.
#[test]
fn a_sub_track_stem_carries_only_its_own_tap() {
    let state = EngineState::new();

    let a = peak(&state.render(StemSource::Track(TAP_A)));
    let b = peak(&state.render(StemSource::Track(TAP_B)));
    let parent = peak(&state.render(StemSource::Track(PARENT)));

    assert!(
        (a - at_master(PORT_LEVELS[1])).abs() < 1e-6,
        "tap A's stem is exactly tap A ({}), got {a}",
        at_master(PORT_LEVELS[1])
    );
    assert!(
        (b - at_master(PORT_LEVELS[2])).abs() < 1e-6,
        "tap B's stem is exactly tap B ({}), got {b}",
        at_master(PORT_LEVELS[2])
    );
    assert!(
        (parent - at_master(BOTH_TAPS)).abs() < 1e-6,
        "the parent's stem is the whole instrument ({}), got {parent}",
        at_master(BOTH_TAPS)
    );
    assert!(
        a < parent,
        "a single tap ({a}) must be less than the whole kit ({parent})"
    );
}

/// The parent's own main output (port 0) must not leak into a sub-track's
/// stem either. The default harness leaves port 0 silent — the drum kit's
/// real behaviour, where exactly one of thirty pads lands there (ba doc
/// #274 §1a) — so it cannot see this on its own; this case makes port 0
/// the LOUDEST port and pins that the tap's stem is unaffected.
///
/// Without the fan-out-only suppression the parent's port-0 signal runs
/// its own chain and follows its own routing to master, so it lands in
/// the stem alongside the tap: "Drums -> Hats" would come back as hats
/// plus the count stick.
#[test]
fn the_parents_port_0_does_not_leak_into_a_sub_track_stem() {
    let levels = [0.5, PORT_LEVELS[1], PORT_LEVELS[2]];
    let state = EngineState::with_port_levels(levels);

    let a = peak(&state.render(StemSource::Track(TAP_A)));
    assert!(
        (a - at_master(levels[1])).abs() < 1e-6,
        "tap A's stem is exactly tap A ({}), got {a} — port 0 ({}) leaked in",
        at_master(levels[1]),
        levels[0]
    );

    // The parent's own stem still carries port 0 plus both taps: the
    // suppression applies only when the parent is in the filter purely
    // to drive somebody else's fan-out.
    let parent = peak(&state.render(StemSource::Track(PARENT)));
    let all = at_master(levels[0] + levels[1] + levels[2]);
    assert!(
        (parent - all).abs() < 1e-6,
        "the parent's own stem keeps its main output: expected {all}, got {parent}"
    );
}

/// A top-level track with sub-tracks renders exactly as before: parent
/// plus every tap, whatever each tap is routed to (an instrument's audio
/// belongs to that instrument).
#[test]
fn a_top_level_track_stem_still_includes_all_its_taps() {
    let state = EngineState::new();
    let before = peak(&state.render(StemSource::Track(PARENT)));
    assert!(
        (before - at_master(BOTH_TAPS)).abs() < 1e-6,
        "parent stem is both taps ({}), got {before}",
        at_master(BOTH_TAPS)
    );

    // Re-routing a tap to a bus must not remove it from its instrument's
    // own stem. It does not stay level-identical, and that is not this
    // fix's doing: `mixer::common` applies the constant-power pan law at
    // the track stage AND again at the bus stage, so a centre-panned bus
    // costs another 3 dB on the way through. Pre-existing gain staging —
    // asserted here as "tap B still contributes", not as a level, so this
    // test does not quietly bless the double attenuation.
    state.add_bus(7, "Kit Bus");
    state.set_output(TAP_B, TrackOutput::Bus(7));
    let after = peak(&state.render(StemSource::Track(PARENT)));

    let tap_a_alone = at_master(PORT_LEVELS[1]);
    assert!(
        after > tap_a_alone + 1e-6,
        "the bus-routed tap B still belongs to its instrument's own stem: \
         {after} must exceed tap A alone ({tap_a_alone})"
    );
    assert!(
        after <= before + 1e-6,
        "and it cannot gain level by being re-routed: {before} -> {after}"
    );
}

/// The fan-out walk does not recurse: pulling the parent in for tap A
/// must not drag tap B along, even when tap B is routed somewhere else.
#[test]
fn a_sub_track_stem_is_unaffected_by_where_its_sibling_is_routed() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(TAP_B, TrackOutput::Bus(7));

    let a = peak(&state.render(StemSource::Track(TAP_A)));
    assert!(
        (a - at_master(PORT_LEVELS[1])).abs() < 1e-6,
        "tap A's stem is exactly tap A ({}), got {a}",
        at_master(PORT_LEVELS[1])
    );
}
