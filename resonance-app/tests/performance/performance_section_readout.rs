//! Headless tests for the Performance-mode arrangement-section readout
//! (epic #11, todo #372): mapping the transport playhead onto the arrangement
//! markers to report the current and next section name, loop-aware (the same
//! windowing the chord look-ahead uses).

use resonance_app::engine_events::performance::{section_readout, SectionReadout};
use resonance_app::state::ArrangementMarker;

fn region(id: u64, name: &str, start: u64, end: u64) -> ArrangementMarker {
    ArrangementMarker::new_region(id, name.to_string(), [10, 20, 30], start, end)
}

fn point(id: u64, name: &str, start: u64) -> ArrangementMarker {
    ArrangementMarker::new_point(id, name.to_string(), [10, 20, 30], start)
}

/// Names of the current / next sections, for terse assertions.
fn names(r: &SectionReadout) -> (Option<&str>, Option<&str>) {
    (
        r.current.as_ref().map(|s| s.name.as_str()),
        r.next.as_ref().map(|s| s.name.as_str()),
    )
}

/// Three back-to-back section regions: Intro [0,100), Verse [100,200),
/// Chorus [200,300).
fn three_regions() -> Vec<ArrangementMarker> {
    vec![
        region(1, "Intro", 0, 99),
        region(2, "Verse", 100, 199),
        region(3, "Chorus", 200, 299),
    ]
}

#[test]
fn empty_arrangement_has_no_sections() {
    let r = section_readout(&[], 1_000, None);
    assert_eq!(names(&r), (None, None));
}

#[test]
fn current_and_next_at_several_positions() {
    let markers = three_regions();

    // Inside Intro.
    assert_eq!(
        names(&section_readout(&markers, 50, None)),
        (Some("Intro"), Some("Verse"))
    );
    // Inside Verse.
    assert_eq!(
        names(&section_readout(&markers, 150, None)),
        (Some("Verse"), Some("Chorus"))
    );
    // Exactly on a region's inclusive end stays in that section.
    assert_eq!(
        names(&section_readout(&markers, 199, None)),
        (Some("Verse"), Some("Chorus"))
    );
    // On the boundary sample the later section takes over.
    assert_eq!(
        names(&section_readout(&markers, 200, None)),
        (Some("Chorus"), None)
    );
    // Inside the last section: nothing comes next.
    assert_eq!(
        names(&section_readout(&markers, 250, None)),
        (Some("Chorus"), None)
    );
}

#[test]
fn gap_between_regions_has_no_current_but_reports_next() {
    // Intro ends at 50, Verse only starts at 100 — a gap in between.
    let markers = vec![region(1, "Intro", 0, 50), region(2, "Verse", 100, 199)];
    assert_eq!(
        names(&section_readout(&markers, 70, None)),
        (None, Some("Verse"))
    );
}

#[test]
fn point_markers_stay_current_until_the_next_one() {
    // Point flags delimit sections: A@0, B@100, C@200.
    let markers = vec![point(1, "A", 0), point(2, "B", 100), point(3, "C", 200)];

    assert_eq!(
        names(&section_readout(&markers, 50, None)),
        (Some("A"), Some("B"))
    );
    assert_eq!(
        names(&section_readout(&markers, 100, None)),
        (Some("B"), Some("C"))
    );
    // Past the last flag: still current, nothing next.
    assert_eq!(
        names(&section_readout(&markers, 250, None)),
        (Some("C"), None)
    );
}

#[test]
fn next_wraps_at_loop_end_back_to_the_first_section() {
    let markers = three_regions();
    // Loop the whole song [0, 300). At the playhead inside Chorus there is no
    // later section before loop_out, so "next" wraps to the loop's first
    // section (Intro).
    assert_eq!(
        names(&section_readout(&markers, 250, Some((0, 300)))),
        (Some("Chorus"), Some("Intro"))
    );
}

#[test]
fn loop_wrap_respects_a_partial_loop_window() {
    let markers = three_regions();
    // Loop only Verse+Chorus [100, 300). Inside Chorus, the next wraps to the
    // first section at/before the playhead within the loop (Verse), not Intro
    // (which sits before loop_in).
    assert_eq!(
        names(&section_readout(&markers, 250, Some((100, 300)))),
        (Some("Chorus"), Some("Verse"))
    );
}

#[test]
fn forward_next_inside_loop_is_preferred_over_wrap() {
    let markers = three_regions();
    // Looping the whole song, but standing in Verse there is still a later
    // section (Chorus) before the loop end — no wrap needed.
    assert_eq!(
        names(&section_readout(&markers, 150, Some((0, 300)))),
        (Some("Verse"), Some("Chorus"))
    );
}

#[test]
fn disabled_loop_does_not_wrap() {
    let markers = three_regions();
    // loop_out <= loop_in means "no loop window": never wraps.
    assert_eq!(
        names(&section_readout(&markers, 250, Some((300, 300)))),
        (Some("Chorus"), None)
    );
}

#[test]
fn unsorted_input_is_handled() {
    // Same three regions, shuffled — the readout must sort internally.
    let markers = vec![
        region(3, "Chorus", 200, 299),
        region(1, "Intro", 0, 99),
        region(2, "Verse", 100, 199),
    ];
    assert_eq!(
        names(&section_readout(&markers, 150, None)),
        (Some("Verse"), Some("Chorus"))
    );
}

#[test]
fn readout_carries_id_and_color() {
    let markers = vec![ArrangementMarker::new_region(
        7,
        "Bridge".to_string(),
        [200, 100, 50],
        0,
        100,
    )];
    let r = section_readout(&markers, 50, None);
    let current = r.current.expect("current section");
    assert_eq!(current.id, 7);
    assert_eq!(current.name, "Bridge");
    assert_eq!(current.color, [200, 100, 50]);
}
