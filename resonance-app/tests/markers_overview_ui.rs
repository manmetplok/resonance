//! UI wiring for the arrangement-markers overview + transport marker
//! navigation (todo #370 / doc #161).
//!
//! Covers the view-layer additions on top of the `MarkerMessage` reducers
//! (todo #367): the overview popover open/close toggle, the overview
//! click-to-jump path (`MarkerMessage::JumpTo`), and the focus-gated
//! next/prev-marker keyboard shortcut (`UiMessage::MarkerNavResolved`).
//! The reducers move the transport playhead in lockstep with the `SeekTo`
//! command they send the engine, so the mirrored playhead is the
//! observable proxy for a correct jump (same convention the reducer tests
//! use).

use resonance_app::message::{MarkerMessage, Message, TransportMessage, UiMessage};
use resonance_app::state::ArrangementMarker;
use resonance_app::Resonance;

const SAMPLE_RATE: u32 = 48_000;

/// A fresh app with an active project (so marker / UI messages aren't
/// swallowed by the startup-modal gate) at a deterministic 120 BPM grid.
fn app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SAMPLE_RATE);
    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("120".into())));
    let _ = app.update(Message::Transport(TransportMessage::CommitBpm));
    app
}

/// Seed three point markers at ascending, on-grid sample positions.
fn seed_three(app: &mut Resonance) {
    for (i, (name, start)) in [("Intro", 48_000u64), ("Verse", 96_000), ("Chorus", 192_000)]
        .into_iter()
        .enumerate()
    {
        app.test_add_marker(ArrangementMarker::new_point(
            (i + 1) as u64,
            name.to_string(),
            [10, 20, 30],
            start,
        ));
    }
}

#[test]
fn overview_toggle_opens_and_closes() {
    let mut app = app();
    assert!(!app.test_markers_overview_open(), "closed by default");

    let _ = app.update(Message::Ui(UiMessage::ToggleMarkersOverview));
    assert!(app.test_markers_overview_open(), "toggle opens the overview");

    let _ = app.update(Message::Ui(UiMessage::ToggleMarkersOverview));
    assert!(!app.test_markers_overview_open(), "toggle again closes it");
}

#[test]
fn close_overview_message_dismisses_it() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::ToggleMarkersOverview));
    assert!(app.test_markers_overview_open());

    // Backdrop click routes CloseMarkersOverview.
    let _ = app.update(Message::Ui(UiMessage::CloseMarkersOverview));
    assert!(!app.test_markers_overview_open());
}

#[test]
fn overview_entry_click_seeks_playhead() {
    let mut app = app();
    seed_three(&mut app);
    // Clicking the "Chorus" entry (id 3) seeks the playhead to its start.
    let _ = app.update(Message::Marker(MarkerMessage::JumpTo(3)));
    assert_eq!(app.test_playhead(), 192_000);

    // The overview stays open across jumps so several sections can be
    // auditioned without reopening it.
    let _ = app.update(Message::Ui(UiMessage::ToggleMarkersOverview));
    let _ = app.update(Message::Marker(MarkerMessage::JumpTo(1)));
    assert_eq!(app.test_playhead(), 48_000);
    assert!(app.test_markers_overview_open());
}

#[test]
fn marker_nav_shortcut_jumps_when_not_editing() {
    let mut app = app();
    seed_three(&mut app);
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(60_000)));

    // Next-marker (`.`) with no focused text field jumps to the following
    // marker (Chorus/Verse boundary after 60k is Verse @96k).
    let _ = app.update(Message::Ui(UiMessage::MarkerNavResolved {
        forward: true,
        editing: false,
    }));
    assert_eq!(app.test_playhead(), 96_000);

    // Prev-marker (`,`) jumps back to Intro @48k.
    let _ = app.update(Message::Ui(UiMessage::MarkerNavResolved {
        forward: false,
        editing: false,
    }));
    assert_eq!(app.test_playhead(), 48_000);
}

#[test]
fn marker_nav_shortcut_suppressed_while_editing() {
    let mut app = app();
    seed_three(&mut app);
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(60_000)));

    // A period typed into a focused text field must not move the playhead.
    let _ = app.update(Message::Ui(UiMessage::MarkerNavResolved {
        forward: true,
        editing: true,
    }));
    assert_eq!(app.test_playhead(), 60_000, "editing suppresses nav");
}
