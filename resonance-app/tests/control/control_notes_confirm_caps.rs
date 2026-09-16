//! `notes.replace_all` confirm gating and the bulk-write bounds.
//!
//! `notes.replace_all` is destructive — every existing note in the clip
//! is dropped — so it follows the same confirm convention as
//! `track.delete` / `section.delete`: without `"confirm": true` it
//! refuses with a summary of what would be lost and mutates NOTHING.
//! The bulk writes are also bounded the way `notes.import_midi` always
//! was: at most [`MAX_BATCH_NOTES`] notes per batch, and every beat
//! value capped at [`MAX_BEATS`] — an unbounded `start_beat` like `1e15`
//! used to dispatch ~1e18 ticks whose later `start_tick +
//! duration_ticks` overflows u64 (debug panic; release wrap = corrupted
//! note in the saved `.mid`).

use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{MidiNote, TrackType};
use resonance_control::methods::notes::{InsertManyResult, MAX_BATCH_NOTES, MAX_BEATS};
use resonance_control::methods::song::NotesView;
use resonance_control::{ErrorKind, Request, Response};
use crate::common::{call, roundtrip};

const SR: u32 = 48_000;
const TPQ: u64 = 480;
const TRACK: u64 = 1;
const CLIP: u64 = 100;

fn app_with_clip(notes: Vec<MidiNote>) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-notes-confirm.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_midi_clip(MidiClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 64 * TPQ,
        name: "part".to_owned(),
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

// ---------------------------------------------------------------------------
// notes.replace_all confirm gating
// ---------------------------------------------------------------------------

#[test]
fn replace_all_refuses_without_confirm_and_mutates_nothing() {
    let mut app = app_with_clip(vec![note(60, 0, 1), note(62, 1, 1), note(64, 2, 1)]);
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.replace_all",
        serde_json::json!({ "clip_id": CLIP, "notes": [spec(72, 0.0, 1.0)] }),
    );
    let message = expect_error(response, ErrorKind::NeedsConfirmation);
    // The summary is concrete: the note count being dropped and how to
    // proceed, so the client can decide without a follow-up read.
    assert!(message.contains("3 existing note(s)"), "{message}");
    assert!(message.contains("\"confirm\": true"), "{message}");

    let pitches: Vec<u8> = notes_of(&mut app).notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 62, 64], "a refusal mutates nothing");
    assert_eq!(app.revision(), before, "no revision bump for a refusal");
}

#[test]
fn replace_all_with_confirm_replaces_the_notes() {
    let mut app = app_with_clip(vec![note(60, 0, 1), note(62, 1, 1)]);

    let response = call(
        &mut app,
        "notes.replace_all",
        serde_json::json!({
            "clip_id": CLIP,
            "notes": [spec(72, 0.0, 2.0)],
            "confirm": true,
        }),
    );
    let result: InsertManyResult = response.result().expect("confirmed replace_all succeeds");
    assert_eq!(result.indices, vec![0]);

    let pitches: Vec<u8> = notes_of(&mut app).notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![72], "nothing of the old part survives");
}

#[test]
fn replace_all_on_an_empty_clip_needs_no_confirm() {
    // An empty clip loses nothing, so requiring a confirm round-trip
    // there would only tax the common authoring flow (the
    // `arrangement.remove_bars` rule: gate on casualties).
    let mut app = app_with_clip(Vec::new());

    let response = call(
        &mut app,
        "notes.replace_all",
        serde_json::json!({ "clip_id": CLIP, "notes": [spec(60, 0.0, 1.0)] }),
    );
    let _: InsertManyResult = response.result().expect("empty clip needs no confirm");
    assert_eq!(notes_of(&mut app).notes.len(), 1);
}

// ---------------------------------------------------------------------------
// The batch cap
// ---------------------------------------------------------------------------

/// One oversized batch, built once — the accepted case slices it down to
/// exactly the cap.
fn huge_batch() -> Vec<serde_json::Value> {
    // All at beat 0 with the minimum duration: the cap is about count,
    // and identical cheap notes keep the fixture fast.
    (0..MAX_BATCH_NOTES + 1)
        .map(|_| spec(60, 0.0, 0.25))
        .collect()
}

#[test]
fn a_batch_over_the_note_cap_is_rejected_up_front() {
    let mut app = app_with_clip(vec![note(60, 0, 1)]);
    let before = app.revision();
    let batch = huge_batch();

    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({ "clip_id": CLIP, "notes": batch }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(
        message.contains(&format!("{} notes", MAX_BATCH_NOTES + 1)),
        "{message}"
    );
    assert!(message.contains(&format!("{MAX_BATCH_NOTES} limit")), "{message}");

    // replace_all shares the bound (and its confirm does not bypass it).
    let batch = huge_batch();
    let response = call(
        &mut app,
        "notes.replace_all",
        serde_json::json!({ "clip_id": CLIP, "notes": batch, "confirm": true }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("limit"));

    assert_eq!(notes_of(&mut app).notes.len(), 1, "nothing was written");
    assert_eq!(app.revision(), before, "no revision bump for a rejection");
}

#[test]
fn a_batch_at_the_note_cap_is_accepted() {
    let mut app = app_with_clip(Vec::new());
    let mut batch = huge_batch();
    batch.truncate(MAX_BATCH_NOTES);

    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({ "clip_id": CLIP, "notes": batch }),
    );
    let result: InsertManyResult = response.result().expect("a batch at the cap succeeds");
    assert_eq!(result.indices.len(), MAX_BATCH_NOTES);
    assert_eq!(notes_of(&mut app).notes.len(), MAX_BATCH_NOTES);
}

// ---------------------------------------------------------------------------
// The beat bound
// ---------------------------------------------------------------------------

#[test]
fn an_absurd_beat_value_is_rejected_with_no_dispatch() {
    let mut app = app_with_clip(vec![note(60, 0, 1)]);
    let before = app.revision();

    // start_beat 1e15 used to convert to ~4.8e17 ticks and get
    // dispatched; the later tick addition overflowed u64.
    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({
            "clip_id": CLIP,
            "pitch": 60,
            "start_beat": 1e15,
            "duration_beats": 1.0,
        }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(
        message.contains(&format!("{} limit", MAX_BEATS as u64)),
        "the error states the bound: {message}"
    );

    // The duration arm shares the bound.
    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({
            "clip_id": CLIP,
            "pitch": 60,
            "start_beat": 0.0,
            "duration_beats": 1e15,
        }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("limit"));

    // And the bulk path points at the offending entry.
    let response = call(
        &mut app,
        "notes.insert_many",
        serde_json::json!({
            "clip_id": CLIP,
            "notes": [spec(60, 0.0, 1.0), spec(62, 1e15, 1.0)],
        }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("notes[1]"), "{message}");
    assert!(message.contains("limit"), "{message}");

    assert_eq!(notes_of(&mut app).notes.len(), 1, "nothing was written");
    assert_eq!(app.revision(), before, "no revision bump for a rejection");
}

#[test]
fn the_beat_bound_itself_is_accepted() {
    let mut app = app_with_clip(Vec::new());
    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({
            "clip_id": CLIP,
            "pitch": 60,
            "start_beat": MAX_BEATS,
            "duration_beats": 1.0,
        }),
    );
    assert!(
        response.error.is_none(),
        "the bound is inclusive: {:?}",
        response.error
    );
    assert_eq!(notes_of(&mut app).notes.len(), 1);
}
