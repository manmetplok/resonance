//! Project-file persistence coverage for external-instrument tracks
//! (architecture doc #169, epic #39, ba todo #456).
//!
//! Verifies the on-disk round-trip of the external-instrument config:
//! `build_project_file` captures the bank/program/latency extras plus the
//! route (MIDI out + audio return) and monitor flag onto the saved
//! `ProjectTrack`; the JSON serialization preserves every field; and old
//! projects that predate the field load cleanly as non-external.
//!
//! The replay side (re-applying the route + patch to the engine on load)
//! drives the real (idle) engine queue and is exercised by the
//! engine-handler tests; here we assert the serialized shape, which is the
//! contract the loader reads back.

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::project::{ProjectExternalInstrument, ProjectFile, ProjectTrack};
use resonance_app::state::TrackState;
use resonance_app::update::project_io::build_project_file;
use resonance_app::Resonance;
use resonance_audio::types::TrackId;

const TRACK: TrackId = 1;

/// Fresh app with an active project and a single instrument track.
fn app_with_track() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

/// Fully wire `TRACK` as an external instrument with a known config.
fn configure_external(app: &mut Resonance) {
    dispatch(app, Eim::Enable(TRACK));
    dispatch(app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(app, Eim::SetMidiOutChannel(TRACK, Some(2)));
    dispatch(app, Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())));
    dispatch(app, Eim::SetReturnPort(TRACK, 3));
    dispatch(app, Eim::SetBank(TRACK, Some(0x0102)));
    dispatch(app, Eim::SetProgram(TRACK, Some(12)));
    dispatch(app, Eim::SetLatencyOffset(TRACK, 256));
    dispatch(app, Eim::ToggleMonitor(TRACK));
}

/// Pull the single track out of a serialized project file.
fn only_track(file: &ProjectFile) -> &ProjectTrack {
    assert_eq!(file.tracks.len(), 1, "expected exactly one track");
    &file.tracks[0]
}

#[test]
fn build_project_file_captures_external_config_and_route() {
    let mut app = app_with_track();
    configure_external(&mut app);

    let file = build_project_file(&app);
    let pt = only_track(&file);

    // External-specific extras land in the dedicated struct.
    let ext = pt
        .external_instrument
        .as_ref()
        .expect("external track must serialize an external_instrument");
    assert_eq!(ext.bank, Some(0x0102));
    assert_eq!(ext.program, Some(12));
    assert_eq!(ext.latency_offset_samples, 256);

    // The route + monitor flag persist via the plain track fields (single
    // source of truth — not duplicated into the external struct).
    assert_eq!(pt.midi_output_device.as_deref(), Some("Moog Muse"));
    assert_eq!(pt.midi_output_channel, Some(2));
    assert_eq!(pt.input_device_name.as_deref(), Some("Scarlett 18i20"));
    assert_eq!(pt.input_port_index, Some(3));
    assert!(pt.monitor_enabled, "monitor flag should persist");
}

#[test]
fn plain_track_serializes_without_external_instrument() {
    let app = app_with_track();
    let file = build_project_file(&app);
    let pt = only_track(&file);
    assert!(
        pt.external_instrument.is_none(),
        "a non-external track must not serialize an external_instrument"
    );
}

#[test]
fn disable_drops_external_instrument_from_saved_file() {
    let mut app = app_with_track();
    configure_external(&mut app);
    dispatch(&mut app, Eim::Disable(TRACK));

    let file = build_project_file(&app);
    let pt = only_track(&file);
    assert!(
        pt.external_instrument.is_none(),
        "Disable removes the track from the external map, so it must not persist"
    );
}

#[test]
fn json_round_trip_preserves_every_field() {
    let mut app = app_with_track();
    configure_external(&mut app);

    let file = build_project_file(&app);
    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let back: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    let pt = only_track(&back);
    let ext = pt
        .external_instrument
        .as_ref()
        .expect("external_instrument survives JSON round-trip");
    assert_eq!(ext.bank, Some(0x0102));
    assert_eq!(ext.program, Some(12));
    assert_eq!(ext.latency_offset_samples, 256);
    assert_eq!(pt.midi_output_device.as_deref(), Some("Moog Muse"));
    assert_eq!(pt.midi_output_channel, Some(2));
    assert_eq!(pt.input_device_name.as_deref(), Some("Scarlett 18i20"));
    assert_eq!(pt.input_port_index, Some(3));
    assert!(pt.monitor_enabled);
}

#[test]
fn external_instrument_struct_round_trips_with_none_fields() {
    // bank/program absent is a valid config (leave the device on its
    // current patch); it must round-trip as None, not as a default.
    let ext = ProjectExternalInstrument {
        bank: None,
        program: None,
        latency_offset_samples: -128,
    };
    let json = serde_json::to_string(&ext).unwrap();
    let back: ProjectExternalInstrument = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ext);
}

#[test]
fn legacy_track_without_field_loads_as_non_external() {
    // A project authored before external-instrument persistence has no
    // `external_instrument` key on its tracks. `#[serde(default)]` must
    // fill it with None so the track loads as a plain (non-external) track.
    let legacy_track = r#"{
        "id": 1,
        "name": "Bass",
        "order": 0,
        "volume": 0.0,
        "pan": 0.0,
        "muted": false,
        "soloed": false,
        "record_armed": false,
        "monitor_enabled": false,
        "mono": false,
        "input_device_name": null,
        "plugins": []
    }"#;
    let pt: ProjectTrack = serde_json::from_str(legacy_track).expect("legacy track parses");
    assert!(
        pt.external_instrument.is_none(),
        "legacy track must default to non-external"
    );
}

#[test]
fn legacy_project_without_external_tracks_loads_cleanly() {
    // A minimal legacy project.json with one track and none of the newer
    // optional fields must parse and yield a non-external track.
    let legacy_project = r#"{
        "version": 2,
        "sample_rate": 44100,
        "bpm": 120.0,
        "time_sig_num": 4,
        "time_sig_den": 4,
        "metronome_enabled": false,
        "master_volume": 0.0,
        "loop_enabled": false,
        "loop_in": 0,
        "loop_out": 0,
        "tracks": [{
            "id": 1,
            "name": "Track 1",
            "order": 0,
            "volume": 0.0,
            "pan": 0.0,
            "muted": false,
            "soloed": false,
            "record_armed": false,
            "monitor_enabled": false,
            "mono": false,
            "input_device_name": null,
            "plugins": []
        }],
        "clips": []
    }"#;
    let file: ProjectFile = serde_json::from_str(legacy_project).expect("legacy project parses");
    assert_eq!(file.tracks.len(), 1);
    assert!(file.tracks[0].external_instrument.is_none());
}
