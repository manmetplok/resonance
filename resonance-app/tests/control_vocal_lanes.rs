//! `song.vocal` per-lane identity + counts, and `section_id` addressing
//! on the `vocal.*` mutations (ba doc #269 FR-3/FR-7).
//!
//! Before this, `song.vocal` reported `{lines, render_state, revision,
//! track_id}` with no section identity, so a client could not tell which
//! lane a write had hit — `vocal.set_lyrics` silently resolves a track's
//! first vocal lane by start bar. These tests set up a track singing in
//! two sections and pin down both halves of the fix.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::VocalView;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const TRACK: u64 = 50;

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-lanes.rprj"));
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

/// Create a section placed at `start_bar` (1-based) with a vocal lane on
/// `TRACK`, returning its definition id.
fn vocal_lane(app: &mut Resonance, name: &str, start_bar: u32) -> u64 {
    let response = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: name.to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("section.create succeeds")
        .section_id;
    let _ = call(
        app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: section_id,
            start_bar,
        },
    );
    let def = u64::from(section_id);
    app.test_install_vocal_lane(def, TRACK);
    def
}

/// A track singing in two sections: Verse at bar 1, Chorus at bar 5.
fn two_lane_app() -> (Resonance, u64, u64) {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let verse = vocal_lane(&mut app, "Verse", 1);
    let chorus = vocal_lane(&mut app, "Chorus", 5);
    (app, verse, chorus)
}

fn vocal_view(app: &mut Resonance) -> VocalView {
    roundtrip(
        app,
        Request::new(9, "song.vocal", &serde_json::json!({ "track_id": TRACK })).unwrap(),
    )
    .result()
    .expect("song.vocal succeeds")
}

// ---------------- FR-3/FR-7: song.vocal lane identity ----------------

#[test]
fn lanes_report_section_identity_in_placement_order() {
    let (mut app, verse, chorus) = two_lane_app();

    let view = vocal_view(&mut app);
    let ids: Vec<u64> = view
        .lanes
        .iter()
        .map(|l| u64::from(l.definition_id))
        .collect();
    assert_eq!(ids, vec![verse, chorus], "lanes read in placement order");
    assert_eq!(view.lanes[0].name, "Verse");
    assert_eq!(view.lanes[1].name, "Chorus");
    // Placement bars are 1-based on the wire.
    assert_eq!(view.lanes[0].start_bar, Some(1));
    assert_eq!(view.lanes[1].start_bar, Some(5));
}

#[test]
fn an_unplaced_lane_reports_no_start_bar() {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Outro".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let def = u64::from(
        response
            .result::<section_proto::CreateResult>()
            .expect("create succeeds")
            .section_id,
    );
    app.test_install_vocal_lane(def, TRACK);

    let view = vocal_view(&mut app);
    assert_eq!(view.lanes.len(), 1);
    assert_eq!(view.lanes[0].start_bar, None);
}

#[test]
fn counts_are_reported_per_lane_and_flag_no_mismatch_while_ungenerated() {
    let (mut app, verse, _chorus) = two_lane_app();
    // Two lines of one syllable each, so the count is the engine's, not
    // a guess: the whole point of reporting it is that G2P decides
    // syllabification, not a by-eye reading.
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(verse)),
            text: "go\nstay".to_owned(),
        },
    );

    let view = vocal_view(&mut app);
    let lane = &view.lanes[0];
    assert_eq!(u64::from(lane.definition_id), verse);
    assert_eq!(lane.syllable_count, 2);
    // Nothing generated yet, so there is no mismatch to report — a
    // note_count of 0 is "not generated", not "wrong".
    assert_eq!(lane.note_count, 0);
    assert!(!lane.counts_mismatch);
    // The counts are per lane: the Chorus lane still carries its own
    // (default) draft and reports its own, different, count.
    assert_ne!(view.lanes[1].syllable_count, lane.syllable_count);
}

// ---------------- FR-3 part 2: section_id addressing ----------------

#[test]
fn section_id_writes_the_addressed_lane() {
    let (mut app, verse, chorus) = two_lane_app();

    let verse_before = app.test_vocal_lines(verse, TRACK);

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            text: "sing it louder".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");

    assert_eq!(app.test_vocal_lines(chorus, TRACK), vec!["sing it louder"]);
    assert_eq!(
        app.test_vocal_lines(verse, TRACK),
        verse_before,
        "the first lane is untouched — before FR-3 this write landed there"
    );
}

#[test]
fn an_omitted_section_id_still_writes_the_first_lane() {
    let (mut app, verse, chorus) = two_lane_app();

    let chorus_before = app.test_vocal_lines(chorus, TRACK);

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            text: "first lane".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");

    assert_eq!(app.test_vocal_lines(verse, TRACK), vec!["first lane"]);
    assert_eq!(app.test_vocal_lines(chorus, TRACK), chorus_before);
}

#[test]
fn set_line_addresses_the_same_lane() {
    let (mut app, verse, chorus) = two_lane_app();
    let verse_before = app.test_vocal_lines(verse, TRACK);
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            text: "one\ntwo".to_owned(),
        },
    );

    let response = call(
        &mut app,
        "vocal.set_line",
        &proto::SetLineParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            line_index: 1,
            text: "TWO".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_line succeeds");
    assert_eq!(app.test_vocal_lines(chorus, TRACK), vec!["one", "TWO"]);
    assert_eq!(app.test_vocal_lines(verse, TRACK), verse_before);
}

#[test]
fn a_section_without_a_vocal_lane_on_the_track_is_rejected() {
    let (mut app, _verse, _chorus) = two_lane_app();
    // A section that exists but carries no vocal lane for this track.
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Bridge".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let bare = u64::from(
        response
            .result::<section_proto::CreateResult>()
            .expect("create succeeds")
            .section_id,
    );

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(bare)),
            text: "nope".to_owned(),
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("no vocal lane"), "{message}");
}

#[test]
fn an_unknown_section_id_is_not_found() {
    let (mut app, _verse, _chorus) = two_lane_app();
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(9999)),
            text: "nope".to_owned(),
        },
    );
    let message = expect_error(response, ErrorKind::NotFound);
    assert!(message.contains("section definition"), "{message}");
}
