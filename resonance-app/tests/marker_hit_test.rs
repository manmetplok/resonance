//! Pure hit-testing coverage for arrangement-marker ruler interaction
//! (todo #369). Exercises `view::timeline::hit_test::marker_hit` — the grab
//! geometry the ruler input handlers use to decide select / move / resize.

use resonance_app::view::timeline::hit_test::{
    marker_hit, MarkerHit, MARKER_EDGE_THRESHOLD, MARKER_FLAG_PAD, MARKER_FLAG_W,
};

#[test]
fn point_marker_flag_zone_is_grabbable() {
    // A point marker (no end) exposes a flag grab zone anchored on the pole.
    let start = 100.0;
    // Dead-centre on the pole.
    assert_eq!(marker_hit(start, start, None), Some(MarkerHit::Flag));
    // Just left of the pole, within the pad.
    assert_eq!(
        marker_hit(start - MARKER_FLAG_PAD + 0.5, start, None),
        Some(MarkerHit::Flag)
    );
    // Out to the right edge of the pennant, within the pad.
    assert_eq!(
        marker_hit(start + MARKER_FLAG_W + MARKER_FLAG_PAD - 0.5, start, None),
        Some(MarkerHit::Flag)
    );
}

#[test]
fn point_marker_misses_outside_the_flag() {
    let start = 100.0;
    // Well left of the pad.
    assert_eq!(marker_hit(start - MARKER_FLAG_PAD - 5.0, start, None), None);
    // Past the pennant + pad.
    assert_eq!(
        marker_hit(start + MARKER_FLAG_W + MARKER_FLAG_PAD + 5.0, start, None),
        None
    );
}

#[test]
fn region_end_edge_wins_the_resize_zone() {
    let start = 100.0;
    let end = 400.0;
    // Right on the end edge → resize handle.
    assert_eq!(
        marker_hit(end, start, Some(end)),
        Some(MarkerHit::EndEdge)
    );
    // Within the edge band.
    assert_eq!(
        marker_hit(end - MARKER_EDGE_THRESHOLD + 0.5, start, Some(end)),
        Some(MarkerHit::EndEdge)
    );
    // Just outside the band and away from the flag → the wide translucent
    // body is not a grab target, so it misses (ruler seek stays usable).
    assert_eq!(
        marker_hit(end - MARKER_EDGE_THRESHOLD - 20.0, start, Some(end)),
        None
    );
}

#[test]
fn region_start_flag_still_moves_it() {
    // A region's start pole moves the whole marker (MoveStart), same as a
    // point marker's flag.
    let start = 100.0;
    let end = 400.0;
    assert_eq!(marker_hit(start, start, Some(end)), Some(MarkerHit::Flag));
}

#[test]
fn narrow_region_prefers_end_edge_over_start() {
    // When start and end are so close their zones overlap, the end edge
    // wins so a tiny region can still be resized rather than only moved.
    let start = 100.0;
    let end = 102.0;
    assert_eq!(
        marker_hit(end, start, Some(end)),
        Some(MarkerHit::EndEdge)
    );
}
