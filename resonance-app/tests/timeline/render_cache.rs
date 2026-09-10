//! Cache-fingerprint coverage for the arrange timeline (view-performance
//! batch).
//!
//! The canvas's cached geometry pass repaints only when
//! `TimelineCanvas::fingerprint()` changes, and a MIDI clip drag mutates
//! `start_sample` / `track_id` in place with no drag ghost in the
//! uncached overlay — so the fingerprint is the *only* thing standing
//! between an in-place MIDI edit and a stale arrange render. These tests
//! pin that every MIDI-clip facet the canvas draws flips the
//! fingerprint, and that playhead-only motion (drawn in the uncached
//! overlay pass) does not.

use resonance_app::message::{Message, MidiClipMessage, TransportMessage};
use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{MidiNote, TrackType};

const TRACK: u64 = 1;
const CLIP: u64 = 77;

fn note(pitch: u8, start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick,
        duration_ticks,
    }
}

fn clip() -> MidiClipState {
    MidiClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 48_000,
        duration_ticks: 3840,
        name: "riff".into(),
        notes: vec![note(60, 0, 480), note(64, 480, 480)],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn app_with(clip: MidiClipState) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    // Without an active project the update gate drops every edit message.
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_midi_clip(clip);
    app
}

/// The regression itself: dragging a MIDI clip mutates it in place, and
/// the fingerprint must move with it or the cached render goes stale.
#[test]
fn moving_a_midi_clip_in_place_changes_the_fingerprint() {
    let mut app = app_with(clip());
    // Selection changes on Start are themselves fingerprint fields, so
    // baseline *after* the drag has started: the Update below changes
    // nothing but the clip's in-place geometry.
    let _ = app.update(Message::MidiClip(MidiClipMessage::StartMidiClipDrag {
        clip_id: CLIP,
        grab_offset_x: 0.0,
        start_x: 100.0,
        start_y: 0.0,
    }));
    let before = app.test_timeline_fingerprint();
    let _ = app.update(Message::MidiClip(MidiClipMessage::UpdateMidiClipDrag(
        800.0, 0.0,
    )));
    let moved = app
        .test_midi_clips()
        .iter()
        .find(|c| c.id == CLIP)
        .expect("clip still present")
        .start_sample;
    assert_ne!(moved, 48_000, "the drag actually moved the clip");
    assert_ne!(
        before,
        app.test_timeline_fingerprint(),
        "an in-place MIDI clip move must invalidate the cached render"
    );
}

/// Playhead motion is drawn in the uncached overlay pass and must NOT
/// invalidate the cached geometry.
#[test]
fn playhead_only_motion_keeps_the_fingerprint_equal() {
    let mut app = app_with(clip());
    let before = app.test_timeline_fingerprint();
    let _ = app.update(Message::Transport(TransportMessage::SkipForward));
    assert_ne!(app.test_playhead(), 0, "the skip moved the playhead");
    assert_eq!(
        before,
        app.test_timeline_fingerprint(),
        "playhead-only motion must keep hitting the cached geometry"
    );
}

/// Two identical states fingerprint identically (the hashes are
/// deterministic), so the inequality assertions below mean something.
#[test]
fn identical_states_share_a_fingerprint() {
    assert_eq!(
        app_with(clip()).test_timeline_fingerprint(),
        app_with(clip()).test_timeline_fingerprint(),
    );
}

/// Table-driven facet coverage: every MIDI-clip field the canvas draws
/// must flip the fingerprint when it differs — including each note's
/// pitch and horizontal extent, which the clip-body minimap renders.
#[test]
fn every_drawn_midi_clip_facet_changes_the_fingerprint() {
    let base = app_with(clip()).test_timeline_fingerprint();
    let facets: Vec<(&str, MidiClipState)> = vec![
        ("start_sample", MidiClipState {
            start_sample: 96_000,
            ..clip()
        }),
        ("track_id", MidiClipState {
            track_id: TRACK + 1,
            ..clip()
        }),
        ("duration_ticks", MidiClipState {
            duration_ticks: 7680,
            ..clip()
        }),
        ("trim_start_ticks", MidiClipState {
            trim_start_ticks: 240,
            ..clip()
        }),
        ("trim_end_ticks", MidiClipState {
            trim_end_ticks: 240,
            ..clip()
        }),
        ("name", MidiClipState {
            name: "renamed".into(),
            ..clip()
        }),
        ("note pitch", MidiClipState {
            notes: vec![note(61, 0, 480), note(64, 480, 480)],
            ..clip()
        }),
        ("note start", MidiClipState {
            notes: vec![note(60, 120, 480), note(64, 480, 480)],
            ..clip()
        }),
        ("note duration", MidiClipState {
            notes: vec![note(60, 0, 960), note(64, 480, 480)],
            ..clip()
        }),
        ("note added", MidiClipState {
            notes: vec![note(60, 0, 480), note(64, 480, 480), note(67, 960, 480)],
            ..clip()
        }),
        ("note removed", MidiClipState {
            notes: vec![note(60, 0, 480)],
            ..clip()
        }),
    ];
    for (facet, mutated) in facets {
        assert_ne!(
            base,
            app_with(mutated).test_timeline_fingerprint(),
            "a changed MIDI clip {facet} must invalidate the cached render"
        );
    }
}
