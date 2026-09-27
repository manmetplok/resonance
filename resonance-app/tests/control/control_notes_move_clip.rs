//! `notes.move_clip` (ba doc #269 FR-4) through the real update path:
//! repositioning a MIDI clip, which is the only way a remote client can
//! correct a clip's start (or re-grid a drifted one) short of deleting
//! and rebuilding it.

use resonance_app::message::Message;
use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{MidiNote, TrackType};
use resonance_control::{ErrorKind, MutationAck, Request, Response};
use crate::common::{call, roundtrip};

const SR: u32 = 48_000;
const TPQ: u64 = 480;
const TRACK: u64 = 1;
const CLIP: u64 = 100;

fn app_with_clip(start_sample: u64) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-move-clip.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_midi_clip(MidiClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample,
        duration_ticks: 4 * TPQ,
        name: "clip".to_owned(),
        notes: vec![MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: TPQ,
        }]
        .into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

/// The sample position of a 0-based bar, through the same tempo map the
/// handler snaps to.
fn bar_to_sample(app: &Resonance, bar: u32) -> u64 {
    app.test_tempo_map().bar_to_sample(bar)
}

fn clip_start(app: &Resonance) -> u64 {
    app.test_midi_clips()
        .iter()
        .find(|c| c.id == CLIP)
        .expect("clip exists")
        .start_sample
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

#[test]
fn moves_a_clip_to_a_start_bar() {
    let mut app = app_with_clip(0);
    let target = bar_to_sample(&app, 4);

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "start_bar": 5 }),
    );
    let _: MutationAck = response.result().expect("move_clip succeeds");
    // start_bar is 1-based on the wire; bar 5 is the 0-based bar 4.
    assert_eq!(clip_start(&app), target);
}

#[test]
fn a_drifted_start_is_re_gridded() {
    // The generated-clip drift this method exists to correct: a start a
    // few samples early snaps back onto the bar line.
    let bar_2 = {
        let app = app_with_clip(0);
        bar_to_sample(&app, 1)
    };
    let mut app = app_with_clip(bar_2 - 75);
    assert_ne!(clip_start(&app), bar_2);

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "start_bar": 2 }),
    );
    let _: MutationAck = response.result().expect("move_clip succeeds");
    assert_eq!(clip_start(&app), bar_2);
}

#[test]
fn the_move_is_one_undoable_step() {
    let mut app = app_with_clip(0);
    let before = app.revision();

    let _ = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "start_bar": 3 }),
    );
    assert!(app.revision() > before, "the move bumps the revision");

    let _ = app.update(Message::Undo);
    assert_eq!(clip_start(&app), 0, "undo restores the original start");
}

#[test]
fn a_placement_anchors_the_clip_to_its_section() {
    let mut app = app_with_clip(0);
    // Section definition placed at bar 9 (0-based bar 8).
    let section = roundtrip(
        &mut app,
        Request::new(
            2,
            "section.create",
            &serde_json::json!({ "name": "Chorus", "length_bars": 4 }),
        )
        .unwrap(),
    )
    .result::<resonance_control::methods::section::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let placement = roundtrip(
        &mut app,
        Request::new(
            3,
            "section.place",
            &serde_json::json!({ "definition_id": section, "start_bar": 9 }),
        )
        .unwrap(),
    )
    .result::<resonance_control::methods::section::PlaceResult>()
    .expect("section.place succeeds")
    .placement_id;

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "placement_id": placement }),
    );
    let _: MutationAck = response.result().expect("move_clip succeeds");
    assert_eq!(clip_start(&app), bar_to_sample(&app, 8));
}

#[test]
fn exactly_one_target_is_required() {
    let mut app = app_with_clip(0);

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("start_bar"), "{message}");

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "start_bar": 2, "placement_id": 1 }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("not both"), "{message}");

    assert_eq!(clip_start(&app), 0, "a rejected move changes nothing");
}

#[test]
fn bar_zero_is_rejected() {
    let mut app = app_with_clip(0);
    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "start_bar": 0 }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("1-based"), "{message}");
}

#[test]
fn unknown_clip_and_placement_are_not_found() {
    let mut app = app_with_clip(0);

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": 9999, "start_bar": 2 }),
    );
    let message = expect_error(response, ErrorKind::NotFound);
    assert!(message.contains("MIDI clip"), "{message}");

    let response = call(
        &mut app,
        "notes.move_clip",
        serde_json::json!({ "clip_id": CLIP, "placement_id": 9999 }),
    );
    let message = expect_error(response, ErrorKind::NotFound);
    assert!(message.contains("placement"), "{message}");
}
