//! Sidechain key capture (`types::sidechain`).
//!
//! The routing model's whole correctness argument is the double buffer:
//! a key always reads the PREVIOUS block's capture, so the result cannot
//! depend on the order tracks happen to render in, and a track keying off
//! itself (or off something downstream of itself) is legal rather than a
//! cycle. These tests pin that, plus the bounded slot allocation that
//! keeps the audio thread allocation-free.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use resonance_audio::types::sidechain::{active_sources, from_bus, from_track, route_source};
use resonance_audio::types::{
    SendSource, SidechainRoute, SidechainTaps, MAX_SIDECHAIN_SOURCES,
};

/// Counts this thread's heap allocations so
/// `begin_block_never_allocates` can prove the per-callback path stays
/// allocation-free (the module's founding invariant). Thread-local so
/// concurrently running tests in this binary don't pollute the count.
struct CountingAllocator;

thread_local! {
    static THREAD_ALLOCS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        THREAD_ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

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
    assert_eq!(active_sources(&routes).as_slice(), &[SendSource::Track(10)]);

    // More distinct sources than slots: the excess is dropped rather than
    // misrouted onto someone else's buffer.
    let many: Vec<_> = (1..=20).map(|p| from_track(p, p + 100)).collect();
    let sources = active_sources(&many);
    assert_eq!(sources.len(), MAX_SIDECHAIN_SOURCES);
    // First-seen order, so which sources survive is deterministic.
    assert_eq!(sources.as_slice()[0], SendSource::Track(101));
}

#[test]
fn active_sources_keeps_first_seen_order_across_kinds_and_gaps() {
    // Disabled routes are skipped without claiming an order position;
    // a track and a bus with the same numeric id stay distinct; and a
    // duplicate later in the list doesn't reorder its first sighting.
    let routes = vec![
        SidechainRoute {
            plugin: 1,
            source: SendSource::Track(7),
            enabled: false,
        },
        from_bus(2, 7),
        from_track(3, 7),
        from_bus(4, 7), // duplicate of the enabled bus route
        from_track(5, 9),
    ];
    assert_eq!(
        active_sources(&routes).as_slice(),
        &[SendSource::Bus(7), SendSource::Track(7), SendSource::Track(9)]
    );
    let sources = active_sources(&routes);
    assert!(sources.contains(SendSource::Track(7)));
    assert!(!sources.contains(SendSource::Track(8)));
    assert!(
        !sources.contains(SendSource::Track(0)),
        "the fixed array's filler value must not read as a member"
    );
    assert_eq!(sources.iter().collect::<Vec<_>>(), sources.as_slice());
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
fn begin_block_never_allocates() {
    // The playing branch calls `begin_block` once per audio callback;
    // per sidechain.rs "the audio thread never allocates". Exercise the
    // worst case — a full slot table churning to entirely new sources
    // every block, routes past the cap included — and require zero heap
    // allocations on this thread across the calls.
    let routes_a: Vec<_> = (1..=20).map(|p| from_track(p, p + 100)).collect();
    let routes_b: Vec<_> = (1..=20).map(|p| from_track(p, p + 200)).collect();
    let mut t = SidechainTaps::new(FRAMES);
    t.begin_block(&routes_a);

    let before = THREAD_ALLOCS.with(|c| c.get());
    for _ in 0..4 {
        t.begin_block(&routes_b);
        t.begin_block(&routes_a);
    }
    let after = THREAD_ALLOCS.with(|c| c.get());
    assert_eq!(after, before, "begin_block allocated on the audio-thread path");
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

#[test]
fn seam_sub_blocks_capture_and_read_at_their_own_offset() {
    // RT-06: a loop-seam callback renders a head and a tail sub-block
    // against one bank pair. The tail captures after the head (not over
    // it) and reads the previous callback's key from the same position.
    const HEAD: usize = 40;
    let routes = vec![from_track(1, 10)];
    let src: Vec<f32> = (0..FRAMES).map(|i| i as f32).collect();
    let mut t = taps(&routes);
    t.set_sub_block_offset(0);
    t.capture(SendSource::Track(10), &src[..HEAD], &src[..HEAD], HEAD);
    t.set_sub_block_offset(HEAD);
    t.capture(SendSource::Track(10), &src[HEAD..], &src[HEAD..], FRAMES - HEAD);

    t.begin_block(&routes);
    let (l, _) = t.key(SendSource::Track(10)).unwrap();
    assert_eq!(&l[..FRAMES], &src[..], "the whole callback, contiguous");
    t.set_sub_block_offset(HEAD);
    let (l, _) = t.key(SendSource::Track(10)).unwrap();
    assert_eq!(l[0], HEAD as f32, "the tail reads from its own frames");
    assert_eq!(l.len(), FRAMES - HEAD);
}

#[test]
fn a_source_first_captured_in_the_tail_keys_silence_before_it() {
    // A source that renders only after the seam must not leave the head
    // frames holding audio from two callbacks ago.
    const HEAD: usize = 20;
    let routes = vec![from_track(1, 10)];
    let mut t = taps(&routes);
    t.capture(SendSource::Track(10), &ramp(0.7), &ramp(0.7), FRAMES);
    t.begin_block(&routes);
    t.begin_block(&routes); // back on the bank holding the 0.7 capture
    t.set_sub_block_offset(HEAD);
    t.capture(SendSource::Track(10), &ramp(0.2), &ramp(0.2), FRAMES - HEAD);
    t.begin_block(&routes);
    let (l, _) = t.key(SendSource::Track(10)).unwrap();
    assert!(l[..HEAD].iter().all(|&s| s == 0.0), "head: silence");
    assert!(l[HEAD..FRAMES].iter().all(|&s| s == 0.2), "tail: this capture");
}
