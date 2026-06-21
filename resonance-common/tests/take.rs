use resonance_common::take::{
    Comp, CompSegment, Take, TakeContent, TakeGroup, TakeNote, TimelineRange,
};

fn r(start: u64, end: u64) -> TimelineRange {
    TimelineRange::from_bounds(start, end)
}

fn seg(start: u64, end: u64, take_id: u64) -> CompSegment {
    CompSegment {
        range: r(start, end),
        take_id,
    }
}

/// Builds a comp by promoting each `(start, end, take)` in order.
fn comp_from(promotions: &[(u64, u64, u64)]) -> Comp {
    let mut comp = Comp::new();
    for &(start, end, take) in promotions {
        comp.promote(r(start, end), take);
    }
    comp
}

// --- TimelineRange -------------------------------------------------------

#[test]
fn range_basics() {
    let range = TimelineRange::new(100, 50);
    assert_eq!(range.start, 100);
    assert_eq!(range.length, 50);
    assert_eq!(range.end(), 150);
    assert!(!range.is_empty());
    assert!(range.contains(100));
    assert!(range.contains(149));
    assert!(!range.contains(150)); // half-open
    assert!(!range.contains(99));
}

#[test]
fn from_bounds_handles_reversed_and_equal() {
    assert_eq!(TimelineRange::from_bounds(10, 30), TimelineRange::new(10, 20));
    assert!(TimelineRange::from_bounds(30, 10).is_empty()); // saturating
    assert!(TimelineRange::from_bounds(10, 10).is_empty());
}

#[test]
fn overlaps_is_symmetric_and_excludes_touching() {
    let a = r(0, 100);
    let b = r(50, 150);
    let c = r(100, 200); // abuts a, no shared position
    assert!(a.overlaps(&b) && b.overlaps(&a));
    assert!(!a.overlaps(&c) && !c.overlaps(&a));
}

// --- comp_at -------------------------------------------------------------

#[test]
fn comp_at_finds_covering_segment() {
    let comp = comp_from(&[(0, 100, 1), (100, 200, 2)]);
    assert_eq!(comp.comp_at(0).map(|s| s.take_id), Some(1));
    assert_eq!(comp.comp_at(99).map(|s| s.take_id), Some(1));
    assert_eq!(comp.comp_at(100).map(|s| s.take_id), Some(2));
    assert_eq!(comp.comp_at(199).map(|s| s.take_id), Some(2));
    assert!(comp.comp_at(200).is_none());
    assert!(Comp::new().comp_at(0).is_none());
}

// --- split_comp ----------------------------------------------------------

#[test]
fn split_comp_cuts_interior_keeping_take() {
    let mut comp = comp_from(&[(0, 200, 7)]);
    comp.split_comp(80);
    assert_eq!(comp.segments, vec![seg(0, 80, 7), seg(80, 200, 7)]);
    // Both halves still resolve to the same take.
    assert_eq!(comp.comp_at(79).map(|s| s.take_id), Some(7));
    assert_eq!(comp.comp_at(80).map(|s| s.take_id), Some(7));
}

#[test]
fn split_comp_noop_on_boundary_or_outside() {
    let mut comp = comp_from(&[(0, 100, 1), (100, 200, 2)]);
    let before = comp.clone();
    comp.split_comp(0); // start boundary
    comp.split_comp(100); // shared boundary
    comp.split_comp(200); // exclusive end / outside
    comp.split_comp(999); // outside
    assert_eq!(comp, before);
}

// --- promote -------------------------------------------------------------

#[test]
fn promote_onto_empty_inserts_segment() {
    let comp = comp_from(&[(10, 60, 3)]);
    assert_eq!(comp.segments, vec![seg(10, 60, 3)]);
}

#[test]
fn promote_empty_range_is_noop() {
    let mut comp = comp_from(&[(0, 100, 1)]);
    let before = comp.clone();
    comp.promote(r(50, 50), 2);
    assert_eq!(comp, before);
}

#[test]
fn promote_inside_splits_surrounding_segment() {
    // take 2 painted into the middle of take 1 -> 1 | 2 | 1.
    let comp = comp_from(&[(0, 300, 1), (100, 200, 2)]);
    assert_eq!(
        comp.segments,
        vec![seg(0, 100, 1), seg(100, 200, 2), seg(200, 300, 1)]
    );
}

#[test]
fn promote_trims_partial_overlaps_on_both_sides() {
    // take 3 spans the seam between take 1 and take 2.
    let comp = comp_from(&[(0, 100, 1), (100, 200, 2), (50, 150, 3)]);
    assert_eq!(
        comp.segments,
        vec![seg(0, 50, 1), seg(50, 150, 3), seg(150, 200, 2)]
    );
}

#[test]
fn promote_fully_replaces_covered_segments() {
    let comp = comp_from(&[(0, 100, 1), (100, 200, 2), (200, 300, 3), (0, 300, 9)]);
    assert_eq!(comp.segments, vec![seg(0, 300, 9)]);
}

#[test]
fn promote_merges_adjacent_same_take() {
    // Painting take 1 next to existing take 1 coalesces into one segment.
    let comp = comp_from(&[(0, 100, 1), (100, 200, 1)]);
    assert_eq!(comp.segments, vec![seg(0, 200, 1)]);

    // Re-promoting the same take over a gap-filling middle merges both seams.
    let comp = comp_from(&[(0, 100, 1), (200, 300, 1), (100, 200, 1)]);
    assert_eq!(comp.segments, vec![seg(0, 300, 1)]);
}

#[test]
fn promote_keeps_segments_sorted() {
    let comp = comp_from(&[(200, 300, 2), (0, 100, 1), (100, 200, 3)]);
    let starts: Vec<u64> = comp.segments.iter().map(|s| s.range.start).collect();
    assert_eq!(starts, vec![0, 100, 200]);
}

// --- is_full_cover -------------------------------------------------------

#[test]
fn is_full_cover_detects_complete_and_gappy_covers() {
    let slot = r(0, 300);

    let full = comp_from(&[(0, 300, 1)]);
    assert!(full.is_full_cover(slot));

    let stitched = comp_from(&[(0, 100, 1), (100, 200, 2), (200, 300, 3)]);
    assert!(stitched.is_full_cover(slot));

    let gap = comp_from(&[(0, 100, 1), (200, 300, 3)]);
    assert!(!gap.is_full_cover(slot));

    let short_start = comp_from(&[(50, 300, 1)]);
    assert!(!short_start.is_full_cover(slot));

    let short_end = comp_from(&[(0, 250, 1)]);
    assert!(!short_end.is_full_cover(slot));

    // Coverage extending past the slot still counts as full.
    let overshoot = comp_from(&[(0, 500, 1)]);
    assert!(overshoot.is_full_cover(slot));

    // Empty slot is trivially covered.
    assert!(Comp::new().is_full_cover(r(10, 10)));
}

// --- TakeGroup -----------------------------------------------------------

#[test]
fn take_group_lookup_and_full_cover() {
    let mut group = TakeGroup::new(1, 42, r(0, 200));
    assert!(group.takes.is_empty());
    assert!(group.active_take.is_none());
    assert!(!group.is_full_cover());

    group.add_take(Take::new(10, 0, 1_000, TakeContent::Audio { clip_ref: 555 }));
    group.add_take(Take::new(
        11,
        1,
        2_000,
        TakeContent::Midi {
            notes: vec![TakeNote {
                note: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
            }],
        },
    ));

    assert_eq!(group.take(10).map(|t| t.pass_index), Some(0));
    assert_eq!(group.take(11).map(|t| t.pass_index), Some(1));
    assert!(group.take(99).is_none());

    group.comp.promote(r(0, 120), 10);
    assert!(!group.is_full_cover());
    group.comp.promote(r(120, 200), 11);
    assert!(group.is_full_cover());
}

// --- serde round-trip ----------------------------------------------------

#[test]
fn take_group_serde_round_trips() {
    let mut group = TakeGroup::new(7, 3, r(0, 240));
    group.add_take(Take::new(1, 0, 111, TakeContent::Audio { clip_ref: 9 }));
    group.add_take(Take::new(
        2,
        1,
        222,
        TakeContent::Midi {
            notes: vec![
                TakeNote {
                    note: 64,
                    velocity: 1.0,
                    start_tick: 0,
                    duration_ticks: 240,
                },
                TakeNote {
                    note: 67,
                    velocity: 0.5,
                    start_tick: 240,
                    duration_ticks: 240,
                },
            ],
        },
    ));
    group.comp.promote(r(0, 120), 1);
    group.comp.promote(r(120, 240), 2);
    group.active_take = Some(2);

    let json = serde_json::to_string(&group).unwrap();
    let back: TakeGroup = serde_json::from_str(&json).unwrap();
    assert_eq!(group, back);
}
