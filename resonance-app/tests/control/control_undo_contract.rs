//! One mutating control call = one revision bump = one undo entry (code
//! review CTL-03).
//!
//! The wire contract tells agents to detect concurrent user edits from
//! revision deltas and to back out one decision with one `edit.undo`.
//! Three things broke it: `section.create` with a scale dispatched two
//! recorded messages, `track.delete` of a track with clips recorded the
//! confirm dialog as its own entry, and two separate fader calls on the
//! same track coalesced into one entry like a GUI drag burst. Every
//! mutating call now runs in its own compound group.

use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::methods::edit::UndoResult;
use resonance_control::methods::song::TracksView;
use resonance_control::{MutationAck, Request};
use crate::common::{call, roundtrip};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-undo-contract.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

fn volume_db(app: &mut Resonance) -> f32 {
    let view: TracksView = roundtrip(app, Request::without_params(96, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    view.tracks
        .iter()
        .find(|t| t.summary.id.0 == TRACK)
        .expect("track")
        .summary
        .volume_db
}

fn set_volume(app: &mut Resonance, db: f32) {
    let _: MutationAck = call(
        app,
        "mixer.set_volume_db",
        serde_json::json!({"track_id": TRACK, "volume_db": db}),
    )
    .result()
    .expect("mixer.set_volume_db succeeds");
}

#[test]
fn section_create_with_a_scale_is_one_revision_and_one_entry() {
    let mut app = app();
    let revision = app.revision();
    let before = entries(&app);

    let response = call(
        &mut app,
        "section.create",
        serde_json::json!({
            "name": "Chorus",
            "length_bars": 4,
            "scale": {"tonic": "A", "scale": "minor"},
            "place": true,
        }),
    );
    assert!(response.result::<serde_json::Value>().is_ok(), "section.create succeeds");

    assert_eq!(app.revision(), revision + 1, "one call, one revision bump");
    assert_eq!(entries(&app), before + 1, "one call, one undo entry");
}

#[test]
fn track_delete_of_a_track_with_a_clip_is_one_revision_and_one_entry() {
    let mut app = app();
    app.test_push_midi_clip(MidiClipState {
        id: 5,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 3840,
        name: "clip".into(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let revision = app.revision();
    let before = entries(&app);

    let _: MutationAck = call(
        &mut app,
        "track.delete",
        serde_json::json!({"track_id": TRACK, "confirm": true}),
    )
    .result()
    .expect("track.delete succeeds");

    assert_eq!(app.revision(), revision + 1, "one call, one revision bump");
    assert_eq!(entries(&app), before + 1, "one call, one undo entry");
}

#[test]
fn two_fader_calls_are_two_entries_and_one_undo_takes_back_one() {
    let mut app = app();
    set_volume(&mut app, -6.0);
    set_volume(&mut app, -4.0);
    assert_eq!(entries(&app), 2, "separate calls never coalesce");

    let _: UndoResult = roundtrip(&mut app, Request::without_params(98, "edit.undo"))
        .result()
        .expect("edit.undo succeeds");
    assert!(
        (volume_db(&mut app) - -6.0).abs() < 1e-4,
        "one undo restores the first call's value"
    );
}
