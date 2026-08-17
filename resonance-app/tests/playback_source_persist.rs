//! Project-file persistence for the external-instrument playback source
//! (doc #257, todo #1100). The mode shares monitor / record-arm's
//! engine-owned lifecycle, and those round-trip through the project
//! file — so `playback_source` persists beside them on `ProjectTrack`:
//! `build_project_file` captures it, JSON preserves it, and legacy
//! projects without the field load as `Live` (the pre-mode behaviour).

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::project::{ProjectFile, ProjectTrack};
use resonance_app::state::TrackState;
use resonance_app::update::project_io::build_project_file;
use resonance_app::Resonance;
use resonance_audio::types::TrackId;
use resonance_common::PlaybackSource;

const TRACK: TrackId = 1;

fn app_with_track() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app
}

#[test]
fn playback_source_round_trips_through_the_project_file() {
    let mut app = app_with_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));
    let _ = app.update(Message::ExternalInstrument(Eim::SetPlaybackSource(
        TRACK,
        PlaybackSource::Recorded,
    )));

    let file = build_project_file(&app);
    assert_eq!(file.tracks.len(), 1);
    assert_eq!(file.tracks[0].playback_source, PlaybackSource::Recorded);

    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let back: ProjectFile = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.tracks[0].playback_source, PlaybackSource::Recorded);
}

#[test]
fn default_mode_serializes_as_live() {
    let app = app_with_track();
    let file = build_project_file(&app);
    assert_eq!(file.tracks[0].playback_source, PlaybackSource::Live);
}

#[test]
fn legacy_track_without_field_loads_as_live() {
    // A project authored before the playback-source field has no
    // `playback_source` key; `#[serde(default)]` must fill it with
    // `Live` so legacy projects keep exactly the old behaviour.
    let legacy_track = r#"{
        "id": 1,
        "name": "Bass",
        "order": 0,
        "volume": 0.0,
        "pan": 0.0,
        "muted": false,
        "soloed": false,
        "record_armed": false,
        "monitor_enabled": true,
        "mono": false,
        "input_device_name": null,
        "plugins": []
    }"#;
    let pt: ProjectTrack = serde_json::from_str(legacy_track).expect("legacy track parses");
    assert_eq!(pt.playback_source, PlaybackSource::Live);
    assert!(pt.monitor_enabled, "neighbouring fields unaffected");
}
