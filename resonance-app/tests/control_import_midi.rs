//! `notes.import_midi` — Standard MIDI File import over the control API
//! (ba doc #273, todo #1195).
//!
//! Bulk material otherwise travels as a JSON note array through a tool
//! call — thousands of notes that way dominate a session's cost. The
//! parser is the app's existing `resonance_audio::midi_io`; this is
//! wiring, and these tests pin the wiring: the target resolution, the
//! multi-track refusal, the size/validity rejections, and that the
//! import is one undoable edit whose result reports what landed.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::midi_io::encode_midi;
use resonance_audio::types::{MidiNote, TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_control::methods::notes::ImportMidiResult;
use resonance_control::methods::song::NotesView;
use resonance_control::{ErrorKind, Request, Response};

const TRACK: u64 = 1;
const AUDIO_TRACK: u64 = 2;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-import-midi.rprj"));
    app.test_set_sample_rate(48_000);
    app.test_rebuild_tempo_map();
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_add_track(AUDIO_TRACK, TrackType::Audio);
    app
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

fn note(pitch: u8, start_beat: u64, beats: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 100.0 / 127.0,
        start_tick: start_beat * TICKS_PER_QUARTER_NOTE,
        duration_ticks: beats * TICKS_PER_QUARTER_NOTE,
    }
}

/// A small Format-0 SMF: C4 E4 G4, one beat each.
fn fixture() -> Vec<u8> {
    encode_midi(&[note(60, 0, 1), note(64, 1, 1), note(67, 2, 1)]).expect("encode")
}

fn base64_of(bytes: &[u8]) -> String {
    // The same alphabet the handler decodes with, hand-rolled so the
    // test does not depend on the crate's API surface.
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn notes_of(app: &mut Resonance, clip_id: u64) -> NotesView {
    call(app, "song.notes", serde_json::json!({"clip_id": clip_id}))
        .result()
        .expect("song.notes succeeds")
}

#[test]
fn importing_onto_a_track_creates_a_clip_and_reports_what_landed() {
    let mut app = app();
    let result: ImportMidiResult = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({
            "track_id": TRACK,
            "data_base64": base64_of(&fixture()),
            "name": "Bassline",
        }),
    )
    .result()
    .expect("notes.import_midi succeeds");

    assert_eq!(result.track_id.0, TRACK);
    assert_eq!(result.note_count, 3);
    assert_eq!(result.source_track, 0);
    assert!((result.length_beats - 3.0).abs() < 1e-6);
    assert!(result.revision > 0);

    // The notes are readable immediately — no follow-up round trip is
    // needed to find out whether the import worked.
    let view = notes_of(&mut app, result.clip_id.0);
    let pitches: Vec<u8> = view.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![60, 64, 67]);
    let starts: Vec<f64> = view.notes.iter().map(|n| n.start_beat).collect();
    assert_eq!(starts, vec![0.0, 1.0, 2.0]);
}

#[test]
fn importing_from_a_file_path_works_too() {
    let mut app = app();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("part.mid");
    std::fs::write(&path, fixture()).expect("write fixture");

    let result: ImportMidiResult = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": TRACK, "path": path.to_str().unwrap()}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(result.note_count, 3);
    assert_eq!(notes_of(&mut app, result.clip_id.0).notes.len(), 3);
}

#[test]
fn importing_into_an_existing_clip_replaces_its_notes() {
    let mut app = app();
    let created: resonance_control::methods::notes::CreateClipResult = call(
        &mut app,
        "notes.create_clip",
        serde_json::json!({"track_id": TRACK, "start_bar": 1, "length_beats": 8.0}),
    )
    .result()
    .expect("notes.create_clip succeeds");
    let clip_id = created.clip_id.0;
    let _: resonance_control::methods::notes::InsertResult = call(
        &mut app,
        "notes.insert",
        serde_json::json!({"clip_id": clip_id, "pitch": 36, "start_beat": 0.0, "duration_beats": 1.0}),
    )
    .result()
    .expect("notes.insert succeeds");

    let result: ImportMidiResult = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"clip_id": clip_id, "data_base64": base64_of(&fixture())}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(result.clip_id.0, clip_id);
    assert_eq!(result.track_id.0, TRACK);

    let pitches: Vec<u8> = notes_of(&mut app, clip_id)
        .notes
        .iter()
        .map(|n| n.pitch)
        .collect();
    assert_eq!(pitches, vec![60, 64, 67], "the old note was replaced");
}

/// The whole import is ONE undoable edit, like any other control write.
#[test]
fn an_import_is_a_single_undoable_edit() {
    let mut app = app();
    let before = app.revision();
    let result: ImportMidiResult = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": TRACK, "data_base64": base64_of(&fixture())}),
    )
    .result()
    .expect("succeeds");
    // Two committed steps at most: create the clip, write its notes —
    // not one per note.
    assert!(
        app.revision() - before <= 2,
        "an import must not record one entry per note (got {})",
        app.revision() - before
    );
    assert_eq!(notes_of(&mut app, result.clip_id.0).notes.len(), 3);
}

#[test]
fn a_multi_track_file_is_refused_rather_than_flattened() {
    let mut app = app();
    // Two note-carrying tracks in one Format-1 file.
    let bytes = multi_track_smf();

    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": TRACK, "data_base64": base64_of(&bytes)}),
    )
    .error
    .expect("a multi-track file needs a selector");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("source_track"),
        "the error must say how to pick: {}",
        error.message
    );
    assert!(
        error.message.contains("notes)"),
        "the error must list the tracks with their note counts: {}",
        error.message
    );

    // Naming one imports exactly that part.
    let result: ImportMidiResult = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({
            "track_id": TRACK,
            "data_base64": base64_of(&bytes),
            "source_track": 2,
        }),
    )
    .result()
    .expect("succeeds with a selector");
    assert_eq!(result.source_track, 2);
    let pitches: Vec<u8> = notes_of(&mut app, result.clip_id.0)
        .notes
        .iter()
        .map(|n| n.pitch)
        .collect();
    assert_eq!(pitches, vec![48], "only the named track was imported");
}

#[test]
fn bad_sources_and_targets_are_rejected() {
    let mut app = app();
    let data = base64_of(&fixture());

    for params in [
        // Neither source, and both sources.
        serde_json::json!({"track_id": TRACK}),
        serde_json::json!({"track_id": TRACK, "path": "/tmp/x.mid", "data_base64": data.clone()}),
        // Neither target, and both targets.
        serde_json::json!({"data_base64": data.clone()}),
        serde_json::json!({"track_id": TRACK, "clip_id": 1, "data_base64": data.clone()}),
        // A relative path.
        serde_json::json!({"track_id": TRACK, "path": "part.mid"}),
    ] {
        let error = call(&mut app, "notes.import_midi", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }

    // A malformed file reports the parser's own reason.
    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": TRACK, "data_base64": base64_of(b"not a midi file")}),
    )
    .error
    .expect("garbage rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("MIDI file"), "{}", error.message);

    // Not base64 at all.
    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": TRACK, "data_base64": "!!!not base64!!!"}),
    )
    .error
    .expect("bad base64 rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    // Unknown ids.
    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": 4242, "data_base64": data.clone()}),
    )
    .error
    .expect("unknown track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"clip_id": 4242, "data_base64": data.clone()}),
    )
    .error
    .expect("unknown clip rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    // An audio track cannot hold MIDI.
    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": AUDIO_TRACK, "data_base64": data}),
    )
    .error
    .expect("audio track rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
}

/// Oversized payloads are refused with the limit, never truncated.
#[test]
fn an_oversized_payload_is_refused_with_the_limit() {
    let mut app = app();
    let huge = "A".repeat(8 * 1024 * 1024);
    let error = call(
        &mut app,
        "notes.import_midi",
        serde_json::json!({"track_id": TRACK, "data_base64": huge}),
    )
    .error
    .expect("oversized payload rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("limit"),
        "the error must state the limit: {}",
        error.message
    );
}

#[test]
fn import_midi_is_advertised_in_the_handshake() {
    assert!(resonance_control::methods::capabilities().contains(&"notes.import_midi"));
}

/// A Format-1 file: conductor track, then two note tracks.
fn multi_track_smf() -> Vec<u8> {
    use resonance_audio::midi_io::{write_midi_project, MidiTrackSource};
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("multi.mid");
    let lead = [note(72, 0, 1)];
    let bass = [note(48, 0, 1)];
    let tempo_map = resonance_audio::types::TempoMap::default();
    write_midi_project(
        &path,
        &tempo_map,
        &[
            MidiTrackSource {
                name: "Lead",
                notes: &lead,
            },
            MidiTrackSource {
                name: "Bass",
                notes: &bass,
            },
        ],
    )
    .expect("write multi-track smf");
    std::fs::read(&path).expect("read back")
}
