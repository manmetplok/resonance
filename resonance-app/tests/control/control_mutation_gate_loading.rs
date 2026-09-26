//! The mutation gate refuses edits while a replay is pending (code review
//! UPD-03).
//!
//! A project load, a template instantiation and an undo/redo's full
//! restore all send `ClearAll` and rebuild everything from a snapshot once
//! the engine answers `AllCleared` — on a later Tick. A control edit
//! landing in between was acknowledged (revision bumped) and then silently
//! wiped by the replay. It must answer `busy` instead, like the other
//! gates. (Since ARCH-01 A-13i an undo never takes the full restore on its
//! own — it is forced here — and a diff undo is synchronous, so it leaves
//! no window.)

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType};
use resonance_control::{ErrorKind, MutationAck};
use crate::common::call;

const TRACK: u64 = 1;

fn set_volume(app: &mut Resonance) -> resonance_control::Response {
    call(
        app,
        "mixer.set_volume_db",
        serde_json::json!({"track_id": TRACK, "volume_db": -6.0}),
    )
}

#[test]
fn a_control_edit_during_a_slow_path_undo_is_busy() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-gate-loading.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    let before_take = app.test_snapshot_for_undo();
    // A finished take is an undoable edit.
    app.test_apply_engine_event(AudioEvent::RecordingFinished {
        clip_id: 7,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 48_000,
        name: "take".into(),
        waveform_peaks: Vec::new(),
    });
    // Its undo through the full path: ClearAll now, the replay on the
    // engine's AllCleared.
    app.test_begin_full_restore_from_snapshot(before_take);
    let revision = app.revision();

    let error = set_volume(&mut app)
        .error
        .expect("an edit mid-replay must not be acknowledged");
    assert_eq!(error.kind(), ErrorKind::Busy, "got {error:?}");
    assert_eq!(app.revision(), revision, "a refused edit bumps nothing");

    // Once the replay has run the project takes edits again.
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let _: MutationAck = set_volume(&mut app).result().expect("edits resume after the replay");
}
