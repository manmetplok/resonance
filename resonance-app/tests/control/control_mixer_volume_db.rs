//! `mixer.set_volume_db` — set a fader in the unit balance work is done
//! in (ba doc #273, todo #1222).
//!
//! The app already stores `TrackState.volume` in dB, so this method is
//! strictly fewer conversions than the linear `mixer.set_volume` it sits
//! beside. These tests pin the round-trip against `song.tracks`, the
//! range/finiteness rejections, and that the edit lands in undo history
//! like a manual fader move.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::TrackType;
use resonance_control::methods::song::TracksView;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // Undo history only records once the project has a path on disk
    // (`can_record_undo`), which the undo test below depends on.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-volume-db-test.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
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

fn set_db(app: &mut Resonance, track_id: u64, volume_db: f32) -> Response {
    call(
        app,
        "mixer.set_volume_db",
        serde_json::json!({"track_id": track_id, "volume_db": volume_db}),
    )
}

fn fader(app: &mut Resonance, track_id: u64) -> (f32, f32) {
    let view: TracksView = roundtrip(app, Request::without_params(99, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    let t = view
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == track_id)
        .expect("track in song.tracks");
    (t.summary.volume_db, t.summary.volume)
}

#[test]
fn set_volume_db_round_trips_through_song_tracks() {
    let mut app = app();
    let ack: MutationAck = set_db(&mut app, TRACK, -6.0)
        .result()
        .expect("mixer.set_volume_db succeeds");
    assert!(ack.revision > 0, "mutation must carry a revision");

    let (db, linear) = fader(&mut app, TRACK);
    assert!((db - -6.0).abs() < 1e-4, "volume_db {db}");
    // -6 dB is ~0.501 linear.
    assert!((linear - 0.501_187).abs() < 1e-3, "volume {linear}");
}

#[test]
fn zero_db_is_unity() {
    let mut app = app();
    let _: MutationAck = set_db(&mut app, TRACK, -12.0).result().expect("succeeds");
    let _: MutationAck = set_db(&mut app, TRACK, 0.0).result().expect("succeeds");
    let (db, linear) = fader(&mut app, TRACK);
    assert!(db.abs() < 1e-6, "volume_db {db}");
    assert!((linear - 1.0).abs() < 1e-6, "volume {linear}");
}

#[test]
fn the_whole_fader_range_is_accepted() {
    let mut app = app();
    for db in [-60.0f32, -30.0, 0.0, 6.0] {
        let response = set_db(&mut app, TRACK, db);
        assert!(
            response.result::<MutationAck>().is_ok(),
            "{db} dB should be inside the fader range"
        );
        assert!((fader(&mut app, TRACK).0 - db).abs() < 1e-4);
    }
}

#[test]
fn out_of_range_is_invalid_params_and_leaves_the_fader_alone() {
    let mut app = app();
    for db in [-60.5f32, 6.5, -1_000.0, 120.0] {
        let response = set_db(&mut app, TRACK, db);
        let error = response
            .error
            .unwrap_or_else(|| panic!("{db} dB should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {db} dB");
        assert!(
            error.message.contains("-60") && error.message.contains('6'),
            "the error must name the accepted range: {}",
            error.message
        );
    }
    assert!(fader(&mut app, TRACK).0.abs() < 1e-6);
}

/// JSON has no NaN/Infinity literal, so a non-finite value arrives as
/// `null` — which must still be `invalid_params`, never a silent no-op.
#[test]
fn non_finite_volume_db_is_invalid_params() {
    let mut app = app();
    let response = call(
        &mut app,
        "mixer.set_volume_db",
        serde_json::json!({"track_id": TRACK, "volume_db": f32::NAN}),
    );
    assert_eq!(
        response.error.expect("NaN rejected").kind(),
        ErrorKind::InvalidParams
    );
    assert!(fader(&mut app, TRACK).0.abs() < 1e-6);
}

#[test]
fn unknown_track_is_not_found() {
    let mut app = app();
    let error = set_db(&mut app, 4_242, -3.0)
        .error
        .expect("unknown track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn the_edit_is_undoable_like_a_manual_fader_move() {
    let mut app = app();
    let _: MutationAck = set_db(&mut app, TRACK, -9.0).result().expect("succeeds");
    assert!((fader(&mut app, TRACK).0 - -9.0).abs() < 1e-4);

    let _ = app.update(Message::Undo);
    assert!(
        fader(&mut app, TRACK).0.abs() < 1e-4,
        "undo should restore the fader to 0 dB"
    );
}
