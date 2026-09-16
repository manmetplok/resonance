//! Read-your-own-writes for `notes.*` (ba doc #265, Bug 2b, todo #1166).
//!
//! A `notes.insert`/`edit`/`delete` that has returned `{revision:N}` must
//! be visible to the very next `song.notes` on the same connection, with
//! NO engine echo driven in between. Previously the app-side note vector
//! was only updated when the engine echoed the edit back, so a follow-up
//! `song.notes` raced the round trip (report: 28 acked inserts, then
//! `song.notes` returned 3, converging to 28 only after ~80 ms — and a
//! reconciling client re-inserted the "missing" 25 as duplicates).
//!
//! The control handler now mirrors each edit into `app.midi_clips`
//! synchronously and suppresses the matching echo, so the tests below
//! never call `test_apply_engine_event`. A separate test drives the late
//! echo afterwards to prove it is a no-op (no duplication / double
//! removal).

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, MidiNote, TrackType};
use resonance_control::methods::notes::{CreateClipResult, InsertResult};
use resonance_control::methods::song::NotesView;
use resonance_control::Request;
use crate::common::{call, roundtrip};

const SR: u32 = 48_000;
const TPQ: u64 = 480;
const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/control-notes-read-your-writes.rprj",
    ));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

fn notes_of(app: &mut Resonance, clip_id: u64) -> NotesView {
    roundtrip(
        app,
        Request::new(9, "song.notes", &serde_json::json!({ "clip_id": clip_id })).unwrap(),
    )
    .result()
    .expect("song.notes succeeds")
}

fn create_clip(app: &mut Resonance) -> u64 {
    let response = call(
        app,
        "notes.create_clip",
        serde_json::json!({ "track_id": TRACK, "start_bar": 1, "length_beats": 16.0 }),
    );
    let result: CreateClipResult = response.result().expect("create_clip succeeds");
    u64::from(result.clip_id)
}

fn insert(app: &mut Resonance, clip_id: u64, pitch: u8, start_beat: f64) -> InsertResult {
    call(
        app,
        "notes.insert",
        serde_json::json!({
            "clip_id": clip_id,
            "pitch": pitch,
            "start_beat": start_beat,
            "duration_beats": 0.5,
            "velocity": 100
        }),
    )
    .result()
    .expect("insert succeeds")
}

#[test]
fn song_notes_reflects_all_acked_inserts_on_the_very_next_request() {
    // The reported failure: N inserts all ack, then song.notes lags. Here
    // it must return exactly N with no echo driven in between.
    let mut app = app();
    let clip_id = create_clip(&mut app);

    const N: u8 = 28;
    for i in 0..N {
        // Distinct start ticks so ordering is unambiguous.
        insert(&mut app, clip_id, 60 + (i % 12), i as f64 * 0.25);
    }

    let view = notes_of(&mut app, clip_id);
    assert_eq!(
        view.notes.len(),
        N as usize,
        "every acked insert must be visible to the next song.notes (Bug 2b)"
    );
}

#[test]
fn reported_insert_index_matches_song_notes_position() {
    // A client that trusts the returned index must find the note there on
    // the next read — the optimistic mirror and the reported index agree.
    let mut app = app();
    let clip_id = create_clip(&mut app);

    insert(&mut app, clip_id, 60, 0.0);
    insert(&mut app, clip_id, 67, 2.0);
    // Insert between the two: the reply's index must be 1, and song.notes
    // must show pitch 64 at index 1 immediately.
    let mid = insert(&mut app, clip_id, 64, 1.0);
    assert_eq!(mid.index, 1);

    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 3);
    assert_eq!(view.notes[1].pitch, 64);
}

#[test]
fn delete_is_visible_immediately_and_the_echo_does_not_double_remove() {
    let mut app = app();
    let clip_id = create_clip(&mut app);
    let pitches = [60u8, 62, 64];
    for (i, &p) in pitches.iter().enumerate() {
        insert(&mut app, clip_id, p, i as f64);
    }
    // Drain the three insert echoes in FIFO order (the engine channel
    // delivers them before the later delete echo).
    for (i, &p) in pitches.iter().enumerate() {
        app.test_apply_engine_event(AudioEvent::MidiNoteAdded {
            clip_id,
            note: MidiNote {
                note: p,
                velocity: 100.0 / 127.0,
                start_tick: i as u64 * TPQ,
                duration_ticks: TPQ / 2,
            },
        });
    }
    assert_eq!(notes_of(&mut app, clip_id).notes.len(), 3);

    // Delete the middle note; the next read must show 2 notes (60, 64).
    let resp = call(
        &mut app,
        "notes.delete",
        serde_json::json!({ "clip_id": clip_id, "index": 1 }),
    );
    assert!(resp.error.is_none(), "delete succeeds: {:?}", resp.error);
    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 2);
    assert_eq!(view.notes[0].pitch, 60);
    assert_eq!(view.notes[1].pitch, 64);

    // The late MidiNoteRemoved echo for that delete must be a no-op — it
    // must NOT remove a second (now index-shifted) note.
    app.test_apply_engine_event(AudioEvent::MidiNoteRemoved {
        clip_id,
        note_index: 1,
    });
    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 2, "suppressed echo must not double-remove");
    assert_eq!(view.notes[0].pitch, 60);
    assert_eq!(view.notes[1].pitch, 64);
}

#[test]
fn edit_is_visible_immediately_and_the_echo_does_not_reapply() {
    let mut app = app();
    let clip_id = create_clip(&mut app);
    insert(&mut app, clip_id, 60, 0.0);
    // Drain the insert echo (FIFO) before the later edit echoes.
    app.test_apply_engine_event(AudioEvent::MidiNoteAdded {
        clip_id,
        note: MidiNote { note: 60, velocity: 100.0 / 127.0, start_tick: 0, duration_ticks: TPQ / 2 },
    });

    // Change pitch + velocity in one edit; visible on the next read.
    let resp = call(
        &mut app,
        "notes.edit",
        serde_json::json!({ "clip_id": clip_id, "index": 0, "pitch": 72, "velocity": 40 }),
    );
    assert!(resp.error.is_none(), "edit succeeds: {:?}", resp.error);
    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].pitch, 72);
    assert_eq!(view.notes[0].velocity, 40);

    // The late move/velocity echoes re-apply the same value (idempotent),
    // and are suppressed anyway — state is unchanged and not duplicated.
    app.test_apply_engine_event(AudioEvent::MidiNoteMoved {
        clip_id,
        note_index: 0,
        new_start_tick: 0,
        new_note: 72,
    });
    app.test_apply_engine_event(AudioEvent::MidiNoteVelocitySet {
        clip_id,
        note_index: 0,
        velocity: 40.0 / 127.0,
    });
    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].pitch, 72);
}

#[test]
fn a_late_add_echo_after_many_inserts_does_not_duplicate() {
    // Insert several, read (all visible), THEN let the engine echoes land
    // in order — each must be consumed as a no-op, leaving the count.
    let mut app = app();
    let clip_id = create_clip(&mut app);
    let pitches = [60u8, 62, 64, 65];
    for (i, &p) in pitches.iter().enumerate() {
        insert(&mut app, clip_id, p, i as f64);
    }
    assert_eq!(notes_of(&mut app, clip_id).notes.len(), 4);

    for (i, &p) in pitches.iter().enumerate() {
        app.test_apply_engine_event(AudioEvent::MidiNoteAdded {
            clip_id,
            note: MidiNote {
                note: p,
                velocity: 100.0 / 127.0,
                start_tick: i as u64 * TPQ,
                duration_ticks: TPQ / 2,
            },
        });
    }
    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 4, "suppressed add echoes must not duplicate");
}
