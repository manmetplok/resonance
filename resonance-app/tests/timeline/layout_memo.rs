//! Null test for the arrange-layout memo (view-performance batch).
//!
//! `TimelineCanvas::arrange_layout()` used to rebuild the sorted,
//! collapse-aware row layout 2-3× per frame (cached pass, overlay pass,
//! hover, pointer, `content_height_px`); it is now memoized per canvas
//! instance. The memo is safe because everything the layout reads sits
//! behind the canvas's `&` borrows — but that argument is only as good
//! as the memo returning *exactly* what an uncached build returns, which
//! is what these tests pin across progressively richer states.

use resonance_app::message::{AutomationMessage, Message};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

fn assert_memo_is_null(app: &Resonance, state: &str) {
    let (memoized, fresh) = app.test_timeline_layout_memo_pair();
    assert_eq!(
        memoized, fresh,
        "memoized arrange layout diverged from an uncached build ({state})"
    );
    // The header column builds the same layout through its own path;
    // the canvas memo must agree with it too.
    assert_eq!(
        memoized,
        app.test_arrange_row_layout(),
        "canvas memo diverged from the header column's layout ({state})"
    );
}

#[test]
fn the_layout_memo_matches_an_uncached_build_across_states() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    // Without an active project the update gate drops every edit message.
    app.test_set_active_project(true);
    assert_memo_is_null(&app, "empty project");

    for id in 1..=4 {
        app.test_add_track(id, TrackType::Instrument);
    }
    app.test_add_track(5, TrackType::Audio);
    assert_memo_is_null(&app, "five plain tracks");

    // A collapsed group hides its member rows and interleaves a
    // group-header row — the layout shape the memo must not distort.
    let mut group = TrackGroup::new(9, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![1, 2];
    group.is_collapsed = true;
    app.test_track_groups_mut().add_group(group);
    assert_memo_is_null(&app, "collapsed group");

    // Expanded automation sub-rows reshape the rows below their track.
    let _ = app.update(Message::Automation(AutomationMessage::ToggleTrackExpanded(3)));
    assert_memo_is_null(&app, "automation rows expanded");
}
