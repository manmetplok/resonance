//! The control layer's shared reply contract (ba todo #1258).
//!
//! `update/control/reply.rs` is the one place a `MutationAck` is built
//! and the one place a `not_found` for a common wire id is worded. Nine
//! namespace modules used to carry their own copies, which agreed only
//! because `MutationAck` happens to have a single field today.
//!
//! These tests assert the two properties that de-duplication buys, from
//! the outside: every mutating method acknowledges with the SAME shape
//! (no namespace omitting a field), and a missing id reads the same
//! whichever namespace is asked.

use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{MidiNote, TrackType};
use resonance_control::{MutationAck, Response};
use crate::common::call;

const SR: u32 = 48_000;
const INSTRUMENT: u64 = 1;
const MIDI_CLIP: u64 = 100;
/// An id no track, bus, clip, section or placement will ever have here.
const MISSING: u64 = 9_999;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_add_track(INSTRUMENT, TrackType::Instrument);
    app.test_push_midi_clip(MidiClipState {
        id: MIDI_CLIP,
        track_id: INSTRUMENT,
        start_sample: 0,
        duration_ticks: 4 * 480,
        name: "riff".to_owned(),
        notes: vec![MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

/// The raw JSON of a successful reply's `result`.
fn result_json(response: &Response) -> serde_json::Value {
    response
        .result
        .clone()
        .unwrap_or_else(|| panic!("expected success, got {:?}", response.error))
}

#[test]
fn every_mutating_namespace_acknowledges_with_the_same_shape() {
    let mut app = app();

    // One mutation per namespace family that answers with a bare ack,
    // deliberately spread across the modules that each used to carry a
    // private copy of the helper.
    let acks = [
        (
            "mixer.set_volume_db",
            serde_json::json!({ "track_id": INSTRUMENT, "volume_db": -6.0 }),
        ),
        (
            "track.rename",
            serde_json::json!({ "track_id": INSTRUMENT, "name": "Lead" }),
        ),
        (
            "notes.edit",
            serde_json::json!({ "clip_id": MIDI_CLIP, "index": 0, "pitch": 62 }),
        ),
        (
            "master.set_volume",
            serde_json::json!({ "volume_db": -3.0 }),
        ),
        (
            "section.create",
            serde_json::json!({ "name": "Verse", "length_bars": 4 }),
        ),
    ];

    let mut last_revision = app.revision();
    for (method, params) in acks {
        let response = call(&mut app, method, params);
        let json = result_json(&response);
        let object = json
            .as_object()
            .unwrap_or_else(|| panic!("{method} must answer with an object, got {json}"));
        // `section.create` adds its new id; every mutating reply carries
        // the revision, and nothing carries LESS than that.
        assert!(
            object.contains_key("revision"),
            "{method} must acknowledge with a revision: {json}"
        );
        // The one construction site means the ack always deserializes as
        // the shared type — a hand-rolled copy that missed a future
        // field would still deserialize, but could not report a bumped
        // revision it never read.
        let ack: MutationAck = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{method} ack must be a MutationAck: {e} ({json})"));
        assert_eq!(
            ack.revision,
            app.revision(),
            "{method} must report the post-edit revision"
        );
        assert!(
            ack.revision > last_revision,
            "{method} is an undoable edit and must bump the revision"
        );
        last_revision = ack.revision;
    }
}

#[test]
fn a_missing_track_id_reads_the_same_in_every_namespace() {
    let mut app = app();

    let messages: Vec<String> = [
        (
            "mixer.set_pan",
            serde_json::json!({ "track_id": MISSING, "pan": 0.0 }),
        ),
        (
            "track.rename",
            serde_json::json!({ "track_id": MISSING, "name": "x" }),
        ),
        (
            "notes.create_clip",
            serde_json::json!({ "track_id": MISSING, "start_bar": 1, "length_bars": 1 }),
        ),
        (
            "external.enable",
            serde_json::json!({ "track_id": MISSING }),
        ),
        (
            "song.tracks",
            serde_json::json!({ "track_id": MISSING }),
        ),
    ]
    .into_iter()
    .map(|(method, params)| {
        call(&mut app, method, params)
            .error
            .unwrap_or_else(|| panic!("{method} must reject an unknown track"))
            .message
    })
    .collect();

    let expected = format!("no track with id {MISSING}");
    for message in &messages {
        assert_eq!(
            message, &expected,
            "every namespace words a missing track the same way"
        );
    }
}

#[test]
fn a_missing_bus_and_clip_id_have_one_wording_each() {
    let mut app = app();

    let bus = call(
        &mut app,
        "bus.set_volume",
        serde_json::json!({ "bus_id": MISSING, "volume_db": 0.0 }),
    )
    .error
    .expect("unknown bus rejected");
    assert_eq!(bus.message, format!("no bus with id {MISSING}"));

    let clip = call(
        &mut app,
        "notes.delete",
        serde_json::json!({ "clip_id": MISSING, "index": 0 }),
    )
    .error
    .expect("unknown clip rejected");
    assert_eq!(clip.message, format!("no MIDI clip with id {MISSING}"));

    // `song.notes` reads the same clips the editor writes.
    let read = call(
        &mut app,
        "song.notes",
        serde_json::json!({ "clip_id": MISSING }),
    )
    .error
    .expect("unknown clip rejected");
    assert_eq!(read.message, clip.message);
}

#[test]
fn a_missing_section_id_has_one_wording() {
    let mut app = app();

    let rename = call(
        &mut app,
        "section.rename",
        serde_json::json!({ "section_id": MISSING, "name": "x" }),
    )
    .error
    .expect("unknown section rejected");
    let chord = call(
        &mut app,
        "harmony.add_chord",
        serde_json::json!({
            "section_id": MISSING,
            "symbol": "Am",
            "start_beat": 0.0,
            "duration_beats": 4.0,
        }),
    )
    .error
    .expect("unknown section rejected");

    let expected = format!("no section definition with id {MISSING}");
    assert_eq!(rename.message, expected);
    assert_eq!(chord.message, expected);
}
