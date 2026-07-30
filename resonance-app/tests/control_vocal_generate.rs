//! `vocal.generate` (ba doc #269 FR-2) through the real update path.
//!
//! `generate.part` refuses vocal tracks and the SVS render reads its
//! notes from the lane's derived clip, so before this method a vocal
//! lane reachable over the wire still had nothing to sing: render_state
//! stayed `not_rendered` with no clips. The acceptance case at the
//! bottom walks the whole GUI-free path.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::NotesView;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, KeyScale, MutationAck, Request, Response};

const TRACK: u64 = 60;

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-generate.rprj"));
    app
}

fn roundtrip(app: &mut Resonance, request: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

/// A section with chords, and a vocal lane on `TRACK` installed the way
/// a remote client would: `section.set_lane_generator kind = vocal`.
fn vocal_lane_with_chords(app: &mut Resonance) -> u64 {
    let response = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("section.create succeeds")
        .section_id;

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(
        ["i", "iv", "v", "i"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    let _ = call(app, "harmony.apply_progression", &params);

    app.test_add_track(TRACK, TrackType::Vocal);
    let response = call(
        app,
        "section.set_lane_generator",
        &section_proto::SetLaneGeneratorParams {
            section_id,
            track_id: ProtoTrackId(TRACK),
            kind: section_proto::LaneKind::Vocal,
            seed: None,
            options: None,
        },
    );
    response
        .result::<section_proto::SetLaneGeneratorResult>()
        .expect("the vocal lane installs");
    u64::from(section_id)
}

fn generate(app: &mut Resonance, lyrics: bool, seed: Option<u64>) -> Response {
    call(
        app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            seed,
            lyrics,
        },
    )
}

fn notes_of(app: &mut Resonance, clip_id: u64) -> NotesView {
    roundtrip(
        app,
        Request::new(9, "song.notes", &serde_json::json!({ "clip_id": clip_id })).unwrap(),
    )
    .result()
    .expect("song.notes succeeds")
}

#[test]
fn generate_produces_a_derived_clip_with_notes() {
    let mut app = app_with_project();
    let _def = vocal_lane_with_chords(&mut app);
    assert_eq!(
        app.test_derived_clip_count(TRACK),
        0,
        "installing the lane derives nothing"
    );

    let response = generate(&mut app, true, Some(7));
    let result: proto::GenerateResult = response.result().expect("vocal.generate succeeds");

    assert!(app.test_derived_clip_count(TRACK) > 0, "a clip was derived");
    let view = notes_of(&mut app, u64::from(result.clip_id));
    assert!(
        !view.notes.is_empty(),
        "the returned clip carries the generated melody"
    );
}

#[test]
fn an_explicit_seed_is_reproducible() {
    let pitches = |seed: u64| {
        let mut app = app_with_project();
        let _ = vocal_lane_with_chords(&mut app);
        let result: proto::GenerateResult = generate(&mut app, true, Some(seed))
            .result()
            .expect("vocal.generate succeeds");
        let clip = u64::from(result.clip_id);
        notes_of(&mut app, clip)
            .notes
            .iter()
            .map(|n| n.pitch)
            .collect::<Vec<u8>>()
    };
    assert_eq!(pitches(1234), pitches(1234), "the same seed repeats");
}

#[test]
fn lyrics_false_generates_melody_only() {
    let mut app = app_with_project();
    let def = vocal_lane_with_chords(&mut app);

    // Lyrics the client wrote itself — generation must not discard them.
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            text: "hold the line\nlet it go".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");
    let written = app.test_vocal_lines(def, TRACK);
    assert_eq!(written, vec!["hold the line", "let it go"]);

    let response = generate(&mut app, false, Some(3));
    let result: proto::GenerateResult = response.result().expect("melody-only generate succeeds");

    assert_eq!(
        app.test_vocal_lines(def, TRACK),
        written,
        "lyrics: false leaves the written lyrics intact"
    );
    assert!(!notes_of(&mut app, u64::from(result.clip_id)).notes.is_empty());
}

#[test]
fn a_section_without_chords_is_rejected() {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Intro".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("create succeeds")
        .section_id;
    let _ = call(
        &mut app,
        "section.set_lane_generator",
        &section_proto::SetLaneGeneratorParams {
            section_id,
            track_id: ProtoTrackId(TRACK),
            kind: section_proto::LaneKind::Vocal,
            seed: None,
            options: None,
        },
    );

    let message = expect_error(generate(&mut app, true, None), ErrorKind::InvalidParams);
    assert!(message.contains("no chords"), "{message}");
}

#[test]
fn a_track_without_a_vocal_lane_is_rejected() {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let message = expect_error(generate(&mut app, true, None), ErrorKind::InvalidParams);
    assert!(message.contains("no vocal lane"), "{message}");
}

/// The regression the field report asked for (doc #271 V1): author notes
/// into a lane, render, and assert the notes survive.
///
/// `vocal.render` used to run the full generate path, so it re-derived
/// the melody from the lane's seed and threw the authored notes away —
/// silently, since every call still reported success. Three different
/// authored note sets rendered to byte-identical audio.
#[test]
fn render_sings_the_authored_notes_and_never_rewrites_them() {
    let mut app = app_with_project();
    let _def = vocal_lane_with_chords(&mut app);

    let result: proto::GenerateResult = generate(&mut app, true, Some(42))
        .result()
        .expect("vocal.generate succeeds");
    let clip = u64::from(result.clip_id);
    let syllables = notes_of(&mut app, clip).notes.len();
    assert!(syllables > 0);

    // Author a deliberately unmistakable melody over the whole lane:
    // one pitch, on the beat, uniform duration and velocity. Keep the
    // note count equal to the syllable count so the render is valid.
    let authored: Vec<serde_json::Value> = (0..syllables)
        .map(|i| {
            serde_json::json!({
                "pitch": 62,
                "start_beat": i as f64 * 2.0,
                "duration_beats": 1.0,
                "velocity": 100,
            })
        })
        .collect();
    call(
        &mut app,
        "notes.replace_all",
        &serde_json::json!({ "clip_id": clip, "notes": authored }),
    )
    .result::<resonance_control::methods::notes::InsertManyResult>()
    .expect("notes.replace_all succeeds");

    let before = notes_of(&mut app, clip).notes;
    assert!(before.iter().all(|n| n.pitch == 62), "authored as written");

    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(ProtoTrackId(TRACK)),
            section_id: None,
            voicebank: None,
        },
    );
    response
        .result::<resonance_control::job::JobStarted>()
        .expect("render returns a job");

    // The clip is the same clip, holding the same notes: render reads,
    // it does not write.
    let after = notes_of(&mut app, clip).notes;
    assert_eq!(after.len(), before.len(), "note count preserved");
    for (a, b) in after.iter().zip(before.iter()) {
        assert_eq!(a.pitch, b.pitch, "pitch survived the render");
        assert_eq!(a.start_tick, b.start_tick, "onset survived the render");
        assert_eq!(
            a.duration_ticks, b.duration_ticks,
            "duration survived the render"
        );
        assert_eq!(a.velocity, b.velocity, "velocity survived the render");
    }
}

/// A lane with no notes cannot be rendered into existence: render says
/// so precisely rather than quietly generating a melody nobody asked for.
#[test]
fn render_without_notes_points_at_generate() {
    let mut app = app_with_project();
    let _def = vocal_lane_with_chords(&mut app);

    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(ProtoTrackId(TRACK)),
            section_id: None,
            voicebank: None,
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("vocal.generate"), "unexpected: {message}");
}

#[test]
fn acceptance_lane_to_editable_notes_without_the_gui() {
    // doc #269 FR-2: section -> chords -> vocal track ->
    // set_lane_generator vocal -> vocal.generate -> notes.* edits ->
    // vocal.set_lyrics, entirely over the wire.
    let mut app = app_with_project();
    let def = vocal_lane_with_chords(&mut app);

    let result: proto::GenerateResult = generate(&mut app, true, Some(42))
        .result()
        .expect("vocal.generate succeeds");
    let clip = u64::from(result.clip_id);
    let before = notes_of(&mut app, clip).notes.len();
    assert!(before > 0);

    // The generated clip is an ordinary MIDI clip: notes.* can edit it.
    let response = call(
        &mut app,
        "notes.insert",
        &serde_json::json!({
            "clip_id": clip,
            "pitch": 72,
            "start_beat": 0.0,
            "duration_beats": 1.0,
        }),
    );
    response
        .result::<resonance_control::methods::notes::InsertResult>()
        .expect("notes.insert into the derived clip succeeds");
    assert_eq!(notes_of(&mut app, clip).notes.len(), before + 1);

    // And the lyrics side still works on the same lane.
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            text: "one more line".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");
    assert_eq!(app.test_vocal_lines(def, TRACK), vec!["one more line"]);
}
