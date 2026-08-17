//! `notes.insert_many` / `notes.replace_all` (ba doc #269 FR-5) through
//! the real update path.
//!
//! The point of both methods is that the whole batch is ONE undoable
//! transaction: every control mutation is its own `record_undo`, so a
//! 400-note melody written with `notes.insert` left 400 undo entries and
//! 400 read-after-write round trips. These tests pin down the single
//! undo step, the reported indices, and read-your-own-writes.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{MidiNote, TrackType};
use resonance_control::methods::notes::InsertManyResult;
use resonance_control::methods::song::NotesView;
use resonance_control::{ErrorKind, Request, Response};

const SR: u32 = 48_000;
const TPQ: u64 = 480;
const TRACK: u64 = 1;
const CLIP: u64 = 100;

fn app_with_clip(notes: Vec<MidiNote>) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-notes-bulk.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_midi_clip(MidiClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 64 * TPQ,
        name: "clip".to_owned(),
        notes,
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

fn note(pitch: u8, start_beats: u64, dur_beats: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick: start_beats * TPQ,
        duration_ticks: dur_beats * TPQ,
    }
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn notes_of(app: &mut Resonance) -> NotesView {
    roundtrip(
        app,
        Request::new(9, "song.notes", &serde_json::json!({ "clip_id": CLIP })).unwrap(),
    )
    .result()
    .expect("song.notes succeeds")
}

fn spec(pitch: u8, start_beat: f64, duration_beats: f64) -> serde_json::Value {
    serde_json::json!({
        "pitch": pitch,
        "start_beat": start_beat,
        "duration_beats": duration_beats,
    })
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

// ---------------- notes.insert_many ----------------

#[test]
fn insert_many_is_one_undo_entry_and_readable_immediately() {
    let mut app = app_with_clip(Vec::new());
    let before = app.revision();

    let batch: Vec<serde_json::Value> = (0..64)
        .map(|i| spec(60 + (i % 12) as u8, i as f64, 1.0))
        .collect();
    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({ "clip_id": CLIP, "notes": batch }),
    );
    let result: InsertManyResult = response.result().expect("insert_many succeeds");
    assert_eq!(result.indices.len(), 64);

    // Read-your-own-writes: the very next song.notes sees all 64.
    assert_eq!(notes_of(&mut app).notes.len(), 64);

    // One transaction for the whole batch — that is the point of the
    // method. 64 notes.insert calls would be 64 revisions and 64 undos.
    assert_eq!(app.revision(), before + 1);
    let _ = app.update(Message::Undo);
    assert_eq!(
        notes_of(&mut app).notes.len(),
        0,
        "a single undo removes the whole batch"
    );
}

#[test]
fn insert_many_keeps_existing_notes_and_reports_sorted_indices() {
    // Existing notes at beats 0 and 4; the batch interleaves around them.
    let mut app = app_with_clip(vec![note(60, 0, 1), note(67, 4, 1)]);

    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({
            "clip_id": CLIP,
            // Submitted out of order on purpose.
            "notes": [spec(64, 2.0, 1.0), spec(62, 1.0, 1.0), spec(69, 6.0, 1.0)],
        }),
    );
    let result: InsertManyResult = response.result().expect("insert_many succeeds");

    let view = notes_of(&mut app);
    assert_eq!(view.notes.len(), 5, "the existing notes survive");
    let pitches: Vec<u8> = view.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 62, 64, 67, 69]);

    // indices[i] addresses the i-th SUBMITTED note, in submission order.
    assert_eq!(result.indices, vec![2, 1, 4]);
    for (i, expected_pitch) in [(0usize, 64u8), (1, 62), (2, 69)] {
        assert_eq!(
            view.notes[result.indices[i]].pitch,
            expected_pitch,
            "indices[{i}] addresses the submitted note"
        );
    }
}

#[test]
fn notes_at_the_same_beat_keep_submission_order_after_existing_ones() {
    let mut app = app_with_clip(vec![note(60, 1, 1)]);
    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({
            "clip_id": CLIP,
            "notes": [spec(64, 1.0, 1.0), spec(67, 1.0, 1.0)],
        }),
    );
    let result: InsertManyResult = response.result().expect("insert_many succeeds");
    assert_eq!(result.indices, vec![1, 2], "the kept note stays first");

    let view = notes_of(&mut app);
    let pitches: Vec<u8> = view.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 64, 67]);
}

#[test]
fn an_empty_batch_is_accepted_and_changes_nothing() {
    let mut app = app_with_clip(vec![note(60, 0, 1)]);
    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({ "clip_id": CLIP, "notes": [] }),
    );
    let result: InsertManyResult = response.result().expect("an empty batch is not an error");
    assert!(result.indices.is_empty());
    assert_eq!(notes_of(&mut app).notes.len(), 1);
}

// ---------------- notes.replace_all ----------------

#[test]
fn replace_all_drops_the_previous_notes_in_one_step() {
    let mut app = app_with_clip(vec![note(60, 0, 1), note(62, 1, 1), note(64, 2, 1)]);
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.replace_all",
        serde_json::json!({
            "clip_id": CLIP,
            "notes": [spec(72, 0.0, 2.0), spec(74, 2.0, 2.0)],
        }),
    );
    let result: InsertManyResult = response.result().expect("replace_all succeeds");
    assert_eq!(result.indices, vec![0, 1]);

    let view = notes_of(&mut app);
    let pitches: Vec<u8> = view.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![72, 74], "nothing of the old part survives");

    assert_eq!(app.revision(), before + 1);
    let _ = app.update(Message::Undo);
    let pitches: Vec<u8> = notes_of(&mut app).notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 62, 64], "one undo restores the old part");
}

#[test]
fn replace_all_with_an_empty_list_clears_the_clip() {
    let mut app = app_with_clip(vec![note(60, 0, 1), note(62, 1, 1)]);
    let response = call(
        &mut app,
        "notes.replace_all",
        serde_json::json!({ "clip_id": CLIP, "notes": [] }),
    );
    let _: InsertManyResult = response.result().expect("replace_all succeeds");
    assert!(notes_of(&mut app).notes.is_empty());
}

// ---------------- validation ----------------

#[test]
fn a_bad_note_names_its_position_and_mutates_nothing() {
    let mut app = app_with_clip(vec![note(60, 0, 1)]);

    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({
            "clip_id": CLIP,
            "notes": [spec(64, 0.0, 1.0), spec(200, 1.0, 1.0)],
        }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("notes[1]"), "{message}");
    assert!(message.contains("pitch"), "{message}");
    assert_eq!(
        notes_of(&mut app).notes.len(),
        1,
        "the whole batch is rejected, not partially applied"
    );

    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({
            "clip_id": CLIP,
            "notes": [spec(64, 0.0, 0.0)],
        }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("notes[0]"), "{message}");
    assert!(message.contains("duration"), "{message}");
}

#[test]
fn an_unknown_clip_is_not_found() {
    let mut app = app_with_clip(Vec::new());
    for method in ["notes.insert_many", "notes.replace_all"] {
        let response = call(
            &mut app,
            method,
            serde_json::json!({ "clip_id": 9999, "notes": [spec(60, 0.0, 1.0)] }),
        );
        let message = expect_error(response, ErrorKind::NotFound);
        assert!(message.contains("MIDI clip"), "{method}: {message}");
    }
}
