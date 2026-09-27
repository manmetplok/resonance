//! `notes.*` control methods through the real update path (ba doc #265,
//! todo #1155): piano-roll note insert/edit/delete + create_clip. The
//! app-side note vector is updated from engine events, so tests drive
//! the matching echo after each mutating call — exactly as the engine
//! would — then assert the mirrored state, undo behaviour, and errors.

use resonance_app::message::Message;
use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, MidiNote, TrackType};
use resonance_control::methods::notes::{CreateClipResult, InsertResult};
use resonance_control::methods::song::NotesView;
use resonance_control::{ErrorKind, MutationAck, Request};
use crate::common::{call, roundtrip};

const SR: u32 = 48_000;
const TPQ: u64 = 480;
const TRACK: u64 = 1;
const CLIP: u64 = 100;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-notes-test.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

/// A clip with the given notes already present.
fn app_with_clip(notes: Vec<MidiNote>) -> Resonance {
    let mut app = app();
    app.test_push_midi_clip(MidiClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 4 * TPQ,
        name: "clip".to_owned(),
        notes: notes.into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

fn note(pitch: u8, start_beats: u64, dur_beats: u64, vel: f32) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: vel,
        start_tick: start_beats * TPQ,
        duration_ticks: dur_beats * TPQ,
    }
}

fn notes_of(app: &mut Resonance, clip_id: u64) -> NotesView {
    roundtrip(
        app,
        Request::new(9, "song.notes", &serde_json::json!({ "clip_id": clip_id })).unwrap(),
    )
    .result()
    .expect("song.notes succeeds")
}

// ---------------- notes.insert ----------------

#[test]
fn insert_reports_sorted_index_and_mirrors_via_echo() {
    // Existing notes at beats 0 and 2; inserting at beat 1 lands at index 1.
    let mut app = app_with_clip(vec![note(60, 0, 1, 0.8), note(64, 2, 1, 0.8)]);
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({
            "clip_id": CLIP,
            "pitch": 62,
            "start_beat": 1.0,
            "duration_beats": 0.5,
            "velocity": 100
        }),
    );
    let result: InsertResult = response.result().expect("insert succeeds");
    assert_eq!(result.index, 1, "sorted insert index reported");
    assert_eq!(result.revision, before + 1);

    // Drive the engine echo that mirrors the note into app state.
    app.test_apply_engine_event(AudioEvent::MidiNoteAdded {
        clip_id: CLIP,
        note: MidiNote {
            note: 62,
            velocity: 100.0 / 127.0,
            start_tick: TPQ,
            duration_ticks: TPQ / 2,
        },
    });
    let view = notes_of(&mut app, CLIP);
    assert_eq!(view.notes.len(), 3);
    assert_eq!(view.notes[1].pitch, 62);
    assert_eq!(view.notes[1].start_beat, 1.0);
    assert_eq!(view.notes[1].duration_beats, 0.5);
    assert_eq!(view.notes[1].velocity, 100);
}

#[test]
fn insert_default_velocity_is_100() {
    let mut app = app_with_clip(vec![]);
    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({ "clip_id": CLIP, "pitch": 60, "start_beat": 0.0, "duration_beats": 1.0 }),
    );
    assert!(response.result::<InsertResult>().is_ok());
}

#[test]
fn insert_validates_pitch_velocity_duration_and_clip() {
    let mut app = app_with_clip(vec![]);
    for params in [
        serde_json::json!({ "clip_id": CLIP, "pitch": 200, "start_beat": 0.0, "duration_beats": 1.0 }),
        serde_json::json!({ "clip_id": CLIP, "pitch": 60, "start_beat": 0.0, "duration_beats": 0.0 }),
        serde_json::json!({ "clip_id": CLIP, "pitch": 60, "start_beat": -1.0, "duration_beats": 1.0 }),
        serde_json::json!({ "clip_id": CLIP, "pitch": 60, "start_beat": 0.0, "duration_beats": 1.0, "velocity": 200 }),
    ] {
        let response = call(&mut app, "notes.insert", params.clone());
        assert_eq!(
            response.error.unwrap_or_else(|| panic!("{params} rejected")).kind(),
            ErrorKind::InvalidParams,
            "{params}"
        );
    }
    let response = call(
        &mut app,
        "notes.insert",
        serde_json::json!({ "clip_id": 9999, "pitch": 60, "start_beat": 0.0, "duration_beats": 1.0 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

// ---------------- notes.edit ----------------

#[test]
fn edit_fans_out_pitch_start_duration_velocity() {
    let mut app = app_with_clip(vec![note(60, 0, 1, 0.5)]);
    let rx = app.test_capture_engine();
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.edit",
        serde_json::json!({
            "clip_id": CLIP,
            "index": 0,
            "pitch": 67,
            "start_beat": 1.0,
            "duration_beats": 2.0,
            "velocity": 120
        }),
    );
    let ack: MutationAck = response.result().expect("edit succeeds");
    // Move + resize + velocity fan out internally, but the call is ONE
    // undoable transaction — one revision bump on the wire.
    assert_eq!(ack.revision, before + 1);

    let cmds: Vec<_> = {
        let mut v = Vec::new();
        while let Ok(c) = rx.try_recv() {
            v.push(c);
        }
        v
    };
    use resonance_audio::types::AudioCommand as C;
    assert!(cmds.iter().any(|c| matches!(
        c,
        C::MoveMidiNote { new_note: 67, new_start_tick, .. } if *new_start_tick == TPQ
    )));
    assert!(cmds.iter().any(|c| matches!(
        c,
        C::ResizeMidiNote { new_duration_ticks, .. } if *new_duration_ticks == 2 * TPQ
    )));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, C::SetMidiNoteVelocity { .. })));
}

#[test]
fn edit_only_changes_requested_fields() {
    let mut app = app_with_clip(vec![note(60, 0, 1, 0.5)]);
    let before = app.revision();
    // Velocity-only edit: one sub-edit.
    let response = call(
        &mut app,
        "notes.edit",
        serde_json::json!({ "clip_id": CLIP, "index": 0, "velocity": 90 }),
    );
    assert!(response.result::<MutationAck>().is_ok());
    assert_eq!(app.revision(), before + 1);
}

/// Code review CTL-02: a move that reorders the clip re-sorts it, so the
/// resize and velocity sub-edits must not address the note by its old
/// index afterwards — they used to land on the neighbour that slid into
/// it. Checked against both the app mirror (`song.notes`) and the real
/// engine handlers replaying the captured commands.
#[test]
fn edit_that_reorders_the_note_changes_that_note_only() {
    use resonance_audio::test_support::EngineHandlerHarness;
    use resonance_audio::types::MidiClip;
    // C@0, D@1, E@2 (beats).
    let start = vec![note(60, 0, 1, 0.8), note(62, 1, 1, 0.8), note(64, 2, 1, 0.8)];
    let mut app = app_with_clip(start.clone());
    let rx = app.test_capture_engine();

    let response = call(
        &mut app,
        "notes.edit",
        serde_json::json!({
            "clip_id": CLIP,
            "index": 0,
            "start_beat": 3.0,
            "duration_beats": 0.25,
            "velocity": 30
        }),
    );
    assert!(response.result::<MutationAck>().is_ok());

    // (pitch, start_tick, duration_ticks, velocity 0..=127)
    let want = vec![
        (62, TPQ, TPQ, 102),
        (64, 2 * TPQ, TPQ, 102),
        (60, 3 * TPQ, TPQ / 4, 30),
    ];
    let view = notes_of(&mut app, CLIP);
    let mirrored: Vec<_> = view
        .notes
        .iter()
        .map(|n| (n.pitch, n.start_tick, n.duration_ticks, n.velocity))
        .collect();
    assert_eq!(mirrored, want, "app mirror");

    let mut engine = EngineHandlerHarness::new();
    engine.push_midi_clip(MidiClip {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 4 * TPQ,
        notes: start,
        name: "clip".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    while let Ok(cmd) = rx.try_recv() {
        engine.replay_midi_note_command(&cmd);
    }
    let played: Vec<_> = engine
        .midi_notes(CLIP)
        .iter()
        .map(|n| {
            (n.note, n.start_tick, n.duration_ticks, (n.velocity * 127.0).round() as u8)
        })
        .collect();
    assert_eq!(played, want, "engine");
}

/// The wire contract (doc #265): one mutating call = one revision bump
/// = one undoable transaction, however many sub-edits the call fans out
/// into. Before the compound-undo fix a three-field `notes.edit` bumped
/// the revision three times (a client falsely inferred a concurrent
/// user edit) and `edit.undo` reverted only a third of the call.
#[test]
fn edit_of_three_fields_is_one_atomic_undo_transaction() {
    let mut app = app_with_clip(vec![note(60, 0, 1, 0.5)]);
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.edit",
        serde_json::json!({
            "clip_id": CLIP,
            "index": 0,
            "pitch": 67,
            "duration_beats": 2.0,
            "velocity": 120
        }),
    );
    let ack: MutationAck = response.result().expect("edit succeeds");
    assert_eq!(ack.revision, before + 1, "one call, one revision bump");
    assert_eq!(
        ack.revision,
        app.revision(),
        "the ack carries the post-call revision"
    );

    // ONE undo restores ALL three fields to their pre-call values.
    let _ = app.update(Message::Undo);
    let view = notes_of(&mut app, CLIP);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].pitch, 60, "pitch restored");
    assert_eq!(view.notes[0].duration_ticks, TPQ, "duration restored");
    assert_eq!(view.notes[0].velocity, 64, "velocity restored (0.5 * 127)");

    // ONE redo replays the call to its final state.
    let _ = app.update(Message::Redo);
    let view = notes_of(&mut app, CLIP);
    assert_eq!(view.notes[0].pitch, 67);
    assert_eq!(view.notes[0].duration_ticks, 2 * TPQ);
    assert_eq!(view.notes[0].velocity, 120);
}

#[test]
fn edit_noop_and_missing_note_are_rejected() {
    let mut app = app_with_clip(vec![note(60, 0, 1, 0.5)]);
    // No fields -> invalid_params.
    let response = call(
        &mut app,
        "notes.edit",
        serde_json::json!({ "clip_id": CLIP, "index": 0 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::InvalidParams);
    // Out-of-range index -> not_found.
    let response = call(
        &mut app,
        "notes.edit",
        serde_json::json!({ "clip_id": CLIP, "index": 5, "pitch": 60 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

// ---------------- notes.delete ----------------

#[test]
fn delete_removes_the_note_and_is_undoable() {
    let mut app = app_with_clip(vec![note(60, 0, 1, 0.8), note(64, 1, 1, 0.8)]);
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.delete",
        serde_json::json!({ "clip_id": CLIP, "index": 0 }),
    );
    let ack: MutationAck = response.result().expect("delete succeeds");
    assert_eq!(ack.revision, before + 1);

    app.test_apply_engine_event(AudioEvent::MidiNoteRemoved {
        clip_id: CLIP,
        note_index: 0,
    });
    let view = notes_of(&mut app, CLIP);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].pitch, 64);

    // Out-of-range index -> not_found.
    let response = call(
        &mut app,
        "notes.delete",
        serde_json::json!({ "clip_id": CLIP, "index": 9 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

// ---------------- frozen-track rejection (DoD) ----------------

#[test]
fn edits_on_a_frozen_track_are_rejected_busy_with_no_change() {
    use resonance_app::state::FreezeStatus;
    use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

    let mut app = app_with_clip(vec![note(60, 0, 1, 0.8), note(64, 1, 1, 0.8)]);
    let cache = FreezeCacheRef::new(
        "freeze_1.wav".to_owned(),
        SR,
        32,
        1,
        FreezeCacheStatus::Frozen,
    );
    app.test_set_freeze_status(TRACK, FreezeStatus::Frozen { cache_ref: cache });

    let revision_before = app.revision();
    let notes_before = notes_of(&mut app, CLIP).notes.len();

    // insert / edit / delete must each be rejected with a stable `busy`
    // kind — not swallowed by the frozen-input gate and falsely acked —
    // and must not mutate the clip or bump the revision.
    let cases = [
        (
            "notes.insert",
            serde_json::json!({ "clip_id": CLIP, "pitch": 62, "start_beat": 0.5, "duration_beats": 1.0 }),
        ),
        (
            "notes.edit",
            serde_json::json!({ "clip_id": CLIP, "index": 0, "pitch": 67 }),
        ),
        (
            "notes.delete",
            serde_json::json!({ "clip_id": CLIP, "index": 0 }),
        ),
    ];
    for (method, params) in cases {
        let response = call(&mut app, method, params);
        let error = response
            .error
            .unwrap_or_else(|| panic!("{method} on a frozen track must be rejected"));
        assert_eq!(error.kind(), ErrorKind::Busy, "{method}");
        assert!(error.message.contains("frozen"), "{method}: {}", error.message);
    }

    // Nothing changed: same note count, same revision.
    assert_eq!(notes_of(&mut app, CLIP).notes.len(), notes_before);
    assert_eq!(app.revision(), revision_before);

    // Once unfrozen, the same edit goes through.
    app.test_set_freeze_status(TRACK, FreezeStatus::Idle);
    let response = call(
        &mut app,
        "notes.delete",
        serde_json::json!({ "clip_id": CLIP, "index": 0 }),
    );
    assert!(response.result::<MutationAck>().is_ok());
    assert_eq!(app.revision(), revision_before + 1);
}

// ---------------- notes.create_clip ----------------

#[test]
fn create_clip_at_bar_returns_id_and_mirrors() {
    let mut app = app();
    let before = app.revision();

    let response = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({ "track_id": TRACK, "start_bar": 2, "length_beats": 4.0, "name": "riff" }),
    );
    let result: CreateClipResult = response.result().expect("create_clip succeeds");
    let clip_id = u64::from(result.clip_id);
    assert_eq!(result.revision, before + 1);

    // Drive the engine echo that mirrors the empty clip.
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id,
        track_id: TRACK,
        start_sample: 2 * SR as u64,
        duration_ticks: 4 * TPQ,
        name: "riff".to_owned(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    // The new clip is now addressable by notes.* and starts empty.
    let view = notes_of(&mut app, clip_id);
    assert_eq!(view.notes.len(), 0);
    assert_eq!(view.clip_id.0, clip_id);
}

#[test]
fn create_clip_rejects_audio_track_and_unknown_track() {
    let mut app = app();
    app.test_add_track(2, TrackType::Audio);
    let response = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({ "track_id": 2, "start_bar": 1 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::InvalidParams);

    let response = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({ "track_id": 9999, "start_bar": 1 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

#[test]
fn create_clip_in_section_placement() {
    let mut app = app();
    // A 2-bar section placed at bar 0.
    let def = section_definition(1, 2);
    app.test_push_section_definition(def);
    let placement_id = app.test_place_section(1, 0);

    let response = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({ "track_id": TRACK, "placement_id": placement_id }),
    );
    let result: CreateClipResult = response.result().expect("create_clip succeeds");
    // Default length = section length (2 bars). Drive the echo and check.
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id: u64::from(result.clip_id),
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 2 * 4 * TPQ,
        name: "MIDI Clip".to_owned(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let view = notes_of(&mut app, u64::from(result.clip_id));
    assert_eq!(view.notes.len(), 0);
}

/// Default-clip-length regression (ba doc #265 fixed defect): the length
/// must come from the signature IN EFFECT at the clip's own target bar,
/// not the transport's current signature. A default-length clip dropped
/// into a 6/8 section of an otherwise-4/4 song must get a 6/8 bar's
/// worth of ticks (six eighth notes = 1440 ticks), not four quarter
/// notes (1920 ticks).
#[test]
fn create_clip_default_length_follows_the_signature_at_the_target_bar() {
    let mut app = app();
    // A meter change to 6/8 at wire bar 5 (0-based bar 4).
    let response = call(
        &mut app,
        "global.add_signature_event",
        serde_json::json!({ "bar": 5, "numerator": 6, "denominator": 8 }),
    );
    assert!(response.result::<MutationAck>().is_ok());

    let rx = app.test_capture_engine();
    let response = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({ "track_id": TRACK, "start_bar": 5 }),
    );
    assert!(response.result::<CreateClipResult>().is_ok());

    use resonance_audio::types::AudioCommand as C;
    let duration = rx
        .try_iter()
        .find_map(|c| match c {
            C::LoadMidiClipDirect { duration_ticks, .. } => Some(duration_ticks),
            _ => None,
        })
        .expect("create_clip dispatches LoadMidiClipDirect");
    // 6/8: six beats of an eighth note each (TPQ/2 ticks), not four
    // quarter-note beats.
    assert_eq!(duration, 6 * (TPQ / 2), "default length follows the 6/8 meter at bar 5");
}

#[test]
fn create_clip_default_length_in_a_4_4_region_is_unchanged() {
    let mut app = app();
    // Same song, but a bar still governed by the initial 4/4 meter.
    let rx = app.test_capture_engine();
    let response = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({ "track_id": TRACK, "start_bar": 1 }),
    );
    assert!(response.result::<CreateClipResult>().is_ok());

    use resonance_audio::types::AudioCommand as C;
    let duration = rx
        .try_iter()
        .find_map(|c| match c {
            C::LoadMidiClipDirect { duration_ticks, .. } => Some(duration_ticks),
            _ => None,
        })
        .expect("create_clip dispatches LoadMidiClipDirect");
    assert_eq!(duration, 4 * TPQ, "unchanged: one 4/4 bar of default length");
}

fn section_definition(id: u64, length_bars: u32) -> resonance_app::compose::SectionDefinitionState {
    use resonance_app::compose::{GenerateParams, SectionDefinitionState};
    use resonance_music_theory::MotifSource;
    SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [0, 0, 0],
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    }
}
