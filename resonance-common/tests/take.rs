use resonance_common::take::{
    effective_cover, latest_take, Comp, CompSegment, CoverSource, CoverSpan, SlotCover, Take,
    TakeContent, TakeGroup, TakeNote, TimelineRange,
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

/// Builds a comp by promoting each `(start, end, take)` in order, with
/// [`SlotCover::NONE`] so only the segment surgery under test happens — these
/// comps are not covers of any slot. The seeding behaviour has its own tests
/// below.
fn comp_from(promotions: &[(u64, u64, u64)]) -> Comp {
    let mut comp = Comp::new();
    for &(start, end, take) in promotions {
        comp.promote(r(start, end), take, SlotCover::NONE);
    }
    comp
}

/// An audio take captured at `captured_at` with pass index `pass_index`.
fn audio_take(id: u64, pass_index: u32, captured_at: i64) -> Take {
    Take::new(
        id,
        pass_index,
        captured_at,
        TakeContent::Audio { clip_ref: 900 + id },
    )
}

/// A MIDI take carrying one note.
fn midi_take(id: u64, pass_index: u32, captured_at: i64) -> Take {
    Take::new(
        id,
        pass_index,
        captured_at,
        TakeContent::Midi {
            notes: vec![TakeNote {
                note: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
            }],
        },
    )
}

/// `(start, end, take_id, source)` per cover span — the readable shape for
/// asserting a whole cover at once.
fn cover_shape(spans: &[CoverSpan]) -> Vec<(u64, u64, u64, CoverSource)> {
    spans
        .iter()
        .map(|s| (s.range.start, s.range.end(), s.take_id, s.source))
        .collect()
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
    comp.promote(r(50, 50), 2, SlotCover::NONE);
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

    group.comp.promote(r(0, 120), 10, SlotCover::NONE);
    assert!(!group.is_full_cover());
    group.comp.promote(r(120, 200), 11, SlotCover::NONE);
    assert!(group.is_full_cover());
}

// --- latest_take (the #1395 ruling) --------------------------------------

/// "Latest" is the most recently *captured* pass, not the last element of
/// `takes`. A project that has been saved, reloaded and undone through can
/// present them in any order.
#[test]
fn latest_take_is_the_newest_capture_not_the_last_in_the_vector() {
    let mut group = TakeGroup::new(1, 42, r(0, 200));
    group.add_take(audio_take(10, 2, 3_000));
    group.add_take(audio_take(11, 0, 1_000));
    group.add_take(audio_take(12, 1, 2_000));

    assert_eq!(latest_take(&group).map(|t| t.id), Some(10));
}

/// **The ruling.** MIDI takes are candidates: "latest" is content-agnostic,
/// so a group holding both kinds resolves to whichever pass was captured
/// last. The engine used to answer "the last *audio* take in vector order"
/// and the take lane "the newest take of any kind" — the divergence #1395
/// exists to remove — and a lane lighting T2 while the mixer plays T1 is
/// exactly what excluding MIDI here would bring back.
#[test]
fn latest_take_counts_midi_takes_too() {
    let mut group = TakeGroup::new(1, 42, r(0, 200));
    group.add_take(audio_take(1, 0, 1_000));
    group.add_take(midi_take(2, 1, 2_000));

    assert_eq!(
        latest_take(&group).map(|t| t.id),
        Some(2),
        "the newest pass wins whether it is audio or MIDI"
    );

    // ...and the audio path's projection of that cover simply has no clip to
    // read for a MIDI span, the same by-design silence as soloing a MIDI
    // take. The cover itself still names the take the user last recorded.
    let cover = effective_cover(&group);
    assert_eq!(
        cover_shape(&cover),
        vec![(0, 200, 2, CoverSource::LatestFallback)]
    );
}

#[test]
fn latest_take_breaks_ties_deterministically() {
    // Same millisecond (the realistic collision): pass index, then id.
    let mut group = TakeGroup::new(1, 42, r(0, 200));
    group.add_take(audio_take(7, 1, 5_000));
    group.add_take(audio_take(3, 4, 5_000));
    assert_eq!(latest_take(&group).map(|t| t.id), Some(3));

    let mut group = TakeGroup::new(1, 42, r(0, 200));
    group.add_take(audio_take(7, 1, 5_000));
    group.add_take(audio_take(9, 1, 5_000));
    assert_eq!(latest_take(&group).map(|t| t.id), Some(9));

    assert!(latest_take(&TakeGroup::new(1, 42, r(0, 200))).is_none());
}

// --- effective_cover: the one resolution ---------------------------------

/// Tier 1: a soloed take covers the whole slot and the comp is ignored
/// entirely, however elaborate it is.
#[test]
fn cover_tier_1_active_take_overrides_the_comp() {
    let mut group = three_take_group();
    group.comp = comp_from(&[(0, 100, 0), (100, 300, 1)]);
    group.active_take = Some(0);

    assert_eq!(
        cover_shape(&effective_cover(&group)),
        vec![(0, 300, 0, CoverSource::ActiveTake)]
    );
}

/// An `active_take` the group does not hold is a corrupt/stale selection.
/// It is ignored rather than silencing the group — the tiers below it still
/// describe something audible.
#[test]
fn cover_ignores_an_active_take_the_group_does_not_hold() {
    let mut group = three_take_group();
    group.comp = comp_from(&[(0, 300, 1)]);
    group.active_take = Some(42);

    assert_eq!(
        cover_shape(&effective_cover(&group)),
        vec![(0, 300, 1, CoverSource::CompSegment)]
    );
}

/// Tier 3 alone: a group that has never been comped plays its newest pass
/// over the whole slot, marked as a fallback rather than a promotion.
#[test]
fn cover_tier_3_empty_comp_is_the_latest_take() {
    let group = three_take_group();
    assert_eq!(
        cover_shape(&effective_cover(&group)),
        vec![(0, 300, 2, CoverSource::LatestFallback)]
    );
}

/// **The user's ruling, at the level everything else reads.** One promoted
/// phrase in the middle of the slot leaves the most recent pass playing
/// either side of it — a complete part, not an island in a hole.
#[test]
fn cover_tier_3_fills_a_partial_comps_gaps_with_the_latest_take() {
    let mut group = three_take_group();
    group.comp = Comp {
        segments: vec![seg(100, 200, 0)],
    };

    assert_eq!(
        cover_shape(&effective_cover(&group)),
        vec![
            (0, 100, 2, CoverSource::LatestFallback),
            (100, 200, 0, CoverSource::CompSegment),
            (200, 300, 2, CoverSource::LatestFallback),
        ]
    );
}

/// Defensive clamping: the mirror adopts whatever the engine echoes, so an
/// unsorted comp naming a vanished take and overhanging the slot must still
/// resolve to an ordered, gap-free, in-slot cover.
#[test]
fn cover_sorts_clamps_and_skips_unknown_takes() {
    let mut group = three_take_group();
    group.comp = Comp {
        segments: vec![
            CompSegment {
                range: r(250, 400),
                take_id: 1,
            },
            CompSegment {
                range: r(0, 50),
                take_id: 99, // no such take
            },
            CompSegment {
                range: r(50, 120),
                take_id: 0,
            },
        ],
    };

    assert_eq!(
        cover_shape(&effective_cover(&group)),
        vec![
            (0, 50, 2, CoverSource::LatestFallback),
            (50, 120, 0, CoverSource::CompSegment),
            (120, 250, 2, CoverSource::LatestFallback),
            (250, 300, 1, CoverSource::CompSegment),
        ]
    );
}

#[test]
fn cover_is_empty_without_takes_or_slot() {
    assert!(effective_cover(&TakeGroup::new(1, 42, r(0, 300))).is_empty());

    let mut empty_slot = TakeGroup::new(1, 42, r(10, 10));
    empty_slot.add_take(audio_take(0, 0, 1_000));
    assert!(effective_cover(&empty_slot).is_empty());
}

// --- promote seeds a full cover -------------------------------------------

/// **The acceptance case.** Promoting one span into an empty comp yields a
/// gap-free cover of the slot, with the latest take holding the remainder.
/// The invariant doc #165 states is true by construction, so nothing
/// downstream has to reach for a fallback tier.
#[test]
fn promote_into_an_empty_comp_seeds_a_full_cover() {
    let mut group = three_take_group();
    let cover = SlotCover::of(&group);
    group.comp.promote(r(100, 200), 0, cover);

    assert_eq!(
        group.comp.segments,
        vec![seg(0, 100, 2), seg(100, 200, 0), seg(200, 300, 2)]
    );
    assert!(group.is_full_cover());
}

/// Seeding fills only what the comp leaves open, and a promote of the
/// filler itself merges rather than fragmenting.
#[test]
fn promote_seeds_only_the_gaps_and_merges() {
    let mut group = three_take_group();
    group.comp = Comp {
        segments: vec![seg(0, 100, 0)],
    };
    let cover = SlotCover::of(&group);
    group.comp.promote(r(250, 300), 2, cover);

    assert_eq!(
        group.comp.segments,
        vec![seg(0, 100, 0), seg(100, 300, 2)],
        "the seeded remainder and the promoted latest take coalesce"
    );
    assert!(group.is_full_cover());
}

/// A promote that changes nothing must not rewrite the comp — an empty
/// range is a no-op *including* the seeding, or a refused gesture would
/// silently materialize a cover and spend an undo entry.
#[test]
fn promote_of_an_empty_range_seeds_nothing() {
    let mut group = three_take_group();
    let cover = SlotCover::of(&group);
    group.comp.promote(r(150, 150), 0, cover);
    assert!(group.comp.segments.is_empty());
}

/// A promote reaching past the slot keeps its full range (callers clamp to
/// what a take can actually sound over); the seeding still covers the slot.
#[test]
fn promote_past_the_slot_still_covers_it() {
    let mut group = three_take_group();
    let cover = SlotCover::of(&group);
    group.comp.promote(r(200, 500), 1, cover);

    assert_eq!(group.comp.segments, vec![seg(0, 200, 2), seg(200, 500, 1)]);
    assert!(group.is_full_cover());
}

#[test]
fn seed_cover_without_a_filler_or_slot_is_a_noop() {
    let mut comp = comp_from(&[(100, 200, 0)]);
    let before = comp.clone();
    comp.seed_cover(SlotCover::NONE);
    comp.seed_cover(SlotCover {
        slot: r(0, 300),
        filler: None,
    });
    assert_eq!(comp, before);
}

// --- effective_comp: materializing what is audible ------------------------

/// Comp editing starts from what is *playing*. With a solo up that is the
/// soloed take across the whole slot — tier 1 — which is the fix for the
/// split-while-soloing case: the user hears take 0, hits "split", and keeps
/// hearing take 0.
#[test]
fn effective_comp_materializes_the_active_take_not_the_fallback() {
    let mut group = three_take_group();
    group.comp = comp_from(&[(0, 300, 1)]);
    group.active_take = Some(0);

    assert_eq!(group.effective_comp().segments, vec![seg(0, 300, 0)]);
}

/// The materialized comp is canonical: adjacent spans naming the same take
/// merge, so a fallback abutting a promotion of the same take does not leave
/// a phantom boundary behind for a later split to cut on.
#[test]
fn effective_comp_merges_adjacent_spans_of_one_take() {
    let mut group = three_take_group();
    group.comp = Comp {
        segments: vec![seg(100, 200, 2)],
    };

    assert_eq!(group.effective_comp().segments, vec![seg(0, 300, 2)]);
}

#[test]
fn effective_comp_of_a_takeless_group_is_empty() {
    assert!(TakeGroup::new(1, 42, r(0, 300))
        .effective_comp()
        .segments
        .is_empty());
}

/// Three audio takes over `[0, 300)`; take 2 is the newest pass.
fn three_take_group() -> TakeGroup {
    let mut group = TakeGroup::new(1, 42, r(0, 300));
    group.add_take(audio_take(0, 0, 1_000));
    group.add_take(audio_take(1, 1, 2_000));
    group.add_take(audio_take(2, 2, 3_000));
    group
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
    group.comp.promote(r(0, 120), 1, SlotCover::NONE);
    group.comp.promote(r(120, 240), 2, SlotCover::NONE);
    group.active_take = Some(2);

    let json = serde_json::to_string(&group).unwrap();
    let back: TakeGroup = serde_json::from_str(&json).unwrap();
    assert_eq!(group, back);
}
