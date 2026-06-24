//! Project-persistence coverage for parameter-automation lanes (todo #379,
//! arch doc #162 §3). Proves lanes survive a save → JSON → load round-trip
//! and that the load path rehydrates the app-side mirror, that legacy
//! project files (no `automation_lanes` field) load with no automation, and
//! that an undo/redo restores prior lane state via the snapshot-diff replay.

use resonance_app::message::{AutomationMessage, Message};
use resonance_app::state::TrackState;
use resonance_app::Resonance;
use resonance_common::{AutomationTarget, CurveKind};

/// One audio track plus a known volume so lane seeding has something to read.
fn app_with_track(id: u64, volume_db: f32) -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    let mut track = TrackState::new_audio(id, 0);
    track.volume = volume_db;
    app.test_push_track(track);
    app
}

fn send(app: &mut Resonance, m: AutomationMessage) {
    let _ = app.update(Message::Automation(m));
}

/// Build a couple of automation lanes on one track: a multi-point gain lane
/// (with a Stepped breakpoint) and a pan lane with Read turned off.
fn seed_lanes(app: &mut Resonance) {
    let gain = AutomationTarget::TrackGain(1);
    for (t, v, curve) in [
        (0u64, 0.2f32, CurveKind::Linear),
        (24_000, 0.8, CurveKind::Stepped),
        (48_000, 0.5, CurveKind::Linear),
    ] {
        send(
            app,
            AutomationMessage::AddBreakpoint {
                target: gain,
                time_frames: t,
                value: v,
                curve,
            },
        );
    }

    let pan = AutomationTarget::TrackPan(1);
    send(app, AutomationMessage::AddLane(pan));
    // Read off — the disabled flag must survive the round-trip.
    send(app, AutomationMessage::ToggleRead(pan));
}

#[test]
fn lanes_survive_save_load_roundtrip() {
    let mut app = app_with_track(1, -6.0);
    seed_lanes(&mut app);
    let before = app.test_automation().lanes.clone();
    assert_eq!(before.len(), 2, "two lanes seeded");

    // Serialize → JSON → deserialize, exactly as save/load does on disk.
    let file = app.test_build_project_file();
    assert_eq!(file.automation_lanes.len(), 2, "both lanes serialized");
    // On-disk order is stable (sorted by lane id).
    let ids: Vec<u64> = file.automation_lanes.iter().map(|l| l.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "lanes persisted in lane-id order");

    let json = serde_json::to_string(&file).expect("serialize project.json");
    let parsed: resonance_app::project::ProjectFile =
        serde_json::from_str(&json).expect("deserialize project.json");
    assert_eq!(parsed.automation_lanes.len(), 2);

    // Load into a fresh app and confirm the mirror is rebuilt exactly.
    let mut loaded = app_with_track(1, -6.0);
    loaded.test_replay_loaded_project(parsed);
    let after = loaded.test_automation().lanes.clone();
    assert_eq!(after, before, "lanes round-trip identically across save/load");

    // The disabled pan lane kept its Read-off flag.
    let pan = AutomationTarget::TrackPan(1);
    assert!(
        !after[&pan].enabled,
        "Read-off flag persists across save/load"
    );

    // A gain breakpoint kept its Stepped curve and value.
    let gain = AutomationTarget::TrackGain(1);
    let stepped = after[&gain]
        .points
        .iter()
        .find(|p| p.time_frames == 24_000)
        .expect("mid breakpoint present");
    assert_eq!(stepped.curve, CurveKind::Stepped);
    assert!((stepped.value - 0.8).abs() < 1e-6);
}

#[test]
fn loading_lanes_bumps_id_allocator_past_persisted_ids() {
    let mut app = app_with_track(1, 0.0);
    seed_lanes(&mut app);
    let file = app.test_build_project_file();
    let max_id = file
        .automation_lanes
        .iter()
        .map(|l| l.id)
        .max()
        .expect("lanes present");

    let mut loaded = app_with_track(1, 0.0);
    loaded.test_replay_loaded_project(file);

    // A lane created after the load must get a fresh id, never colliding
    // with a persisted one.
    let master = AutomationTarget::MasterGain;
    send(&mut loaded, AutomationMessage::AddLane(master));
    let new_id = loaded.test_automation().lanes[&master].id;
    assert!(
        new_id > max_id,
        "new lane id {new_id} must exceed max persisted id {max_id}"
    );
}

#[test]
fn legacy_project_without_automation_field_loads_empty() {
    // A v2 project authored before automation lanes existed simply omits
    // the field; serde's #[serde(default)] must fill an empty vec.
    let json = r#"{
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
        "tracks": [],
        "clips": []
    }"#;
    let parsed: resonance_app::project::ProjectFile =
        serde_json::from_str(json).expect("legacy project parses");
    assert!(
        parsed.automation_lanes.is_empty(),
        "legacy projects load with no automation"
    );

    let mut app = app_with_track(1, 0.0);
    seed_lanes(&mut app);
    assert!(!app.test_automation().lanes.is_empty());
    // Loading a legacy (lane-free) project on top of one with lanes must
    // clear the stale lanes.
    app.test_replay_loaded_project(parsed);
    assert!(
        app.test_automation().lanes.is_empty(),
        "loading a lane-free project clears stale lanes"
    );
}

#[test]
fn undo_restores_prior_lane_state() {
    let mut app = app_with_track(1, 0.0);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-test"));
    let target = AutomationTarget::TrackGain(1);
    send(&mut app, AutomationMessage::AddLane(target));
    let before = app.test_automation().lanes[&target].clone();

    // Mutate the lane: add a breakpoint.
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: 48_000,
            value: 1.0,
            curve: CurveKind::Linear,
        },
    );
    assert_eq!(app.test_automation().lanes[&target].points.len(), 2);

    // Undo restores the prior lane state (snapshot-diff replay).
    let _ = app.update(Message::Undo);
    assert_eq!(
        app.test_automation().lanes[&target],
        before,
        "undo restores the lane to its pre-edit breakpoints"
    );

    // Redo re-applies the edit.
    let _ = app.update(Message::Redo);
    assert_eq!(app.test_automation().lanes[&target].points.len(), 2);
}
