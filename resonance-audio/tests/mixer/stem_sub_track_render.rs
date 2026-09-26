//! Sub-track stems (ba todo #1242) — the `StemSource::Track` sibling of
//! ba todo #1239's bus fix.
//!
//! Every assertion here is on RENDERED AUDIO, not on `stem_filter`'s set.
//! That is the lesson #1239 paid for: the filter-only tests went green
//! while the stem itself was still digital silence, because a sub-track
//! produces no audio of its own — its signal is one output port of the
//! PARENT's instrument, fanned out while the parent renders, and
//! `in_filter` drops a track before its instrument runs.

use crate::multi_out_harness;

use multi_out_harness::{
    at_master, peak, EngineState, FRAMES, PARENT, PORT_LEVELS, SIBLING, SIBLING_TAP, TAP_A,
    TAP_B,
};
use resonance_audio::test_support::StemSource;
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

// ---------------------------------------------------------------------------
// Frozen parents (ba todo #1248)
// ---------------------------------------------------------------------------

/// The whole kit as one signal, which is what a freeze cache of a
/// multi-output instrument holds: `freeze_raw` forces every sub-track
/// into master at unity, so the parent's single cache file carries the
/// summed fan-out.
const FROZEN_KIT: f32 = PORT_LEVELS[1] + PORT_LEVELS[2];

/// ba todo #1248, the regression #1242 introduced in kind. A frozen
/// track never reaches `discard_own_output` — that lives in the
/// instrument arm of the per-track loop, and the frozen branch fills the
/// buffer from cache and short-circuits it — so a sub-track stem came
/// back carrying the parent's ENTIRE frozen cache under one tap's name.
///
/// Measured before the fix: tap A's stem peaked at the whole kit's level
/// (0.26516503) instead of tap A's (0.17677669). Silence would have been
/// wrong; the whole kit is worse, because a caller cannot tell.
///
/// The refusal is asserted on the RENDER call, and the level the bug
/// produced is named explicitly, so a future change that resumes
/// returning audio here cannot pass by being merely non-silent.
#[test]
fn a_sub_track_stem_of_a_frozen_parent_is_refused_not_answered_with_the_kit() {
    let state = EngineState::new();
    state.freeze(PARENT, FROZEN_KIT, FRAMES as usize);

    let err = state
        .try_render(StemSource::Track(TAP_A))
        .expect_err("a tap of a frozen parent has no separable signal");

    // The message must be actionable: which track is frozen, and what to
    // do about it.
    assert!(
        err.contains(&PARENT.to_string()) && err.contains("frozen"),
        "the error must name the frozen parent: {err}"
    );
    assert!(
        err.contains("Unfreeze"),
        "the error must say how to get a real answer: {err}"
    );
    assert!(
        err.contains("Tap 1"),
        "the error must name the tap that was asked for: {err}"
    );
}

/// The same refusal covers a BUS stem fed by a frozen parent's taps —
/// the routing ba doc #274 option (b) recommends. The defect is
/// identical there (the parent's cache reaches master and lands in the
/// bus stem), so the guard is on the filter's fan-out parents rather
/// than on `StemSource::Track`.
#[test]
fn a_bus_stem_fed_by_a_frozen_parents_taps_is_refused_too() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(TAP_A, TrackOutput::Bus(7));
    state.freeze(PARENT, FROZEN_KIT, FRAMES as usize);

    let err = state
        .try_render(StemSource::Bus(7))
        .expect_err("the bus is fed only by taps of a frozen instrument");
    assert!(err.contains("frozen"), "{err}");
}

/// The frozen parent's OWN stem is still perfectly renderable — the
/// cache is exactly that instrument's whole output, which is what this
/// stem is asking for. Only fan-out parents are refused, so freezing a
/// kit must not break stemming the kit.
#[test]
fn a_frozen_parents_own_stem_still_renders_from_its_cache() {
    let state = EngineState::new();
    state.freeze(PARENT, FROZEN_KIT, FRAMES as usize);

    let stem = state
        .try_render(StemSource::Track(PARENT))
        .expect("a frozen track's own stem comes straight from its cache");
    let got = peak(&stem);
    assert!(
        (got - at_master(FROZEN_KIT)).abs() < 1e-6,
        "the frozen kit's stem is its cache at master ({}), got {got}",
        at_master(FROZEN_KIT)
    );
}

/// An unrelated track's stem is unaffected by somebody else's freeze —
/// the guard keys on the filter's own fan-out parents, not on "is
/// anything in this project frozen".
#[test]
fn an_unfrozen_instruments_taps_are_unaffected_by_a_frozen_neighbour() {
    let state = EngineState::new();
    state.add_unfrozen_sibling(SIBLING, SIBLING_TAP);
    state.freeze(PARENT, FROZEN_KIT, FRAMES as usize);

    let stem = state
        .try_render(StemSource::Track(SIBLING_TAP))
        .expect("the sibling instrument is not frozen");
    assert!(
        (peak(&stem) - at_master(PORT_LEVELS[1])).abs() < 1e-6,
        "the unfrozen instrument's tap still renders normally, got {}",
        peak(&stem)
    );
}

/// And once the parent is unfrozen the tap renders again — the refusal
/// is a statement about the freeze, not a permanent property of the
/// track.
#[test]
fn unfreezing_the_parent_makes_the_tap_renderable_again() {
    let state = EngineState::new();
    state.freeze(PARENT, FROZEN_KIT, FRAMES as usize);
    assert!(state.try_render(StemSource::Track(TAP_A)).is_err());

    state.unfreeze(PARENT);
    let stem = state
        .try_render(StemSource::Track(TAP_A))
        .expect("unfrozen, the tap is separable again");
    assert!(
        (peak(&stem) - at_master(PORT_LEVELS[1])).abs() < 1e-6,
        "and it carries exactly tap A, got {}",
        peak(&stem)
    );
}

// ---------------------------------------------------------------------------
// Solo resolution in the mixdown (code review MIX-07)
// ---------------------------------------------------------------------------

/// Soloing a multi-output kit and exporting the mix. Sub-tracks follow
/// their parent's solo (`any_top_level_solo` ignores their own flag), and
/// the live mixer honours that — but the bounce arm used to demand each
/// sub-track's OWN solo flag, so the export dropped every tap and carried
/// only the parent's (silent) port 0 while playback had the whole kit.
#[test]
fn a_soloed_kits_taps_are_in_the_mixdown() {
    let state = EngineState::new();
    state.tracks.read().get(&PARENT).unwrap().set_soloed(true);

    let mix = peak(&state.render(StemSource::Master));
    assert!(
        (mix - at_master(BOTH_TAPS)).abs() < 1e-6,
        "the soloed kit exports with both taps ({}), got {mix}",
        at_master(BOTH_TAPS)
    );
}

/// The other half of the same rule: the solo still suppresses a kit that
/// is NOT soloed — its taps follow their (silenced) parent out — and a
/// tap's own mute still applies under its soloed parent.
#[test]
fn solo_still_suppresses_other_kits_and_a_muted_tap() {
    let state = EngineState::new();
    state.add_unfrozen_sibling(SIBLING, SIBLING_TAP);
    state.tracks.read().get(&PARENT).unwrap().set_soloed(true);
    state.tracks.read().get(&TAP_B).unwrap().set_muted(true);

    let mix = peak(&state.render(StemSource::Master));
    assert!(
        (mix - at_master(PORT_LEVELS[1])).abs() < 1e-6,
        "only the soloed kit's unmuted tap A ({}) is exported, got {mix}",
        at_master(PORT_LEVELS[1])
    );
}
