//! Sidechain key capture (`types::sidechain`).
//!
//! The routing model's whole correctness argument is the double buffer:
//! a key always reads the PREVIOUS block's capture, so the result cannot
//! depend on the order tracks happen to render in, and a track keying off
//! itself (or off something downstream of itself) is legal rather than a
//! cycle. These tests pin that, plus the bounded slot allocation that
//! keeps the audio thread allocation-free.

use resonance_audio::types::sidechain::{active_sources, from_bus, from_track, route_source};
use resonance_audio::types::{
    SendSource, SidechainRoute, SidechainTaps, MAX_SIDECHAIN_SOURCES,
};

const FRAMES: usize = 64;

fn taps(routes: &[SidechainRoute]) -> SidechainTaps {
    let mut t = SidechainTaps::new(FRAMES);
    t.begin_block(routes);
    t
}

fn ramp(v: f32) -> Vec<f32> {
    vec![v; FRAMES]
}

// ---------------------------------------------------------------------------
// Route resolution
// ---------------------------------------------------------------------------

#[test]
fn a_plugins_route_resolves_to_its_source() {
    let routes = vec![from_track(1, 10), from_bus(2, 20)];
    assert_eq!(route_source(&routes, 1), Some(SendSource::Track(10)));
    assert_eq!(route_source(&routes, 2), Some(SendSource::Bus(20)));
    assert_eq!(route_source(&routes, 3), None, "unrouted plugin");
}

#[test]
fn a_disabled_route_resolves_to_nothing() {
    // Disabled keeps its configuration but delivers no key, so the plugin
    // falls back to its own input rather than going silent.
    let routes = vec![SidechainRoute {
        plugin: 1,
        source: SendSource::Track(10),
        enabled: false,
    }];
    assert_eq!(route_source(&routes, 1), None);
    assert!(active_sources(&routes).is_empty());
}

#[test]
fn distinct_sources_are_deduplicated_and_capped() {
    // Ten plugins all keyed from the same track is one tap, not ten.
    let routes: Vec<_> = (1..=10).map(|p| from_track(p, 10)).collect();
    assert_eq!(active_sources(&routes), vec![SendSource::Track(10)]);

    // More distinct sources than slots: the excess is dropped rather than
    // misrouted onto someone else's buffer.
    let many: Vec<_> = (1..=20).map(|p| from_track(p, p + 100)).collect();
    let sources = active_sources(&many);
    assert_eq!(sources.len(), MAX_SIDECHAIN_SOURCES);
    // First-seen order, so which sources survive is deterministic.
    assert_eq!(sources[0], SendSource::Track(101));
}

// ---------------------------------------------------------------------------
// The double buffer
// ---------------------------------------------------------------------------

#[test]
fn a_key_reads_the_previous_block_not_this_one() {
    // The property the whole design rests on. Capturing inside a block
    // must not be visible to a key read in that same block, or the result
    // would depend on whether the source track happened to render first.
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);

    t.capture(SendSource::Track(10), &ramp(1.0), &ramp(1.0), FRAMES);
    assert!(
        t.key(SendSource::Track(10)).is_none(),
        "this block's capture must not be readable in this block"
    );

    t.begin_block(&routes);
    let (l, r) = t.key(SendSource::Track(10)).expect("previous block's audio");
    assert!(l[..FRAMES].iter().all(|s| *s == 1.0));
    assert!(r[..FRAMES].iter().all(|s| *s == 1.0));
}

#[test]
fn each_block_sees_exactly_the_block_before_it() {
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);

    for block in 1..=5u32 {
        let v = block as f32;
        t.capture(SendSource::Track(10), &ramp(v), &ramp(v), FRAMES);
        t.begin_block(&routes);
        let (l, _) = t.key(SendSource::Track(10)).expect("a capture is available");
        assert_eq!(l[0], v, "block {} read the wrong capture", block + 1);
    }
}

#[test]
fn a_track_may_key_off_itself_without_feedback() {
    // Legal by construction: self-keying reads last block, so there is no
    // cycle to detect and nothing to refuse.
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(10), &ramp(0.5), &ramp(0.5), FRAMES);
    t.begin_block(&routes);
    assert_eq!(t.key_for(&routes, 1).map(|(l, _)| l[0]), Some(0.5));
}

#[test]
fn a_source_that_produced_nothing_keys_as_silence_not_stale_audio() {
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(10), &ramp(1.0), &ramp(1.0), FRAMES);
    t.begin_block(&routes);
    assert!(t.key(SendSource::Track(10)).is_some());

    // Next block: the source rendered nothing, so no capture happened.
    t.begin_block(&routes);
    assert!(
        t.key(SendSource::Track(10)).is_none(),
        "a silent block must not replay the block before it"
    );
}

#[test]
fn a_short_block_does_not_leave_a_longer_ones_tail_behind() {
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(10), &ramp(1.0), &ramp(1.0), FRAMES);
    t.begin_block(&routes);

    // Half-length block over the same slot.
    let half = vec![0.25f32; FRAMES / 2];
    t.capture(SendSource::Track(10), &half, &half, FRAMES / 2);
    t.begin_block(&routes);

    let (l, _) = t.key(SendSource::Track(10)).expect("capture");
    assert!(l[..FRAMES / 2].iter().all(|s| *s == 0.25));
    assert!(
        l[FRAMES / 2..].iter().all(|s| *s == 0.0),
        "the previous, longer block's tail leaked through"
    );
}

// ---------------------------------------------------------------------------
// Slot management
// ---------------------------------------------------------------------------

#[test]
fn only_routed_sources_are_tapped() {
    let routes = vec![from_track(1, 10)];
    let t = taps(&routes);
    assert!(t.is_tapped(SendSource::Track(10)));
    assert!(!t.is_tapped(SendSource::Track(11)));
    assert!(!t.is_tapped(SendSource::Bus(10)));
}

#[test]
fn capturing_an_untapped_source_is_a_no_op() {
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(99), &ramp(1.0), &ramp(1.0), FRAMES);
    t.begin_block(&routes);
    assert!(t.key(SendSource::Track(99)).is_none());
}

#[test]
fn an_unrelated_route_change_keeps_an_existing_capture() {
    // Slot assignment is stable: adding a second route must not drop the
    // first source's key for a block.
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(10), &ramp(0.75), &ramp(0.75), FRAMES);

    let grown = vec![from_track(1, 10), from_track(2, 20)];
    t.begin_block(&grown);
    assert_eq!(
        t.key(SendSource::Track(10)).map(|(l, _)| l[0]),
        Some(0.75),
        "adding an unrelated route dropped an existing capture"
    );
}

#[test]
fn dropping_a_route_frees_its_slot_for_a_new_source() {
    // Without the release the cap would be a high-water mark rather than
    // a concurrent limit, and a project that had churned through sources
    // would stop accepting new ones.
    let mut t = SidechainTaps::new(FRAMES);
    let first: Vec<_> = (0..MAX_SIDECHAIN_SOURCES)
        .map(|i| from_track(i as u64, 100 + i as u64))
        .collect();
    t.begin_block(&first);
    assert!(t.is_tapped(SendSource::Track(100)));

    // Replace every route with new sources.
    let second: Vec<_> = (0..MAX_SIDECHAIN_SOURCES)
        .map(|i| from_track(i as u64, 200 + i as u64))
        .collect();
    t.begin_block(&second);
    assert!(!t.is_tapped(SendSource::Track(100)), "old slot not released");
    assert!(t.is_tapped(SendSource::Track(200)), "new source not tapped");
}

#[test]
fn clear_drops_every_captured_signal() {
    // Transport stop must not let a key carry audio across the gap.
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(10), &ramp(1.0), &ramp(1.0), FRAMES);
    t.begin_block(&routes);
    assert!(t.key(SendSource::Track(10)).is_some());

    t.clear();
    assert!(t.key(SendSource::Track(10)).is_none());
}

#[test]
fn a_bus_source_is_tapped_independently_of_a_track_with_the_same_id() {
    // Track ids and bus ids are separate spaces; `SendSource` keeps them
    // apart, and the tap must too.
    let routes = vec![from_track(1, 7), from_bus(2, 7)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(7), &ramp(0.1), &ramp(0.1), FRAMES);
    t.capture(SendSource::Bus(7), &ramp(0.9), &ramp(0.9), FRAMES);
    t.begin_block(&routes);

    assert_eq!(t.key(SendSource::Track(7)).map(|(l, _)| l[0]), Some(0.1));
    assert_eq!(t.key(SendSource::Bus(7)).map(|(l, _)| l[0]), Some(0.9));
}
