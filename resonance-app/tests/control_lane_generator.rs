//! `section.set_lane_generator` (ba doc #268, todo #1168), driven
//! through the real `update()` path with synthesized requests.
//!
//! This is the method that makes the whole `vocal.*` namespace reachable
//! over the wire, so the interesting cases are the vocal-lane install and
//! the track-kind guards; the melodic kinds and `manual` share the same
//! code path.

use resonance_app::compose::LaneGeneratorKindTag;
use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::section::{
    self as proto, LaneKind, SetLaneGeneratorParams, SetLaneGeneratorResult,
};
use resonance_control::{ErrorKind, Request, Response};

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-lane-generator.rprj"));
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

/// A bare 4-bar section — deliberately WITHOUT chords, so the tests also
/// prove this method does not inherit `generate.part`'s chord gate.
fn section(app: &mut Resonance) -> SectionDefinitionId {
    let response = call(
        app,
        "section.create",
        &proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
        },
    );
    response
        .result::<proto::CreateResult>()
        .expect("section.create succeeds")
        .section_id
}

fn params(
    section_id: SectionDefinitionId,
    track_id: ProtoTrackId,
    kind: LaneKind,
) -> SetLaneGeneratorParams {
    SetLaneGeneratorParams {
        section_id,
        track_id,
        kind,
        seed: None,
        options: None,
    }
}

#[test]
fn installs_a_vocal_lane_without_chords_or_derived_clips() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(40, TrackType::Vocal);

    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(40), LaneKind::Vocal),
    );
    let result: SetLaneGeneratorResult = response.result().expect("set_lane_generator succeeds");
    assert!(result.revision > 0, "the edit bumps the revision counter");

    assert_eq!(
        app.test_lane_generator_tag(u64::from(section_id), 40),
        Some(LaneGeneratorKindTag::Vocal)
    );
    // Installing a generator derives nothing — the vocal material arrives
    // later via vocal.set_lyrics / vocal.render.
    assert_eq!(app.test_derived_clip_count(40), 0);
}

#[test]
fn melodic_kinds_install_without_generating() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(41, TrackType::Instrument);

    for (kind, tag) in [
        (LaneKind::Bass, LaneGeneratorKindTag::Bass),
        (LaneKind::Melody, LaneGeneratorKindTag::Melody),
        (LaneKind::Pad, LaneGeneratorKindTag::Pad),
    ] {
        let response = call(
            &mut app,
            "section.set_lane_generator",
            &params(section_id, ProtoTrackId(41), kind),
        );
        let _: SetLaneGeneratorResult = response.result().expect("set_lane_generator succeeds");
        assert_eq!(
            app.test_lane_generator_tag(u64::from(section_id), 41),
            Some(tag)
        );
    }
    assert_eq!(app.test_derived_clip_count(41), 0);
}

#[test]
fn manual_clears_the_lane() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(42, TrackType::Instrument);

    let _ = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(42), LaneKind::Bass),
    );
    assert!(app
        .test_lane_generator_tag(u64::from(section_id), 42)
        .is_some());

    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(42), LaneKind::Manual),
    );
    let _: SetLaneGeneratorResult = response.result().expect("manual succeeds");
    assert_eq!(app.test_lane_generator_tag(u64::from(section_id), 42), None);

    // Idempotent: clearing an already-clear lane is not an error.
    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(42), LaneKind::Manual),
    );
    let _: SetLaneGeneratorResult = response.result().expect("manual is idempotent");
}

#[test]
fn explicit_seed_and_options_reach_the_config() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(43, TrackType::Instrument);

    let mut p = params(section_id, ProtoTrackId(43), LaneKind::Bass);
    p.seed = Some(4242);
    p.options = Some(serde_json::json!({
        "style": "Walking",
        "base_note": 31,
        "velocity": 0.9,
    }));
    let response = call(&mut app, "section.set_lane_generator", &p);
    let _: SetLaneGeneratorResult = response.result().expect("options deserialize");
    assert_eq!(
        app.test_lane_generator_seed(u64::from(section_id), 43),
        Some(4242)
    );
}

#[test]
fn unparseable_options_are_invalid_params() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(44, TrackType::Instrument);

    let mut p = params(section_id, ProtoTrackId(44), LaneKind::Bass);
    p.options = Some(serde_json::json!({ "base_note": "not a number" }));
    let response = call(&mut app, "section.set_lane_generator", &p);
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("options"), "{message}");
    assert_eq!(app.test_lane_generator_tag(u64::from(section_id), 44), None);
}

#[test]
fn vocal_kind_rejects_an_instrument_track() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(45, TrackType::Instrument);

    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(45), LaneKind::Vocal),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("vocal track"), "{message}");
    assert_eq!(app.test_lane_generator_tag(u64::from(section_id), 45), None);
}

#[test]
fn melodic_kinds_reject_a_vocal_track() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(46, TrackType::Vocal);

    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(46), LaneKind::Melody),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("synth instrument track"), "{message}");
}

#[test]
fn unknown_section_and_track_are_not_found() {
    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(47, TrackType::Vocal);

    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(SectionDefinitionId(9999), ProtoTrackId(47), LaneKind::Vocal),
    );
    let message = expect_error(response, ErrorKind::NotFound);
    assert!(message.contains("section definition"), "{message}");

    let response = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, ProtoTrackId(9999), LaneKind::Vocal),
    );
    let message = expect_error(response, ErrorKind::NotFound);
    assert!(message.contains("track"), "{message}");
}

#[test]
fn installed_vocal_lane_unblocks_the_vocal_namespace() {
    use resonance_control::methods::vocal as vocal_proto;
    use resonance_control::MutationAck;

    let mut app = app_with_project();
    let section_id = section(&mut app);
    app.test_add_track(48, TrackType::Vocal);
    let track = ProtoTrackId(48);

    // Before the lane exists, vocal.set_lyrics has nothing to target.
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &vocal_proto::SetLyricsParams {
            track_id: track,
            text: "hello world".to_owned(),
        },
    );
    assert!(
        response.error.is_some(),
        "vocal.set_lyrics needs a vocal lane first"
    );

    let _ = call(
        &mut app,
        "section.set_lane_generator",
        &params(section_id, track, LaneKind::Vocal),
    );

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &vocal_proto::SetLyricsParams {
            track_id: track,
            text: "hello world".to_owned(),
        },
    );
    let _: MutationAck = response
        .result()
        .expect("vocal.set_lyrics reaches the installed lane");
    assert_eq!(app.test_vocal_lines(u64::from(section_id), 48), vec!["hello world"]);
}
