//! `external.*` control methods through the real update path: creating
//! an external track, wiring both halves of its hardware route, patch /
//! latency / monitoring / playback-source edits, and the read-only
//! device + status views.
//!
//! The hardware itself is never touched — device lists are seeded with
//! the engine events the real enumeration would deliver, so the tests
//! run headless.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, InputDeviceInfo};
use resonance_audio::MidiDeviceInfo;
use resonance_control::methods::external::{DevicesView, PlaybackSource, StatusView};
use resonance_control::methods::song::TracksView;
use resonance_control::methods::track::AddResult;
use resonance_control::{ErrorKind, Request, Response, TrackKind};
use crate::common::{call, roundtrip};

const MIDI_OUT: &str = "Moog Muse MIDI 1";
const RETURN_IN: &str = "alsa_input.usb-Focusrite_Scarlett";

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-external-test.rprj"));
    seed_devices(&mut app);
    app
}

/// Publish the hardware lists the engine would enumerate at startup.
fn seed_devices(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::MidiOutputDevicesListed {
        devices: vec![MidiDeviceInfo {
            name: MIDI_OUT.to_owned(),
        }],
    });
    app.test_apply_engine_event(AudioEvent::InputDevicesListed {
        devices: vec![InputDeviceInfo {
            name: RETURN_IN.to_owned(),
            description: "Scarlett 18i20 Analog".to_owned(),
            channels: 4,
        }],
        default_name: Some(RETURN_IN.to_owned()),
    });
}

fn error_kind(response: &Response) -> ErrorKind {
    response.error.as_ref().expect("an error reply").kind()
}

/// Add an external track over the control endpoint, pumping the engine
/// echo that mirrors it into the registry.
fn add_external_track(app: &mut Resonance) -> u64 {
    let result: AddResult = call(app, "track.add", serde_json::json!({ "kind": "external" }))
        .result()
        .expect("track.add external succeeds");
    let id = u64::from(result.track_id);
    app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id: id });
    id
}

fn status(app: &mut Resonance) -> StatusView {
    roundtrip(app, Request::without_params(99, "external.status"))
        .result()
        .expect("external.status succeeds")
}

#[test]
fn devices_lists_what_the_setters_accept() {
    let mut app = app();
    let view: DevicesView = roundtrip(&mut app, Request::without_params(1, "external.devices"))
        .result()
        .expect("external.devices succeeds");

    assert_eq!(view.midi_outputs, vec![MIDI_OUT.to_owned()]);
    assert_eq!(view.audio_inputs.len(), 1);
    assert_eq!(view.audio_inputs[0].name, RETURN_IN);
    assert_eq!(view.audio_inputs[0].channels, 4);
    assert!(view.audio_inputs[0].default, "the default capture device");
}

#[test]
fn add_external_track_reports_its_kind_and_status() {
    let mut app = app();
    let id = add_external_track(&mut app);

    // The track is external the moment `track.add` returns — one call,
    // not add-then-enable.
    let tracks: TracksView = roundtrip(&mut app, Request::without_params(2, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    let track = tracks
        .tracks
        .iter()
        .find(|t| u64::from(t.summary.id) == id)
        .expect("the new track is listed");
    assert_eq!(track.summary.kind, TrackKind::External);

    // Nothing is wired yet, and the status view says exactly that.
    let view = status(&mut app);
    assert_eq!(view.tracks.len(), 1);
    let ext = &view.tracks[0];
    assert_eq!(ext.status, "unconfigured");
    assert_eq!(ext.midi_out_device, None);
    assert_eq!(ext.return_device, None);
    assert_eq!(ext.playback_source, PlaybackSource::Live);
    assert_eq!(ext.take_count, 0);
}

#[test]
fn wiring_both_halves_reaches_live() {
    let mut app = app();
    let id = add_external_track(&mut app);

    call(
        &mut app,
        "external.set_midi_out",
        serde_json::json!({ "track_id": id, "device": MIDI_OUT, "channel": 3 }),
    )
    .result::<serde_json::Value>()
    .expect("set_midi_out succeeds");

    // MIDI out alone is not enough to hear anything.
    assert_eq!(status(&mut app).tracks[0].status, "configuring");

    call(
        &mut app,
        "external.set_return",
        serde_json::json!({ "track_id": id, "device": RETURN_IN, "port": 2 }),
    )
    .result::<serde_json::Value>()
    .expect("set_return succeeds");
    call(
        &mut app,
        "external.set_monitor",
        serde_json::json!({ "track_id": id, "enabled": true }),
    )
    .result::<serde_json::Value>()
    .expect("set_monitor succeeds");

    let ext = status(&mut app).tracks.remove(0);
    assert_eq!(ext.status, "live");
    assert_eq!(ext.midi_out_device.as_deref(), Some(MIDI_OUT));
    // 1-based on the wire, matching the UI, whatever the app stores.
    assert_eq!(ext.midi_out_channel, 3);
    assert_eq!(ext.return_device.as_deref(), Some(RETURN_IN));
    assert_eq!(ext.return_port, 2);
    assert!(ext.monitor_enabled);
}

#[test]
fn unknown_device_names_are_rejected_not_stored() {
    let mut app = app();
    let id = add_external_track(&mut app);

    // A stored-but-wrong port name silently swallows every note, so it
    // has to bounce rather than be accepted.
    let response = call(
        &mut app,
        "external.set_midi_out",
        serde_json::json!({ "track_id": id, "device": "Nonexistent Synth" }),
    );
    assert_eq!(error_kind(&response), ErrorKind::NotFound);

    let response = call(
        &mut app,
        "external.set_return",
        serde_json::json!({ "track_id": id, "device": "Nonexistent Interface" }),
    );
    assert_eq!(error_kind(&response), ErrorKind::NotFound);

    assert_eq!(status(&mut app).tracks[0].status, "unconfigured");
}

#[test]
fn out_of_range_settings_are_rejected() {
    let mut app = app();
    let id = add_external_track(&mut app);

    let channel = call(
        &mut app,
        "external.set_midi_out",
        serde_json::json!({ "track_id": id, "device": MIDI_OUT, "channel": 17 }),
    );
    assert_eq!(error_kind(&channel), ErrorKind::InvalidParams);

    // The return device has 4 channels, so port 4 is off the end.
    let port = call(
        &mut app,
        "external.set_return",
        serde_json::json!({ "track_id": id, "device": RETURN_IN, "port": 4 }),
    );
    assert_eq!(error_kind(&port), ErrorKind::InvalidParams);

    let bank = call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": id, "bank": 20_000 }),
    );
    assert_eq!(error_kind(&bank), ErrorKind::InvalidParams);
}

#[test]
fn methods_refuse_a_track_that_is_not_external() {
    let mut app = app();
    let result: AddResult = call(&mut app, "track.add", serde_json::json!({ "kind": "instrument" }))
        .result()
        .expect("track.add succeeds");
    let id = u64::from(result.track_id);
    app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id: id });

    // Fixable with external.enable — which is why this is invalid_params
    // and not not_found.
    let response = call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": id, "program": 4 }),
    );
    assert_eq!(error_kind(&response), ErrorKind::InvalidParams);

    let missing = call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": 9_999, "program": 4 }),
    );
    assert_eq!(error_kind(&missing), ErrorKind::NotFound);

    // Enabling it makes the same call work, and the track now reports as
    // external.
    call(&mut app, "external.enable", serde_json::json!({ "track_id": id }))
        .result::<serde_json::Value>()
        .expect("enable succeeds");
    call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": id, "program": 4 }),
    )
    .result::<serde_json::Value>()
    .expect("set_patch succeeds once external");
    assert_eq!(status(&mut app).tracks[0].program, Some(4));
}

#[test]
fn patch_bank_and_program_land_together() {
    let mut app = app();
    let id = add_external_track(&mut app);

    call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": id, "bank": 1_024, "program": 7 }),
    )
    .result::<serde_json::Value>()
    .expect("set_patch succeeds");

    let ext = status(&mut app).tracks.remove(0);
    assert_eq!(ext.bank, Some(1_024));
    assert_eq!(ext.program, Some(7));

    // Editing one half leaves the other alone.
    call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": id, "program": 9 }),
    )
    .result::<serde_json::Value>()
    .expect("set_patch succeeds");
    let ext = status(&mut app).tracks.remove(0);
    assert_eq!(ext.bank, Some(1_024), "bank untouched");
    assert_eq!(ext.program, Some(9));

    // Explicit null clears.
    call(
        &mut app,
        "external.set_patch",
        serde_json::json!({ "track_id": id, "bank": null }),
    )
    .result::<serde_json::Value>()
    .expect("set_patch succeeds");
    assert_eq!(status(&mut app).tracks[0].bank, None);
}

#[test]
fn playback_source_and_latency_round_trip() {
    let mut app = app();
    let id = add_external_track(&mut app);

    call(
        &mut app,
        "external.set_playback_source",
        serde_json::json!({ "track_id": id, "source": "recorded" }),
    )
    .result::<serde_json::Value>()
    .expect("set_playback_source succeeds");
    call(
        &mut app,
        "external.set_latency",
        serde_json::json!({ "track_id": id, "offset_samples": -512 }),
    )
    .result::<serde_json::Value>()
    .expect("set_latency succeeds");

    let ext = status(&mut app).tracks.remove(0);
    assert_eq!(ext.playback_source, PlaybackSource::Recorded);
    assert_eq!(ext.latency_offset_samples, -512);
}

#[test]
fn declarative_flags_do_not_flip_when_already_set() {
    let mut app = app();
    let id = add_external_track(&mut app);

    for _ in 0..3 {
        call(
            &mut app,
            "external.set_monitor",
            serde_json::json!({ "track_id": id, "enabled": true }),
        )
        .result::<serde_json::Value>()
        .expect("set_monitor succeeds");
    }
    assert!(
        status(&mut app).tracks[0].monitor_enabled,
        "repeat calls with the same value must not toggle"
    );

    call(
        &mut app,
        "external.set_record_arm",
        serde_json::json!({ "track_id": id, "armed": true }),
    )
    .result::<serde_json::Value>()
    .expect("set_record_arm succeeds");
    call(
        &mut app,
        "external.set_record_arm",
        serde_json::json!({ "track_id": id, "armed": true }),
    )
    .result::<serde_json::Value>()
    .expect("set_record_arm succeeds");
    assert!(status(&mut app).tracks[0].record_armed);
}

#[test]
fn disable_takes_the_track_out_of_external_mode() {
    let mut app = app();
    let id = add_external_track(&mut app);
    assert_eq!(status(&mut app).tracks.len(), 1);

    call(&mut app, "external.disable", serde_json::json!({ "track_id": id }))
        .result::<serde_json::Value>()
        .expect("disable succeeds");

    assert!(status(&mut app).tracks.is_empty(), "no longer external");
    let tracks: TracksView = roundtrip(&mut app, Request::without_params(2, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    let track = tracks
        .tracks
        .iter()
        .find(|t| u64::from(t.summary.id) == id)
        .expect("the track itself survives");
    assert_eq!(track.summary.kind, TrackKind::Instrument);
}

#[test]
fn detect_latency_needs_both_halves_wired() {
    let mut app = app();
    let id = add_external_track(&mut app);

    // Nothing to ping and nothing to listen on.
    let response = call(
        &mut app,
        "external.detect_latency",
        serde_json::json!({ "track_id": id }),
    );
    assert_eq!(error_kind(&response), ErrorKind::InvalidParams);

    call(
        &mut app,
        "external.set_midi_out",
        serde_json::json!({ "track_id": id, "device": MIDI_OUT }),
    )
    .result::<serde_json::Value>()
    .expect("set_midi_out succeeds");
    call(
        &mut app,
        "external.set_return",
        serde_json::json!({ "track_id": id, "device": RETURN_IN }),
    )
    .result::<serde_json::Value>()
    .expect("set_return succeeds");

    call(
        &mut app,
        "external.detect_latency",
        serde_json::json!({ "track_id": id }),
    )
    .result::<serde_json::Value>()
    .expect("detect_latency starts once both halves are wired");
    assert!(
        status(&mut app).tracks[0].latency_detect_in_progress,
        "the measurement is reported as running"
    );
}

#[test]
fn bounce_refuses_a_track_with_no_midi_to_play() {
    let mut app = app();
    let id = add_external_track(&mut app);
    call(
        &mut app,
        "external.set_return",
        serde_json::json!({ "track_id": id, "device": RETURN_IN }),
    )
    .result::<serde_json::Value>()
    .expect("set_return succeeds");

    // A realtime bounce plays the track's MIDI to the synth and records
    // what comes back; with no MIDI it would capture silence.
    let response = call(&mut app, "external.bounce", serde_json::json!({ "track_id": id }));
    assert_eq!(error_kind(&response), ErrorKind::InvalidParams);
}

/// `external.set_midi_out` with both halves given dispatches two
/// messages; the wire contract makes them ONE undoable transaction —
/// one revision bump per call. `set_return` shares the exact wrapper.
#[test]
fn set_midi_out_of_device_and_channel_is_one_revision_bump() {
    let mut app = app();
    let id = add_external_track(&mut app);
    let before = app.revision();
    let entries = app.test_undo_history().test_undo_entries().len();

    call(
        &mut app,
        "external.set_midi_out",
        serde_json::json!({ "track_id": id, "device": MIDI_OUT, "channel": 3 }),
    )
    .result::<serde_json::Value>()
    .expect("set_midi_out succeeds");

    assert_eq!(app.revision(), before + 1, "device + channel are one call");
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        entries + 1,
        "and one history entry"
    );
}

/// `external.bounce` drives the dialog with FIVE dispatches (open, pick
/// device, mono, port, confirm); the wire contract makes the whole flow
/// ONE undoable transaction — one revision bump, one history entry.
#[test]
fn bounce_is_one_revision_bump_and_one_undo_entry() {
    let mut app = app();
    let id = add_external_track(&mut app);
    // Both halves wired: the bounce classifier requires a MIDI Out and
    // the handler a return to capture from.
    call(
        &mut app,
        "external.set_midi_out",
        serde_json::json!({ "track_id": id, "device": MIDI_OUT }),
    )
    .result::<serde_json::Value>()
    .expect("set_midi_out succeeds");
    call(
        &mut app,
        "external.set_return",
        serde_json::json!({ "track_id": id, "device": RETURN_IN }),
    )
    .result::<serde_json::Value>()
    .expect("set_return succeeds");
    // Something for the bounce to re-drive the synth with.
    app.test_push_midi_clip(resonance_app::state::MidiClipState {
        id: 500,
        track_id: id,
        start_sample: 0,
        duration_ticks: 4 * 480,
        name: "riff".to_owned(),
        notes: vec![resonance_audio::types::MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }]
        .into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let before = app.revision();
    let entries = app.test_undo_history().test_undo_entries().len();

    call(&mut app, "external.bounce", serde_json::json!({ "track_id": id }))
        .result::<serde_json::Value>()
        .expect("external.bounce starts");

    assert_eq!(app.revision(), before + 1, "five dispatches, one revision bump");
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        entries + 1,
        "the whole dialog flow is one history entry"
    );
}
