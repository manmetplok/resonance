//! Bus stems fed by a multi-output instrument's sub-tracks (ba todo #1239).
//!
//! `stem_filter` used to resolve a bus's membership by scanning only
//! top-level tracks, so a bus fed exclusively by an instrument's group
//! taps came out EMPTY — and its stem export, and every `meter.measure` /
//! `meter.stems` built on the same filter (ba todo #1218), reported
//! digital silence rather than an error. That routing is precisely what
//! ba doc #274 recommends for glue-compressing a multi-output instrument,
//! so the filter tests in `stem_render.rs` are not enough: this file
//! proves it end to end, through a real render.
//!
//! The hand-rolled three-port CLAP instrument these tests render lives in
//! `multi_out_harness` (moved there by ba todo #1242, which needed the
//! same plugin for the `StemSource::Track` arm); see that module for why
//! sub-track audio cannot be produced with plain clips.

mod multi_out_harness;

use multi_out_harness::{peak, EngineState, FRAMES};
use resonance_audio::__test_support::{stem_filter, StemSource};
use resonance_audio::types::TrackOutput;

/// The bug, end to end: an instrument's group taps routed into a bus of
/// their own must render as that bus's stem. Before ba todo #1239 the
/// filter was empty and this buffer was digital silence.
#[test]
fn bus_stem_fed_only_by_sub_tracks_is_not_silent() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(10, TrackOutput::Bus(7));
    state.set_output(11, TrackOutput::Bus(7));

    let filter = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert!(
        filter.contains(10) && filter.contains(11),
        "both taps feed the bus: {:?}",
        filter.set
    );

    let stem = state.render(StemSource::Bus(7));
    assert_eq!(stem.len(), FRAMES as usize * 2);
    let bus_peak = peak(&stem);
    assert!(
        bus_peak > 0.05,
        "the bus stem must carry the taps' audio, got peak {bus_peak}"
    );

    // The taps are the ONLY contributors, so the bus stem must equal the
    // parent's whole-instrument stem (port 0 is silent by construction,
    // as it is on the real drum kit).
    let track_stem = state.render(StemSource::Track(1));
    let track_peak = peak(&track_stem);
    assert!(
        (bus_peak - track_peak).abs() < 1e-6,
        "bus stem ({bus_peak}) must carry exactly the instrument's audio ({track_peak})"
    );
}

/// The pre-existing path must not regress: a top-level track routed to a
/// bus still brings its sub-tracks into that bus's stem.
#[test]
fn bus_stem_still_includes_a_routed_parents_sub_tracks() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(1, TrackOutput::Bus(7));

    let filter = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert!(filter.contains(1), "the routed parent");
    assert!(
        filter.contains(10) && filter.contains(11),
        "its sub-tracks ride along: {:?}",
        filter.set
    );

    let stem = state.render(StemSource::Bus(7));
    assert!(
        peak(&stem) > 0.05,
        "the routed instrument still reaches its bus stem"
    );
}

/// A parent and one of its sub-tracks both pointed at the same bus must
/// contribute their audio ONCE. `stem_filter` dedupes the ids; this pins
/// the render side, which is what a set alone cannot prove.
#[test]
fn parent_and_sub_track_on_one_bus_are_not_summed_twice() {
    // Baseline: taps routed to the bus, parent on master.
    let baseline = {
        let state = EngineState::new();
        state.add_bus(7, "Kit Bus");
        state.set_output(10, TrackOutput::Bus(7));
        state.set_output(11, TrackOutput::Bus(7));
        state.render(StemSource::Bus(7))
    };

    // Same routing, but the (silent-port-0) parent also targets the bus,
    // so it is picked up BOTH by the top-level scan and by `add_sub_tracks`
    // reaching its taps. Port 0 carries nothing, so if any tap's audio
    // were summed twice this stem would be ~6 dB hotter.
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(1, TrackOutput::Bus(7));
    state.set_output(10, TrackOutput::Bus(7));
    state.set_output(11, TrackOutput::Bus(7));

    let filter = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert_eq!(
        filter.set.len(),
        3,
        "parent + two taps, each exactly once: {:?}",
        filter.set
    );

    let doubled = state.render(StemSource::Bus(7));
    assert_eq!(baseline.len(), doubled.len());
    let (a, b) = (peak(&baseline), peak(&doubled));
    assert!(a > 0.05, "baseline carries audio, got {a}");
    assert!(
        (a - b).abs() < 1e-6,
        "no double-count: peak {a} (parent on master) vs {b} (parent on the bus)"
    );
}

/// A tap routed to a different bus leaves the first bus's stem and joins
/// the other one; the instrument's own track stem keeps both.
#[test]
fn a_tap_routed_elsewhere_moves_between_bus_stems() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.add_bus(8, "Other Bus");
    state.set_output(10, TrackOutput::Bus(7));
    state.set_output(11, TrackOutput::Bus(8));

    let seven = state.render(StemSource::Bus(7));
    let eight = state.render(StemSource::Bus(8));
    let both = state.render(StemSource::Track(1));

    // Tap A is louder than tap B (PORT_LEVELS), so the two bus stems are
    // distinguishable and neither carries the other's audio.
    let (p7, p8, pall) = (peak(&seven), peak(&eight), peak(&both));
    assert!(p7 > 0.05 && p8 > 0.05, "each bus carries its own tap");
    assert!(
        p7 > p8 * 1.5,
        "bus 7 has the louder tap ({p7}) and bus 8 the quieter ({p8})"
    );
    assert!(
        pall > p7,
        "the instrument's own stem ({pall}) carries both taps, more than either bus ({p7})"
    );
}
