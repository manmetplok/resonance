//! Sub-track parentage, output routing and dB faders on the `song.*`
//! views (ba doc #273, todo #1221).
//!
//! These three fields already existed in the app and were simply not on
//! the wire: a client could not tell a multi-output instrument's child
//! track from an independent one (the only signal was a `→` in the
//! name), could not see where a track was routed, and had to convert
//! every fader out of linear gain to reason about balance.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, TrackOutput, TrackType};
use resonance_control::methods::song::{SongSummary, TracksView};
use resonance_control::{Request, TrackOutput as WireTrackOutput};
use crate::common::roundtrip;

const PARENT: u64 = 1;
const KICK: u64 = 2;
const SNARE: u64 = 3;
const PLAIN: u64 = 4;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_add_track(PARENT, TrackType::Instrument);
    app.test_add_track(PLAIN, TrackType::Audio);
    // The two sub-tracks `ensure_subtracks` would create for output
    // ports 1 and 2 of a multi-output instrument.
    app.test_push_track(TrackState::new_sub_track(
        KICK,
        2,
        "Drums → Kick".to_owned(),
        PARENT,
        1,
    ));
    app.test_push_track(TrackState::new_sub_track(
        SNARE,
        3,
        "Drums → Snare".to_owned(),
        PARENT,
        2,
    ));
    app
}

fn summary(app: &mut Resonance) -> SongSummary {
    roundtrip(app, Request::without_params(1, "song.summary"))
        .result()
        .expect("song.summary succeeds")
}

fn tracks(app: &mut Resonance) -> TracksView {
    roundtrip(app, Request::without_params(2, "song.tracks"))
        .result()
        .expect("song.tracks succeeds")
}

fn raw(app: &mut Resonance, method: &str) -> serde_json::Value {
    let response = roundtrip(app, Request::without_params(3, method));
    serde_json::to_value(&response).expect("response serializes")["result"].clone()
}

#[test]
fn sub_tracks_report_their_parent() {
    let mut app = app();
    let view = summary(&mut app);

    let by_id = |id: u64| {
        view.tracks
            .iter()
            .find(|t| t.id.0 == id)
            .unwrap_or_else(|| panic!("track {id} in song.summary"))
    };
    assert_eq!(by_id(KICK).parent_id.map(|p| p.0), Some(PARENT));
    assert_eq!(by_id(SNARE).parent_id.map(|p| p.0), Some(PARENT));
    // The parent itself and an unrelated track are not sub-tracks.
    assert_eq!(by_id(PARENT).parent_id, None);
    assert_eq!(by_id(PLAIN).parent_id, None);

    // song.tracks reports it too (TrackDetail flattens TrackSummary).
    let detail = tracks(&mut app);
    let kick = detail
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == KICK)
        .expect("kick in song.tracks");
    assert_eq!(kick.summary.parent_id.map(|p| p.0), Some(PARENT));
}

/// A non-sub-track must omit `parent_id` entirely rather than emit
/// `null` noise on every line.
#[test]
fn parent_id_is_elided_for_ordinary_tracks() {
    let mut app = app();
    let result = raw(&mut app, "song.summary");
    let lines = result["tracks"].as_array().expect("tracks array");

    let line = |id: u64| {
        lines
            .iter()
            .find(|t| t["id"] == serde_json::json!(id))
            .unwrap_or_else(|| panic!("track {id} on the wire"))
    };
    assert!(
        line(PLAIN).get("parent_id").is_none(),
        "ordinary track carried a parent_id: {}",
        line(PLAIN)
    );
    assert_eq!(line(KICK)["parent_id"], serde_json::json!(PARENT));
}

#[test]
fn volume_db_agrees_with_linear_volume() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(PARENT, -6.0)));

    let view = summary(&mut app);
    let parent = view
        .tracks
        .iter()
        .find(|t| t.id.0 == PARENT)
        .expect("parent track");
    assert!(
        (parent.volume_db - -6.0).abs() < 1e-4,
        "volume_db {}",
        parent.volume_db
    );
    let expected = 10f32.powf(parent.volume_db / 20.0);
    assert!(
        (parent.volume - expected).abs() < 1e-4,
        "volume {} != 10^(volume_db/20) = {expected}",
        parent.volume
    );

    // A fader at unity: 0 dB, gain 1.0.
    let plain = view
        .tracks
        .iter()
        .find(|t| t.id.0 == PLAIN)
        .expect("plain track");
    assert!((plain.volume_db - 0.0).abs() < 1e-6);
    assert!((plain.volume - 1.0).abs() < 1e-6);
}

#[test]
fn output_reports_master_and_bus_routing() {
    let mut app = app();
    let bus_id = 9_000;
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id,
        name: "Drum Bus".to_owned(),
    });
    let _ = app.update(Message::Track(TrackMessage::SetTrackOutput(
        KICK,
        TrackOutput::Bus(bus_id),
    )));

    let view = summary(&mut app);
    let by_id = |id: u64| {
        view.tracks
            .iter()
            .find(|t| t.id.0 == id)
            .unwrap_or_else(|| panic!("track {id} in song.summary"))
    };
    assert_eq!(by_id(KICK).output, WireTrackOutput::Bus(bus_id.into()));
    assert_eq!(by_id(KICK).output.bus_id().map(|b| b.0), Some(bus_id));
    assert_eq!(by_id(SNARE).output, WireTrackOutput::Master);
    // The bus itself appears as a track line and feeds master.
    assert_eq!(by_id(bus_id).output, WireTrackOutput::Master);

    // Wire shape: "master" or {"bus_id": N}.
    let result = raw(&mut app, "song.summary");
    let lines = result["tracks"].as_array().expect("tracks array");
    let line = |id: u64| {
        lines
            .iter()
            .find(|t| t["id"] == serde_json::json!(id))
            .unwrap_or_else(|| panic!("track {id} on the wire"))
    };
    assert_eq!(line(KICK)["output"], serde_json::json!({"bus_id": bus_id}));
    assert_eq!(line(SNARE)["output"], serde_json::json!("master"));
}
