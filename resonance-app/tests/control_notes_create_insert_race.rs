//! `notes.create_clip` -> immediate `notes.insert` must succeed
//! deterministically (ba doc #265, todo #1162).
//!
//! `notes.create_clip` allocates the clip id app-side and returns it in
//! the reply. Previously the clip only entered `app.midi_clips` when the
//! engine echoed `MidiClipCreated`, so a `notes.insert` arriving before
//! that async round trip failed `not_found` for the id create had just
//! handed out (in a burst, ~82 of the first inserts failed this way).
//!
//! The fix mirrors the empty clip into `app.midi_clips` synchronously at
//! create time, so the returned id is live on the very next request. The
//! `MidiClipCreated` echo stays idempotent, so a late echo is a no-op.
//! These tests drive create + insert with NO echo in between.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, MidiNote, TrackType};
use resonance_control::methods::notes::{CreateClipResult, InsertResult};
use resonance_control::methods::song::NotesView;
use resonance_control::Request;

const SR: u32 = 48_000;
const TPQ: u64 = 480;
const TRACK: u64 = 1;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/control-notes-create-insert-race.rprj",
    ));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

fn roundtrip(app: &mut Resonance, req: Request) -> resonance_control::Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> resonance_control::Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn create_clip(app: &mut Resonance) -> u64 {
    let response = call(
        app,
        "notes.create_clip",
        serde_json::json!({ "track_id": TRACK, "start_bar": 1, "length_beats": 4.0 }),
    );
    let result: CreateClipResult = response.result().expect("create_clip succeeds");
    u64::from(result.clip_id)
}

#[test]
fn insert_immediately_after_create_succeeds_without_the_engine_echo() {
    let mut app = app();
    let clip_id = create_clip(&mut app);

    // No `MidiClipCreated` echo yet — the very next request must already
    // see the clip. Before the fix this returned `not_found`.
    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({
            "clip_id": clip_id,
            "pitch": 60,
            "start_beat": 0.0,
            "duration_beats": 1.0,
            "velocity": 100
        }),
    );
    let result: InsertResult = response
        .result()
        .expect("insert into a just-created clip must succeed, not not_found");
    assert_eq!(result.index, 0, "first note lands at index 0");
}

#[test]
fn burst_of_creates_then_inserts_all_succeed_deterministically() {
    // Reproduce the reported failure shape: create many clips back to
    // back, THEN insert into each — all before any engine echo. Every
    // insert must succeed; the pre-fix bug failed the first ~82.
    let mut app = app();
    let clip_ids: Vec<u64> = (0..16).map(|_| create_clip(&mut app)).collect();

    for (i, &clip_id) in clip_ids.iter().enumerate() {
        let response = call(
            &mut app,
            "notes.insert",
            serde_json::json!({
                "clip_id": clip_id,
                "pitch": 60,
                "start_beat": 0.0,
                "duration_beats": 1.0,
                "velocity": 90
            }),
        );
        assert!(
            response.result::<InsertResult>().is_ok(),
            "insert into clip #{i} (id {clip_id}) must succeed with no echo: {:?}",
            response.error
        );
    }
    // The ids are all distinct (each create allocates a fresh id).
    let mut sorted = clip_ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), clip_ids.len(), "each create_clip yields a unique id");
}

#[test]
fn a_late_engine_echo_does_not_duplicate_the_clip() {
    // The synchronous mirror plus the idempotent `MidiClipCreated`
    // handler must not produce two entries for one id when the echo
    // finally lands — and any notes inserted before the echo survive it.
    let mut app = app();
    let clip_id = create_clip(&mut app);

    let _ = call(
        &mut app,
        "notes.insert",
        serde_json::json!({ "clip_id": clip_id, "pitch": 64, "start_beat": 0.0, "duration_beats": 1.0 }),
    );
    app.test_apply_engine_event(AudioEvent::MidiNoteAdded {
        clip_id,
        note: MidiNote { note: 64, velocity: 100.0 / 127.0, start_tick: 0, duration_ticks: TPQ },
    });

    // The delayed empty-clip echo arrives now.
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 4 * TPQ,
        name: "MIDI Clip".to_owned(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });

    let view: NotesView = roundtrip(
        &mut app,
        Request::new(9, "song.notes", &serde_json::json!({ "clip_id": clip_id })).unwrap(),
    )
    .result()
    .expect("song.notes succeeds");
    // Exactly one clip, and its earlier-inserted note was not clobbered
    // by the idempotent echo (which carries an empty note list).
    assert_eq!(view.notes.len(), 1, "note inserted before the echo survives");
    assert_eq!(view.notes[0].pitch, 64);
}
