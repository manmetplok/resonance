//! Reducer coverage for the `MarkerMessage` group (todo #367 / doc #161).
//!
//! These drive the `update/marker.rs` handlers through the public
//! `Resonance::update` entry point and assert the resulting state. Marker
//! navigation reducers move the transport playhead / loop range in
//! lockstep with the `SeekTo` / `SetLoopRange` commands they send to the
//! engine; the engine itself has no test-side command capture, so the
//! mirrored transport state is the observable proxy for "the right
//! command was sent" (the same convention the transport reducer tests
//! use). Undo behaviour is asserted at the classifier level — markers
//! ride the `ProjectFile` snapshot/replay path, so a `Record`
//! classification is exactly what produces an undo entry.

use resonance_app::message::{MarkerMessage, Message, TransportMessage};
use resonance_app::state::ArrangementMarker;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;

const SAMPLE_RATE: u32 = 48_000;
const ZOOM: f32 = 200.0;
/// At 120 BPM / 48 kHz the grid is one beat = 24 000 samples, and the
/// high zoom forces beat-resolution snapping.
const SAMPLES_PER_BEAT: u64 = 24_000;

/// A fresh app with an active project (so marker / transport messages
/// aren't swallowed by the startup-modal gate) and a deterministic
/// 120 BPM grid for the snap-sensitive reducers.
fn marker_app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SAMPLE_RATE);
    app.test_set_arrange_zoom(ZOOM);
    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("120".into())));
    let _ = app.update(Message::Transport(TransportMessage::CommitBpm));
    app
}

/// Seed a point marker at an exact sample position, bypassing the
/// `AddAtPlayhead` snap path. Uses the start sample as a stable id so
/// seeds at distinct positions never collide.
fn marker(app: &mut Resonance, name: &str, start: u64) -> u64 {
    app.test_add_marker(ArrangementMarker::new_point(
        start.max(1),
        name.to_string(),
        [10, 20, 30],
        start,
    ))
}

// ---------------- Mutating reducers ----------------

#[test]
fn add_at_playhead_drops_a_snapped_point_marker() {
    let mut app = marker_app();
    // Seek off-grid: 25 000 is just past beat 1 (24 000).
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(25_000)));
    let _ = app.update(Message::Marker(MarkerMessage::AddAtPlayhead));

    let markers = app.test_markers();
    assert_eq!(markers.len(), 1);
    let m = &markers.as_slice()[0];
    assert!(m.is_point(), "AddAtPlayhead should create a point marker");
    assert_eq!(
        m.start_sample, SAMPLES_PER_BEAT,
        "playhead should snap to the nearest beat (24 000), not stay at 25 000"
    );
}

#[test]
fn add_at_playhead_cycles_palette_colours() {
    let mut app = marker_app();
    for _ in 0..3 {
        let _ = app.update(Message::Marker(MarkerMessage::AddAtPlayhead));
    }
    let colours: Vec<[u8; 3]> = app
        .test_markers()
        .as_slice()
        .iter()
        .map(|m| m.color)
        .collect();
    assert_eq!(colours.len(), 3);
    // Three consecutive adds must yield three distinct palette colours.
    assert_ne!(colours[0], colours[1]);
    assert_ne!(colours[1], colours[2]);
    assert_ne!(colours[0], colours[2]);
}

#[test]
fn rename_and_recolor_edit_the_marker() {
    let mut app = marker_app();
    let id = marker(&mut app, "A", 1_000);

    let _ = app.update(Message::Marker(MarkerMessage::Rename(id, "Chorus".into())));
    let _ = app.update(Message::Marker(MarkerMessage::Recolor(id, [1, 2, 3])));

    let m = app.test_markers().get(id).expect("marker should still exist");
    assert_eq!(m.name, "Chorus");
    assert_eq!(m.color, [1, 2, 3]);

    // Editing a missing id is a no-op (no panic, no spurious marker).
    let _ = app.update(Message::Marker(MarkerMessage::Rename(999, "x".into())));
    assert_eq!(app.test_markers().len(), 1);
}

#[test]
fn delete_removes_the_marker() {
    let mut app = marker_app();
    let id = marker(&mut app, "A", 1_000);
    assert_eq!(app.test_markers().len(), 1);

    let _ = app.update(Message::Marker(MarkerMessage::Delete(id)));
    assert!(app.test_markers().is_empty());

    // Deleting an absent id is harmless.
    let _ = app.update(Message::Marker(MarkerMessage::Delete(id)));
    assert!(app.test_markers().is_empty());
}

#[test]
fn move_start_snaps_and_resorts() {
    let mut app = marker_app();
    let a = marker(&mut app, "A", 0);
    let _b = marker(&mut app, "B", 4 * SAMPLES_PER_BEAT); // 96 000, on-grid

    // Move A past B to ~5 beats; off-grid 121 000 snaps to beat 5.
    let _ = app.update(Message::Marker(MarkerMessage::MoveStart(a, 121_000)));

    let markers = app.test_markers();
    assert_eq!(
        markers.get(a).unwrap().start_sample,
        5 * SAMPLES_PER_BEAT,
        "MoveStart should snap to the nearest beat"
    );
    // Collection must re-sort: B (96 000) now precedes A (120 000).
    let order: Vec<u64> = markers.as_slice().iter().map(|m| m.id).collect();
    assert_eq!(order, vec![_b, a], "markers must stay sorted by start");
}

#[test]
fn set_region_end_toggles_point_and_region() {
    let mut app = marker_app();
    let id = marker(&mut app, "Verse", 1_000);
    assert!(app.test_markers().get(id).unwrap().is_point());

    let _ = app.update(Message::Marker(MarkerMessage::SetRegionEnd(id, Some(5_000))));
    let m = app.test_markers().get(id).unwrap();
    assert!(m.is_region());
    assert_eq!(m.end_sample, Some(5_000));

    let _ = app.update(Message::Marker(MarkerMessage::SetRegionEnd(id, None)));
    assert!(app.test_markers().get(id).unwrap().is_point());
}

// ---------------- Navigation reducers ----------------

#[test]
fn jump_to_next_and_prev_move_the_playhead() {
    let mut app = marker_app();
    marker(&mut app, "A", 1_000);
    marker(&mut app, "B", 5_000);
    marker(&mut app, "C", 9_000);

    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(2_000)));
    let _ = app.update(Message::Marker(MarkerMessage::JumpToNext));
    assert_eq!(app.test_playhead(), 5_000, "next marker after 2 000 is B");

    let _ = app.update(Message::Marker(MarkerMessage::JumpToPrev));
    assert_eq!(app.test_playhead(), 1_000, "prev marker before 5 000 is A");
}

#[test]
fn jump_to_specific_marker() {
    let mut app = marker_app();
    marker(&mut app, "A", 1_000);
    let c = marker(&mut app, "C", 9_000);

    let _ = app.update(Message::Marker(MarkerMessage::JumpTo(c)));
    assert_eq!(app.test_playhead(), 9_000);

    // Jumping to a missing marker leaves the playhead put.
    let _ = app.update(Message::Marker(MarkerMessage::JumpTo(424_242)));
    assert_eq!(app.test_playhead(), 9_000);
}

#[test]
fn play_from_marker_seeks_and_starts_playback() {
    let mut app = marker_app();
    let id = marker(&mut app, "A", 7_000);
    assert!(!app.test_transport_playing());

    let _ = app.update(Message::Marker(MarkerMessage::PlayFromMarker(id)));
    assert_eq!(app.test_playhead(), 7_000);
    assert!(
        app.test_transport_playing(),
        "PlayFromMarker should start playback"
    );
}

// ---------------- LoopToRegion ----------------

#[test]
fn loop_to_region_uses_a_ranged_markers_bounds() {
    let mut app = marker_app();
    let id = app.test_add_marker(ArrangementMarker::new_region(
        1,
        "Verse".into(),
        [1, 2, 3],
        4_000,
        12_000,
    ));

    let _ = app.update(Message::Marker(MarkerMessage::LoopToRegion(id)));
    assert_eq!(app.test_loop_state(), (4_000, 12_000, true));
}

#[test]
fn loop_to_region_point_loops_to_next_marker() {
    let mut app = marker_app();
    let a = marker(&mut app, "A", 2_000);
    marker(&mut app, "B", 8_000);

    let _ = app.update(Message::Marker(MarkerMessage::LoopToRegion(a)));
    assert_eq!(
        app.test_loop_state(),
        (2_000, 8_000, true),
        "a point marker loops from its start to the next marker's start"
    );
}

#[test]
fn loop_to_region_point_without_following_marker_is_noop() {
    let mut app = marker_app();
    let only = marker(&mut app, "Only", 2_000);

    let before = app.test_loop_state();
    let _ = app.update(Message::Marker(MarkerMessage::LoopToRegion(only)));
    assert_eq!(
        app.test_loop_state(),
        before,
        "a lone point marker has no region to loop, so transport is untouched"
    );
}

// ---------------- Undo classification ----------------

#[test]
fn mutating_marker_messages_record_undo() {
    for m in [
        MarkerMessage::AddAtPlayhead,
        MarkerMessage::Rename(1, "x".into()),
        MarkerMessage::Recolor(1, [0, 0, 0]),
        MarkerMessage::Delete(1),
        MarkerMessage::MoveStart(1, 0),
        MarkerMessage::SetRegionEnd(1, Some(10)),
        MarkerMessage::LoopToRegion(1),
    ] {
        assert!(
            matches!(classify(&Message::Marker(m.clone())), UndoAction::Record),
            "{m:?} should record an undo entry"
        );
    }
}

#[test]
fn navigation_marker_messages_skip_undo() {
    for m in [
        MarkerMessage::JumpToNext,
        MarkerMessage::JumpToPrev,
        MarkerMessage::JumpTo(1),
        MarkerMessage::PlayFromMarker(1),
    ] {
        assert!(
            matches!(classify(&Message::Marker(m.clone())), UndoAction::Skip),
            "{m:?} should not touch the undo history"
        );
    }
}
